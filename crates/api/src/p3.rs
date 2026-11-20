//! Routy strumienia P3: runbooki, akcje (CRUD, brakujące), wykonania kroków,
//! kolejka zatwierdzeń, dziennik. Właściciel: P3. Zob. `contracts/openapi.yaml` (tag `p3`).

use axum::Router;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
}
