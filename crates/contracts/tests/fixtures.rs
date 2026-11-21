//! Każdy fixture deserializuje się do swojego typu, a ponowna serializacja daje
//! identyczny JSON (round-trip). Pilnuje zgodności fixtures z kontraktem.

use std::fmt::Debug;
use std::path::PathBuf;

use contracts::*;
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;

fn fixture(rel: &str) -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("fixtures")
        .join(rel);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn roundtrip<T: Serialize + DeserializeOwned + PartialEq + Debug>(rel: &str) -> T {
    let original = fixture(rel);
    let typed: T =
        serde_json::from_value(original.clone()).unwrap_or_else(|e| panic!("{rel}: {e}"));
    let reserialized = serde_json::to_value(&typed).unwrap();
    assert_eq!(
        reserialized, original,
        "{rel}: re-serialized JSON differs from fixture"
    );
    let again: T = serde_json::from_value(reserialized).unwrap();
    assert_eq!(again, typed, "{rel}: round-trip changed the value");
    typed
}

#[test]
fn appendix_a_events() {
    for f in [
        "workflow_failed",
        "alertmanager_high_error_rate",
        "trade_rejected",
    ] {
        let env: IncomingEnvelope = roundtrip(&format!("events/{f}.json"));
        assert_eq!(env.format, formats::JSON);
    }
}

#[test]
fn every_topic_has_a_fixture() {
    for topic in topics::ALL {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("fixtures/topics/{topic}.json"));
        assert!(path.exists(), "missing fixture for topic {topic}");
    }
}

#[test]
fn topic_messages() {
    roundtrip::<Envelope>("topics/events.raw.json");
    roundtrip::<Envelope>("topics/events.unmapped.json");
    let ev: CanonicalEvent = roundtrip("topics/events.normalized.json");
    assert_eq!(ev.severity, Severity::Error);
    roundtrip::<AlertCommand>("topics/alerts.commands.json");
    let changed: AlertChanged = roundtrip("topics/alerts.changed.json");
    assert_eq!(changed.transition, Transition::Opened);
    let req: DeliveryRequest = roundtrip("topics/deliveries.requested.json");
    assert_eq!(req.current_channel(), Some(Channel::Sms));
    assert_eq!(req.chain.last(), Some(&Channel::Ui));
    let retry: DeliveryRequest = roundtrip("topics/deliveries.retry.json");
    assert!(retry.not_before.is_some());
    let dlq: DeliveryRequest = roundtrip("topics/deliveries.dlq.json");
    assert_eq!(dlq.current_channel(), Some(Channel::Ui));
    let cmd: RunbookCommand = roundtrip("topics/runbooks.commands.json");
    assert!(matches!(cmd.kind, RunbookCommandKind::Done { .. }));
    roundtrip::<UiEvent>("topics/ui.events.json");
}

#[test]
fn config_definitions() {
    for f in ["mapping.workflow", "mapping.trade", "mapping.alertmanager"] {
        roundtrip::<MappingDef>(&format!("config/{f}.json"));
    }
    for f in ["rule.settlements", "rule.rates"] {
        let rule: RuleDef = roundtrip(&format!("config/{f}.json"));
        assert!(matches!(rule.condition, Condition::All { .. }));
    }
    roundtrip::<PersonDef>("config/person.anna.json");
    roundtrip::<TeamDef>("config/team.settlements.json");
    let action: ActionDef = roundtrip("config/action.flush_cache.json");
    assert_eq!(action.risk, Risk::High);
    let auto: RunbookDef = roundtrip("config/runbook.settlement_timeout.json");
    assert_eq!(auto.steps[2].approval, Approval::Required);
    let manual: RunbookDef = roundtrip("config/runbook.payments_manual.json");
    assert_eq!(manual.kind, RunbookKind::Manual);
}

#[test]
fn types_without_fixtures_roundtrip() {
    let explanation = Explanation {
        result: false,
        node: "all".into(),
        actual: None,
        children: vec![Explanation {
            result: false,
            node: "raw.payload.notional gt 10000000".into(),
            actual: Some(serde_json::json!(5000000)),
            children: vec![],
        }],
    };
    let v = serde_json::to_value(&explanation).unwrap();
    assert_eq!(
        serde_json::from_value::<Explanation>(v).unwrap(),
        explanation
    );

    for s in [StepStatus::WaitingApproval, StepStatus::Rejected] {
        let v = serde_json::to_value(s).unwrap();
        assert_eq!(serde_json::from_value::<StepStatus>(v).unwrap(), s);
    }
    for s in [DeliveryStatus::Fallback, DeliveryStatus::Dlq] {
        let v = serde_json::to_value(s).unwrap();
        assert_eq!(serde_json::from_value::<DeliveryStatus>(v).unwrap(), s);
    }
    assert_eq!(
        serde_json::to_value(AlertState::Reopened).unwrap(),
        "reopened"
    );
    assert!(Severity::Critical > Severity::Warning);
}
