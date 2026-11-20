//! Routy strumienia P2: reguły (CRUD, test, podpowiedzi pól), osoby i zespoły,
//! dostarczenia, symulowane kanały. Właściciel: P2. Zob. `contracts/openapi.yaml` (tag `p2`).

use axum::Router;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
}
