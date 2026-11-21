//! Akcje wbudowane `escalate` i `notify` (zadanie 10.1, design D7, D11).
//!
//! Krok runbooka zamienia się w zlecenia `deliveries.requested` z `origin = Action`.
//! Kanały i łańcuch zapasowy wybiera ta sama logika co dla reguł (`router::resolve_person_channels`,
//! `router::default_fallback_chain`), więc preferencje osób i godziny działają tak samo.
//! Sender deduplikuje po `keys::delivery(alert, epizod, osoba, kanał)`: osoba, która o tym
//! epizodzie dostała już powiadomienie z reguły na tym samym kanale, nie dostanie go drugi raz.
//! Dlatego silnik sprawdza przed publikacją tabelę `deliveries` (P2, tylko odczyt) i takich odbiorców
//! nie zleca, a dziennik mówi wprost, kto został pominięty (`summary`).

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use contracts::{Channel, DeliveryOrigin, DeliveryRequest, Rendered, Severity, StepDef, Timestamp};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::store::Snapshot;

/// Dane alertu i kroku potrzebne do treści powiadomienia.
pub struct Notice {
    pub action: String,
    pub alert_id: Uuid,
    pub episode: i32,
    pub run_id: Uuid,
    pub step_id: String,
    pub severity: Severity,
    pub received_at: Timestamp,
    pub alert_title: String,
    pub runbook_title: String,
    pub step_text: String,
    pub link: String,
}

impl Notice {
    /// `context` to kontekst alertu zapisany w runie (`matching::alert_context`).
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action: &str,
        alert_id: Uuid,
        episode: i32,
        run_id: Uuid,
        runbook_title: &str,
        step: &StepDef,
        context: &Value,
        public_ui_url: &str,
    ) -> Self {
        Self {
            action: action.to_string(),
            alert_id,
            episode,
            run_id,
            step_id: step.id.clone(),
            severity: serde_json::from_value(context["severity"].clone())
                .unwrap_or(Severity::Error),
            received_at: serde_json::from_value(context["received_at"].clone())
                .unwrap_or_else(|_| Utc::now()),
            alert_title: context["title"].as_str().unwrap_or("alert").to_string(),
            runbook_title: runbook_title.to_string(),
            step_text: step.text.clone(),
            link: format!("{}/alerts/{alert_id}", public_ui_url.trim_end_matches('/')),
        }
    }
}

/// Odbiorcy: `escalate` → członkowie zespołu z `team`; `notify` → osoby z `people` (lista) albo `person`.
/// Para (osoba, kontekst zespołu) – zespół daje domyślne preferencje.
fn recipients(
    n: &Notice,
    params: &Map<String, Value>,
    snap: &Snapshot,
) -> Result<Vec<(String, Option<String>)>, String> {
    match n.action.as_str() {
        "escalate" => {
            let team_id = params
                .get("team")
                .and_then(Value::as_str)
                .ok_or("brak parametru `team`")?;
            let team = snap
                .teams
                .get(team_id)
                .ok_or_else(|| format!("zespół `{team_id}` nie istnieje w katalogu osób"))?;
            if team.members.is_empty() {
                return Err(format!("zespół `{team_id}` nie ma członków"));
            }
            Ok(team
                .members
                .iter()
                .map(|m| (m.clone(), Some(team_id.to_string())))
                .collect())
        }
        "notify" => {
            let people: Vec<String> = match (params.get("people"), params.get("person")) {
                (Some(Value::Array(list)), _) => list
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect(),
                (_, Some(Value::String(one))) => vec![one.clone()],
                _ => return Err("brak parametru `people` (lista) albo `person`".into()),
            };
            Ok(people.into_iter().map(|p| (p, None)).collect())
        }
        other => Err(format!("`{other}` nie jest akcją powiadamiającą")),
    }
}

fn render(n: &Notice, channel: Channel) -> Rendered {
    let label = if n.action == "escalate" {
        "Eskalacja"
    } else {
        "Powiadomienie z runbooka"
    };
    let title = format!("{label}: {}", n.alert_title);
    let body = match channel {
        Channel::Sms => format!("{label}: {}. Szczegóły: {}", n.alert_title, n.link),
        Channel::Phone => format!(
            "{label}. Alert: {}. Krok runbooka: {}.",
            n.alert_title, n.step_text
        ),
        Channel::Email | Channel::Ui => {
            format!(
                "Runbook „{}”, krok {}: {}\nAlert: {}\nSzczegóły: {}",
                n.runbook_title, n.step_id, n.step_text, n.alert_title, n.link
            )
        }
    };
    Rendered {
        title,
        body,
        link: Some(n.link.clone()),
    }
}

