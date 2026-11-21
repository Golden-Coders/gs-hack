//! Hot reload mappingów (tasks 3.4): wymaga Postgresa i Kafki.

mod common;

use std::time::Duration;

use ::common::db::sqlx;
use lifecycle::config::new_store;
use lifecycle::run_config;
use uuid::Uuid;

fn yaml(schema: &str, title: &str) -> String {
    format!("schema: {schema}\nfields: {{}}\ntitle: {title}\nseverity:\n  type: const\n  value: warning\nfingerprint:\n- $.id\n")
}

#[tokio::test]
async fn mapping_changes_are_hot_reloaded() {
    let (pool, producer) = common::setup().await;
    let schema = format!("t-{}", Uuid::new_v4());
    let mut cl = common::Cleanup::new(1);
    cl.schema(&schema);
    let dir = std::env::temp_dir().join(format!("lifecycle-cfg-{}", Uuid::new_v4()));
    let mdir = dir.join("mappings");
    std::fs::create_dir_all(&mdir).unwrap();
    let file = mdir.join(format!("{schema}.yaml"));
    std::fs::write(&file, yaml(&schema, "$.a")).unwrap();

    let store = new_store();
    let (tx, rx) = tokio::sync::watch::channel(false);
    let h = tokio::spawn(run_config(common::database_url(), dir.to_string_lossy().into(), pool.clone(), producer, store.clone(), rx));

    let title = |s: &lifecycle::MappingStore, schema: &str| s.load().get(schema).map(|m| (m.version, m.def.title.clone().unwrap_or_default()));
    common::wait_for("seed + load", Duration::from_secs(20), || async { (title(&store, &schema) == Some((1, "$.a".into()))).then_some(()) }).await;

    // zmiana w bazie + NOTIFY (jak z API) -> nowa wersja bez restartu
    sqlx::query("UPDATE mappings SET def = jsonb_set(def, '{title}', '\"$.b\"'), version = version + 1 WHERE schema = $1")
        .bind(&schema)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("SELECT pg_notify('config_changed', 'mappings')").execute(&pool).await.unwrap();
    common::wait_for("reload po NOTIFY", Duration::from_secs(20), || async { (title(&store, &schema) == Some((2, "$.b".into()))).then_some(()) }).await;

    // zmiana pliku -> upsert z wyższą wersją
    tokio::time::sleep(Duration::from_millis(1200)).await; // pewna zmiana mtime
    std::fs::write(&file, yaml(&schema, "$.c")).unwrap();
    common::wait_for("reload po zmianie pliku", Duration::from_secs(20), || async { (title(&store, &schema) == Some((3, "$.c".into()))).then_some(()) }).await;

    tx.send(true).unwrap();
    h.await.unwrap().unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}
