//! People and teams routes (`/people`, `/teams`).

use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    extract::{rejection::JsonRejection, Path, State},
    http::StatusCode,
    routing::{get, put},
    Json, Router,
};
use common::db::sqlx::{self, types::Json as SqlJson, PgPool};
use contracts::{PersonDef, TeamDef};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::error::{db_err, err, opt_json, ApiResult};
use crate::routes::resolve_config_path;
use crate::AppState;

static PEOPLE_SEEDED: AtomicBool = AtomicBool::new(false);

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/people", get(list_people))
        .route("/teams", get(list_teams).post(create_team))
        .route("/teams/{id}", put(put_team).delete(delete_team))
}

#[derive(Deserialize)]
pub(crate) struct PeopleConfigFile {
    #[serde(default)]
    pub people: Vec<PersonDef>,
    #[serde(default)]
    pub teams: Vec<TeamDef>,
}

pub(crate) async fn ensure_people_seeded(db: &PgPool, config_dir: &str) {
    if PEOPLE_SEEDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let Some(path) = resolve_config_path(config_dir, "people.yaml") else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(parsed) = serde_yaml::from_str::<PeopleConfigFile>(&text) else {
        return;
    };
    for person in &parsed.people {
        let Ok(def_val) = serde_json::to_value(person) else {
            continue;
        };
        let contacts_val = serde_json::to_value(&person.contacts).unwrap_or_else(|_| json!({}));
        let fallback_val = serde_json::to_value(&person.fallback).unwrap_or_else(|_| json!([]));
        let _ = sqlx::query(
            "INSERT INTO people (id, name, timezone, contacts, fallback, def, updated_at) \
             VALUES ($1, $2, $3, $4, $5, $6, now()) ON CONFLICT (id) DO UPDATE SET \
             name = EXCLUDED.name, def = EXCLUDED.def, updated_at = now()",
        )
        .bind(&person.id)
        .bind(&person.name)
        .bind(&person.timezone)
        .bind(SqlJson(contacts_val))
        .bind(SqlJson(fallback_val))
        .bind(SqlJson(def_val))
        .execute(db)
        .await;
    }
    let teams_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teams")
        .fetch_one(db)
        .await
        .unwrap_or(1);
    if teams_count == 0 {
        for team in &parsed.teams {
            let Ok(def_val) = serde_json::to_value(team) else {
                continue;
            };
            let _ = sqlx::query(
                "INSERT INTO teams (id, name, def, updated_at) \
                 VALUES ($1, $2, $3, now()) ON CONFLICT (id) DO NOTHING",
            )
            .bind(&team.id)
            .bind(&team.name)
            .bind(SqlJson(def_val))
            .execute(db)
            .await;

            for member in &team.members {
                let _ = sqlx::query(
                    "INSERT INTO team_members (team_id, person_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
                )
                .bind(&team.id)
                .bind(member)
                .execute(db)
                .await;
            }
        }
    }
}

async fn list_people(State(st): State<AppState>) -> ApiResult<Json<Value>> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let rows = sqlx::query("SELECT def FROM people ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let mut people = Vec::with_capacity(rows.len());
    for r in &rows {
        let p: PersonDef = serde_json::from_value(opt_json(r, "def"))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored person", e))?;
        people.push(p);
    }
    Ok(Json(json!({
        "source_file": "config/people.yaml",
        "people": people,
    })))
}

async fn list_teams(State(st): State<AppState>) -> ApiResult<Json<Vec<TeamDef>>> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let rows = sqlx::query("SELECT def FROM teams ORDER BY id")
        .fetch_all(&st.db)
        .await
        .map_err(db_err)?;
    let mut teams = Vec::with_capacity(rows.len());
    for r in &rows {
        let t: TeamDef = serde_json::from_value(opt_json(r, "def"))
            .map_err(|e| err(StatusCode::INTERNAL_SERVER_ERROR, "invalid stored team", e))?;
        teams.push(t);
    }
    Ok(Json(teams))
}

async fn upsert_team(db: &PgPool, old_id: Option<&str>, team: TeamDef) -> ApiResult<TeamDef> {
    if team.id.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "invalid team", "id zespołu nie może być puste"));
    }
    if team.name.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "invalid team", "nazwa zespołu nie może być pusta"));
    }
    let def_val = serde_json::to_value(&team)
        .map_err(|e| err(StatusCode::BAD_REQUEST, "invalid team", e))?;

    let mut tx = db.begin().await.map_err(db_err)?;
    if let Some(prev) = old_id {
        if prev != team.id {
            sqlx::query("DELETE FROM team_members WHERE team_id = $1")
                .bind(prev)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            sqlx::query("DELETE FROM teams WHERE id = $1")
                .bind(prev)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
    }

    sqlx::query(
        "INSERT INTO teams (id, name, def, updated_at) VALUES ($1, $2, $3, now()) \
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, def = EXCLUDED.def, updated_at = now()",
    )
    .bind(&team.id)
    .bind(&team.name)
    .bind(SqlJson(def_val))
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;

    sqlx::query("DELETE FROM team_members WHERE team_id = $1")
        .bind(&team.id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;

    for member in &team.members {
        let _ = sqlx::query(
            "INSERT INTO team_members (team_id, person_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
        )
        .bind(&team.id)
        .bind(member)
        .execute(&mut *tx)
        .await;
    }

    sqlx::query("SELECT pg_notify('config_changed', 'people')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(team)
}

async fn create_team(
    State(st): State<AppState>,
    body: Result<Json<TeamDef>, JsonRejection>,
) -> ApiResult<(StatusCode, Json<TeamDef>)> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let Json(team) = body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid team", e.body_text()))?;
    let saved = upsert_team(&st.db, None, team).await?;
    Ok((StatusCode::CREATED, Json(saved)))
}

async fn put_team(
    State(st): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<TeamDef>, JsonRejection>,
) -> ApiResult<Json<TeamDef>> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let Json(team) = body.map_err(|e| err(StatusCode::BAD_REQUEST, "invalid team", e.body_text()))?;
    let saved = upsert_team(&st.db, Some(&id), team).await?;
    Ok(Json(saved))
}

async fn delete_team(
    State(st): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<StatusCode> {
    ensure_people_seeded(&st.db, &st.config.config_dir).await;
    let mut tx = st.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM team_members WHERE team_id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM teams WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("SELECT pg_notify('config_changed', 'people')")
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
