//! People and teams routes (`/people`, `/teams`).

use std::sync::atomic::{AtomicBool, Ordering};

use axum::{extract::State, http::StatusCode, routing::get, Json, Router};
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
        .route("/teams", get(list_teams))
}

#[derive(Deserialize)]
pub(crate) struct PeopleConfigFile {
    #[serde(default)]
    pub people: Vec<PersonDef>,
    #[serde(default)]
    pub teams: Vec<TeamDef>,
}

async fn ensure_people_seeded(db: &PgPool, config_dir: &str) {
    if PEOPLE_SEEDED.swap(true, Ordering::SeqCst) {
        return;
    }
    let people_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM people")
        .fetch_one(db)
        .await
        .unwrap_or(1);
    let teams_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM teams")
        .fetch_one(db)
        .await
        .unwrap_or(1);
    if people_count > 0 && teams_count > 0 {
        return;
    }
    let path = resolve_config_path(config_dir, "people.yaml");
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
             VALUES ($1, $2, $3, $4, $5, $6, now()) ON CONFLICT (id) DO NOTHING",
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