/// Zlecenia dostarczenia dla kroku i lista nieznanych osób (do dziennika, zob. `summary`).
/// Błąd = żaden odbiorca nie został ustalony (krok kończy się `failed`, operator może ponowić albo pominąć).
pub fn build(
    n: &Notice,
    params: &Map<String, Value>,
    snap: &Snapshot,
    now: DateTime<Utc>,
) -> Result<(Vec<DeliveryRequest>, Vec<String>), String> {
    let mut requests = Vec::new();
    let mut seen = HashSet::new();
    let mut unknown = Vec::new();
    for (person_id, team_ctx) in recipients(n, params, snap)? {
        let Some(person) = snap.people.get(&person_id) else {
            unknown.push(person_id);
            continue;
        };
        let channels = router::resolve_person_channels(
            person,
            team_ctx.as_deref(),
            &snap.teams,
            n.severity,
            now,
        );
        for primary in channels {
            if !seen.insert((person.id.clone(), primary)) {
                continue;
            }
            let chain = router::default_fallback_chain(primary, &person.fallback);
            let rendered: BTreeMap<Channel, Rendered> =
                chain.iter().map(|&c| (c, render(n, c))).collect();
            requests.push(DeliveryRequest {
                delivery_id: Uuid::now_v7(),
                alert_id: n.alert_id,
                episode: n.episode,
                person_id: person.id.clone(),
                severity: n.severity,
                origin: DeliveryOrigin::Action {
                    run_id: n.run_id,
                    step_id: n.step_id.clone(),
                },
                chain,
                chain_index: 0,
                contacts: person.contacts.clone(),
                rendered,
                received_at: n.received_at,
                attempt: 0,
                not_before: None,
                last_error: None,
            });
        }
    }
    if requests.is_empty() {
        return Err(if unknown.is_empty() {
            "brak odbiorców".into()
        } else {
            format!("nieznane osoby: {}", unknown.join(", "))
        });
    }
    Ok((requests, unknown))
}

/// Klucz idempotencji, po którym sender rozpozna zlecenie (pierwszy kanał łańcucha).
pub fn delivery_key(r: &DeliveryRequest) -> String {
    contracts::keys::delivery(
        r.alert_id,
        r.episode,
        &r.person_id,
        r.chain.first().copied().unwrap_or(Channel::Ui),
    )
}

fn label(r: &DeliveryRequest) -> String {
    let channel = r
        .chain
        .first()
        .and_then(|c| serde_json::to_value(c).ok())
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default();
    format!("{} ({channel})", r.person_id)
}

