//! SSE `/stream` (Faza 0: szkielet – zdarzenie `hello` i heartbeat).
//!
//! Docelowo (design D13): konsumuje `alerts.changed` i `ui.events` (grupa
//! `api-sse-<instancja>`) i rozsyła je do klientów, filtrując po zespole.
//! Format zdarzeń SSE: `event: alert_changed` z `AlertChanged` albo
//! `event: ui_event` z `UiEvent` (zob. `contracts/openapi.yaml`).

use std::{convert::Infallible, time::Duration};

use axum::response::sse::{Event, KeepAlive, Sse};
use futures::{stream, Stream, StreamExt};

pub async fn sse() -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let hello = stream::once(async { Ok(Event::default().event("hello").data("{}")) });
    Sse::new(hello.chain(stream::pending())).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}
