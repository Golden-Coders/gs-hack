//! Routy strumienia P1: alerty (lista, szczegóły, ack/resolve), mappingi,
//! UNMAPPED, błędy ingestu. Właściciel: P1. Zob. `contracts/openapi.yaml` (tag `p1`).

use axum::Router;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
}