/// Uczciwy opis wyniku do dziennika. `already` = klucze dostarczeń, które sender ma już jako `delivered`;
/// tych odbiorców nie zlecamy, bo sender i tak by ich pominął. „Zlecono”, nie „powiadomiono” —
/// dostarczenie jest asynchroniczne, jego wynik widać w zakładce dostarczeń alertu.
pub fn summary(
    requests: &[DeliveryRequest],
    already: &HashSet<String>,
    unknown: &[String],
) -> String {
    let (skipped, sent): (Vec<_>, Vec<_>) = requests
        .iter()
        .partition(|r| already.contains(&delivery_key(r)));
    let mut parts = Vec::new();
    if sent.is_empty() {
        parts.push("nikogo nie powiadomiono".to_string());
    } else {
        parts.push(format!(
            "zlecono powiadomienie: {}",
            sent.iter().map(|r| label(r)).collect::<Vec<_>>().join(", ")
        ));
    }
    if !skipped.is_empty() {
        parts.push(format!(
            "pominięto (już powiadomieni o tym epizodzie na tym kanale): {}",
            skipped
                .iter()
                .map(|r| label(r))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if !unknown.is_empty() {
        parts.push(format!("pominięto nieznane osoby: {}", unknown.join(", ")));
    }
    parts.join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::{PersonDef, TeamDef};
    use serde_json::json;

    fn snapshot() -> Snapshot {
        let kasia: PersonDef = serde_json::from_value(json!({
            "id": "kasia", "name": "Kasia", "contacts": { "email": "kasia@example.com", "sms": "+48500000006" },
            "preferences": [], "fallback": ["sms"]
        }))
        .unwrap();
        let anna: PersonDef = serde_json::from_value(json!({
            "id": "anna", "name": "Anna", "contacts": { "sms": "+48500000001" },
            "preferences": [{ "severity": "error", "channels": ["sms"], "hours": null }]
        }))
        .unwrap();
        let team: TeamDef = serde_json::from_value(json!({
            "id": "counterparty-support", "name": "Counterparty Support", "members": ["kasia"],
            "preferences": [{ "severity": "error", "channels": ["email"], "hours": null }]
        }))
        .unwrap();
        Snapshot {
            people: [("kasia".to_string(), kasia), ("anna".to_string(), anna)].into(),
            teams: [("counterparty-support".to_string(), team)].into(),
            ..Default::default()
        }
    }

    fn notice(action: &str) -> Notice {
        let step: StepDef = serde_json::from_value(json!({
            "id": "s3", "type": "action", "text": "Escalate to counterparty-support", "action": action,
            "params": {}, "approval": "required", "condition": null, "automation": "automated"
        }))
        .unwrap();
        let ctx = json!({ "severity": "error", "title": "Counterparty service did not respond", "received_at": "2026-10-23T09:14:02.500Z" });
        Notice::new(
            action,
            Uuid::nil(),
            1,
            Uuid::nil(),
            "Settlement timeout",
            &step,
            &ctx,
            "http://localhost:5173/",
        )
    }

    fn params(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn escalate_creates_requests_for_team_members_with_team_preferences() {
        let (reqs, unknown) = build(
            &notice("escalate"),
            &params(json!({ "team": "counterparty-support" })),
            &snapshot(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(reqs.len(), 1);
        let r = &reqs[0];
        assert_eq!(
            (
                r.person_id.as_str(),
                r.chain.first().copied(),
                r.chain.last().copied()
            ),
            ("kasia", Some(Channel::Email), Some(Channel::Ui))
        );
        assert_eq!(
            r.origin,
            DeliveryOrigin::Action {
                run_id: Uuid::nil(),
                step_id: "s3".into()
            }
        );
        assert_eq!(r.contacts[&Channel::Email], "kasia@example.com");
        assert!(r.rendered[&Channel::Email]
            .body
            .contains("http://localhost:5173/alerts/"));
        assert!(r.rendered[&Channel::Email].title.starts_with("Eskalacja"));
        assert_eq!(
            summary(&reqs, &HashSet::new(), &unknown),
            "zlecono powiadomienie: kasia (email)"
        );
    }

    #[test]
    fn notify_uses_personal_preferences_and_reports_unknown_people() {
        let (reqs, unknown) = build(
            &notice("notify"),
            &params(json!({ "people": ["anna", "ghost"] })),
            &snapshot(),
            Utc::now(),
        )
        .unwrap();
        assert_eq!(
            reqs.iter()
                .map(|r| (r.person_id.as_str(), r.chain[0]))
                .collect::<Vec<_>>(),
            [("anna", Channel::Sms)]
        );
        let text = summary(&reqs, &HashSet::new(), &unknown);
        assert!(text.contains("pominięto nieznane osoby: ghost"), "{text}");
    }

    #[test]
    fn already_delivered_recipients_are_reported_as_skipped_not_notified() {
        let (reqs, unknown) = build(
            &notice("notify"),
            &params(json!({ "people": ["anna", "kasia"] })),
            &snapshot(),
            Utc::now(),
        )
        .unwrap();
        let anna = reqs.iter().find(|r| r.person_id == "anna").unwrap();
        let already: HashSet<String> = [delivery_key(anna)].into();
        let text = summary(&reqs, &already, &unknown);
        assert!(text.starts_with("zlecono powiadomienie: kasia"), "{text}");
        assert!(
            text.contains("pominięto (już powiadomieni o tym epizodzie na tym kanale): anna (sms)"),
            "{text}"
        );

        let all: HashSet<String> = reqs.iter().map(delivery_key).collect();
        let text = summary(&reqs, &all, &unknown);
        assert!(
            text.starts_with("nikogo nie powiadomiono; pominięto"),
            "{text}"
        );
    }

    #[test]
    fn unknown_team_or_missing_param_fails_the_step() {
        let snap = snapshot();
        assert!(build(
            &notice("escalate"),
            &params(json!({ "team": "nope" })),
            &snap,
            Utc::now()
        )
        .unwrap_err()
        .contains("nie istnieje"));
        assert!(build(&notice("escalate"), &Map::new(), &snap, Utc::now())
            .unwrap_err()
            .contains("team"));
        assert!(build(
            &notice("notify"),
            &params(json!({ "people": ["ghost"] })),
            &snap,
            Utc::now()
        )
        .unwrap_err()
        .contains("ghost"));
    }
}
