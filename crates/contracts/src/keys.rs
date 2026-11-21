//! Klucze idempotencji (design D9). At-least-once z Kafki oznacza, że każda
//! operacja ze skutkiem ubocznym musi być bezpieczna przy powtórzeniu.

use uuid::Uuid;

use crate::{Channel, Transition};

fn channel_str(c: Channel) -> &'static str {
    match c {
        Channel::Ui => "ui",
        Channel::Email => "email",
        Channel::Sms => "sms",
        Channel::Phone => "phone",
    }
}

fn transition_str(t: Transition) -> &'static str {
    match t {
        Transition::Opened => "opened",
        Transition::Reopened => "reopened",
        Transition::Acked => "acked",
        Transition::Resolved => "resolved",
    }
}

/// Przejście stanu alertu: `(fingerprint, episode, transition)`.
pub fn alert_transition(fingerprint: &str, episode: i32, transition: Transition) -> String {
    format!(
        "alert:{fingerprint}:{episode}:{}",
        transition_str(transition)
    )
}

/// Dostarczenie: `(alert_id, episode, person_id, channel)` – co najwyżej jedno
/// powiadomienie osoby na kanale o danym epizodzie.
pub fn delivery(alert_id: Uuid, episode: i32, person_id: &str, channel: Channel) -> String {
    format!(
        "delivery:{alert_id}:{episode}:{person_id}:{}",
        channel_str(channel)
    )
}

/// Uruchomienie runbooka: `(alert_id, episode, runbook_id)` – raz na epizod.
pub fn runbook_run(alert_id: Uuid, episode: i32, runbook_id: &str) -> String {
    format!("run:{alert_id}:{episode}:{runbook_id}")
}

/// Wykonanie kroku: `(run_id, step_id, attempt)`. Wysyłany też jako nagłówek
/// `Idempotency-Key` w akcjach HTTP.
pub fn step(run_id: Uuid, step_id: &str, attempt: u32) -> String {
    format!("step:{run_id}:{step_id}:{attempt}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_stable_and_distinct() {
        let a = Uuid::nil();
        assert_eq!(
            delivery(a, 1, "anna", Channel::Sms),
            format!("delivery:{a}:1:anna:sms")
        );
        assert_ne!(
            delivery(a, 1, "anna", Channel::Sms),
            delivery(a, 2, "anna", Channel::Sms)
        );
        assert_eq!(
            alert_transition("F", 2, Transition::Reopened),
            "alert:F:2:reopened"
        );
        assert_eq!(
            runbook_run(a, 1, "settlement-timeout"),
            format!("run:{a}:1:settlement-timeout")
        );
        assert_eq!(step(a, "s1", 0), format!("step:{a}:s1:0"));
    }
}
