//! Kompleksowe testy jednostkowe i integracyjne routera (tasks 7.1, 8.1, 8.2, 8.3).
//! Pokrywają scenariusze z `specs/alert-rules/spec.md`, `specs/recipient-routing/spec.md`
//! oraz `specs/notification-delivery/spec.md`.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use chrono::{TimeZone, Utc};
use contracts::{
    AlertChanged, AlertState, CanonicalEvent, Channel, Condition, DeliveryOrigin, Op, PersonDef,
    PreferenceDef, Presentation, Recipient, RuleDef, RunbookDef, Severity, TeamDef, Transition,
};
use router::{
    default_fallback_chain, is_within_hours, parse_and_validate_rule_yaml, route_alert,
    CompiledManualRunbook, CompiledRule, PeopleConfigFile, RouterSnapshot,
};
use serde_json::{json, Map};
use uuid::Uuid;

fn load_repo_snapshot() -> RouterSnapshot {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();

    // Load people.yaml
    let people_yaml =
        std::fs::read_to_string(repo_root.join("config/people.yaml")).expect("people.yaml");
    let people_cfg: PeopleConfigFile = serde_yaml::from_str(&people_yaml).expect("parse people");

    let mut people = HashMap::new();
    for p in people_cfg.people {
        people.insert(p.id.clone(), p);
    }
    let mut teams = HashMap::new();
    for t in people_cfg.teams {
        teams.insert(t.id.clone(), t);
    }

    // Load rules/*.yaml
    let rules_dir = repo_root.join("config/rules");
    let mut rule_paths: Vec<_> = std::fs::read_dir(&rules_dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            matches!(
                p.extension().and_then(|s| s.to_str()),
                Some("yaml" | "yml")
            )
        })
        .collect();
    rule_paths.sort();

    let mut rules = Vec::new();
    for p in rule_paths {
        let content = std::fs::read_to_string(&p).unwrap();
        let def = parse_and_validate_rule_yaml(&content).expect("valid rule yaml");
        rules.push(CompiledRule::compile(def).unwrap());
    }

    // Load manual runbook fixture
    let rb_json = std::fs::read_to_string(
        repo_root.join("crates/contracts/fixtures/config/runbook.payments_manual.json"),
    )
    .unwrap();
    let rb_def: RunbookDef = serde_json::from_str(&rb_json).unwrap();
    let manual_runbooks = vec![CompiledManualRunbook::compile(rb_def).unwrap()];

    RouterSnapshot {
        rules,
        people,
        teams,
        manual_runbooks,
    }
}

fn workflow_alert_fixture() -> AlertChanged {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = manifest_dir.parent().unwrap().parent().unwrap();
    let json_str = std::fs::read_to_string(
        repo_root.join("crates/contracts/fixtures/topics/alerts.changed.json"),
    )
    .unwrap();
    serde_json::from_str(&json_str).unwrap()
}

fn payments_critical_alert(at: chrono::DateTime<Utc>) -> AlertChanged {
    let mut fields = Map::new();
    fields.insert("service".into(), json!("payments-api"));
    fields.insert("region".into(), json!("eu-central-1"));

    let ev = CanonicalEvent {
        event_id: Uuid::nil(),
        schema: "alertmanager.v1".into(),
        source: "webhook:alertmanager".into(),
        severity: Severity::Critical,
        title: "HighErrorRate on payments-api".into(),
        fingerprint: "alertmanager.v1|HighErrorRate|payments-api".into(),
        resolved: false,
        fields,
        raw: json!({
            "alertname": "HighErrorRate",
            "status": "firing",
            "labels": {
                "service": "payments-api",
                "severity": "critical",
                "region": "eu-central-1"
            },
            "annotations": {
                "summary": "5xx rate above 8% for 5m"
            }
        }),
        received_at: at,
        mapping_version: 1,
    };

    AlertChanged {
        alert_id: Uuid::parse_str("0192f0a0-0000-7000-8000-0000000000a2").unwrap(),
        fingerprint: ev.fingerprint.clone(),
        episode: 1,
        transition: Transition::Opened,
        state: AlertState::Open,
        severity: Severity::Critical,
        count: 1,
        event: ev,
        actor: None,
        at,
    }
}

#[test]
fn seed_files_parse_and_compile_cleanly() {
    let snap = load_repo_snapshot();
    assert_eq!(snap.rules.len(), 4);
    assert!(snap.people.contains_key("anna"));
    assert!(snap.people.contains_key("jan"));
    assert!(snap.people.contains_key("ola"));
    assert!(snap.people.contains_key("piotr"));
    assert!(snap.people.contains_key("tomek"));
    assert!(snap.people.contains_key("kasia"));
    assert!(snap.teams.contains_key("settlements"));
    assert!(snap.teams.contains_key("rates"));
    assert!(snap.teams.contains_key("pricing"));
    assert!(snap.teams.contains_key("payments"));
}

#[test]
fn settlements_rule_expands_team_and_overrides_severity_to_critical() {
    // Scenario: Reguła na zespół (team:settlements -> jan, ola) + Nadpisanie severity na critical
    // settlements team has critical -> [phone, sms], jan & ola have fallback [email]
    let snap = load_repo_snapshot();
    let alert = workflow_alert_fixture();
    assert_eq!(alert.severity, Severity::Error);

    let reqs = route_alert(&snap, &alert, "http://localhost:5173");
    // jan: phone, sms; ola: phone, sms -> 4 DeliveryRequests
    assert_eq!(reqs.len(), 4);

    for r in &reqs {
        assert_eq!(r.severity, Severity::Critical);
        assert_eq!(
            r.origin,
            DeliveryOrigin::Rule {
                rule_id: "settlements-prod".into()
            }
        );
        // Łańcuch zawsze kończy się Channel::Ui
        assert_eq!(r.chain.last(), Some(&Channel::Ui));
    }

    let jan_sms = reqs
        .iter()
        .find(|r| r.person_id == "jan" && r.current_channel() == Some(Channel::Sms))
        .expect("jan sms delivery");
    assert_eq!(jan_sms.chain, vec![Channel::Sms, Channel::Email, Channel::Ui]);
    assert_eq!(
        jan_sms.rendered[&Channel::Sms].title,
        "WORKFLOW_FAILED order-settlement"
    );
    assert!(jan_sms.rendered[&Channel::Sms]
        .body
        .contains("Step validate-counterparty failed: Counterparty service did not respond in 5000 ms"));
    assert_eq!(
        jan_sms.rendered[&Channel::Sms].link.as_deref(),
        Some("http://localhost:5173/alerts/0192f0a0-0000-7000-8000-0000000000a1")
    );

    let ola_phone = reqs
        .iter()
        .find(|r| r.person_id == "ola" && r.current_channel() == Some(Channel::Phone))
        .expect("ola phone delivery");
    assert_eq!(
        ola_phone.chain,
        vec![Channel::Phone, Channel::Email, Channel::Ui]
    );
}

#[test]
fn anna_working_hours_vs_after_hours_in_warsaw_timezone() {
    // Scenario: Anna poza godzinami (08:00-20:00 -> sms, 20:00-08:00 -> email w Europe/Warsaw)
    let snap = load_repo_snapshot();

    // W październiku 2026 (2026-10-23) w Warszawie obowiązuje CEST (UTC+2).
    // 10:00 UTC = 12:00 Europe/Warsaw (w godzinach 08:00-20:00 -> SMS)
    let daytime_utc = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    assert!(is_within_hours(daytime_utc, "Europe/Warsaw", "08:00-20:00"));
    assert!(!is_within_hours(daytime_utc, "Europe/Warsaw", "20:00-08:00"));

    let alert_day = payments_critical_alert(daytime_utc);
    let reqs_day = route_alert(&snap, &alert_day, "http://localhost:5173");
    assert_eq!(reqs_day.len(), 1);
    assert_eq!(reqs_day[0].person_id, "anna");
    assert_eq!(reqs_day[0].current_channel(), Some(Channel::Sms));
    assert_eq!(
        reqs_day[0].chain,
        vec![Channel::Sms, Channel::Email, Channel::Ui]
    );

    // 20:00 UTC = 22:00 Europe/Warsaw (poza godzinami 08:00-20:00 -> Email)
    let night_utc = Utc.with_ymd_and_hms(2026, 10, 23, 20, 0, 0).unwrap();
    assert!(!is_within_hours(night_utc, "Europe/Warsaw", "08:00-20:00"));
    assert!(is_within_hours(night_utc, "Europe/Warsaw", "20:00-08:00"));

    let alert_night = payments_critical_alert(night_utc);
    let reqs_night = route_alert(&snap, &alert_night, "http://localhost:5173");
    assert_eq!(reqs_night.len(), 1);
    assert_eq!(reqs_night[0].person_id, "anna");
    assert_eq!(reqs_night[0].current_channel(), Some(Channel::Email));
    assert_eq!(reqs_night[0].chain, vec![Channel::Email, Channel::Ui]);
}

#[test]
fn three_person_team_with_distinct_preferences_and_team_defaults() {
    // Scenarios:
    // - "Zespół trzyosobowy" (trzej członkowie o różnych preferencjach)
    // - "Osoba bez własnych preferencji" (dziedziczy z zespołu)
    // - "Brak jakichkolwiek preferencji" (trafia do UI)
    let mut people = HashMap::new();
    // Członek 1: własna preferencja warning -> sms
    people.insert(
        "m1".into(),
        PersonDef {
            id: "m1".into(),
            name: "Member 1".into(),
            timezone: "Europe/Warsaw".into(),
            contacts: BTreeMap::from([(Channel::Sms, "+48111".into())]),
            preferences: vec![PreferenceDef {
                severity: Severity::Warning,
                channels: vec![Channel::Sms],
                hours: None,
            }],
            fallback: vec![Channel::Email],
        },
    );
    // Członek 2: własna preferencja warning -> phone
    people.insert(
        "m2".into(),
        PersonDef {
            id: "m2".into(),
            name: "Member 2".into(),
            timezone: "Europe/Warsaw".into(),
            contacts: BTreeMap::from([(Channel::Phone, "+48222".into())]),
            preferences: vec![PreferenceDef {
                severity: Severity::Warning,
                channels: vec![Channel::Phone],
                hours: None,
            }],
            fallback: vec![],
        },
    );
    // Członek 3: brak własnych preferencji -> dziedziczy domyślne z zespołu (warning -> email)
    people.insert(
        "m3".into(),
        PersonDef {
            id: "m3".into(),
            name: "Member 3".into(),
            timezone: "Europe/Warsaw".into(),
            contacts: BTreeMap::from([(Channel::Email, "m3@example.com".into())]),
            preferences: vec![],
            fallback: vec![],
        },
    );

    let mut teams = HashMap::new();
    teams.insert(
        "t3".into(),
        TeamDef {
            id: "t3".into(),
            name: "Three Person Team".into(),
            members: vec!["m1".into(), "m2".into(), "m3".into()],
            preferences: vec![PreferenceDef {
                severity: Severity::Warning,
                channels: vec![Channel::Email],
                hours: None,
            }],
        },
    );

    let rule = CompiledRule::compile(RuleDef {
        id: "r-t3".into(),
        name: "Three Person Rule".into(),
        team: Some("t3".into()),
        condition: Condition::Leaf {
            field: "schema".into(),
            op: Op::Eq,
            value: json!("workflow.v1"),
        },
        recipients: vec![Recipient::Team("t3".into())],
        severity_override: None,
        presentation: Presentation::default(),
        enabled: true,
        version: 1,
    })
    .unwrap();

    let snap = RouterSnapshot {
        rules: vec![rule],
        people,
        teams,
        manual_runbooks: vec![],
    };

    let mut alert = workflow_alert_fixture();
    alert.severity = Severity::Warning;

    let reqs = route_alert(&snap, &alert, "http://localhost:5173");
    assert_eq!(reqs.len(), 3);
    let ch_by_person: HashMap<String, Channel> = reqs
        .iter()
        .map(|r| (r.person_id.clone(), r.current_channel().unwrap()))
        .collect();
    assert_eq!(ch_by_person.get("m1"), Some(&Channel::Sms));
    assert_eq!(ch_by_person.get("m2"), Some(&Channel::Phone));
    assert_eq!(ch_by_person.get("m3"), Some(&Channel::Email));

    // Scenario: Brak jakichkolwiek preferencji (dla Severity::Info ani m1..m3, ani zespół t3 nie mają preferencji -> wszyscy dostają UI)
    alert.severity = Severity::Info;
    let reqs_info = route_alert(&snap, &alert, "http://localhost:5173");
    assert_eq!(reqs_info.len(), 3);
    for r in &reqs_info {
        assert_eq!(r.current_channel(), Some(Channel::Ui));
        assert_eq!(r.chain, vec![Channel::Ui]);
    }
}

#[test]
fn person_in_two_matching_rules_is_deduplicated_per_channel() {
    // Scenario: "Osoba w dwóch regułach"
    // GIVEN Anna jest odbiorcą reguły zespołowej (team:payments) i reguły osobistej (person:anna),
    // które pasują do tego samego alertu
    // WHEN alert zostaje otwarty
    // THEN Anna dostaje jedno powiadomienie na każdy swój kanał
    let mut snap = load_repo_snapshot();

    // Dodajemy drugą regułę wskazującą `team:payments` (którego członkiem jest Anna), pasującą do tego samego alertu
    let team_payments_rule = CompiledRule::compile(RuleDef {
        id: "team-payments-all".into(),
        name: "Team Payments Rule".into(),
        team: Some("payments".into()),
        condition: Condition::Leaf {
            field: "raw.labels.service".into(),
            op: Op::Eq,
            value: json!("payments-api"),
        },
        recipients: vec![Recipient::Team("payments".into())],
        severity_override: Some(Severity::Critical),
        presentation: Presentation {
            title: Some("Payments Alert: {{title}}".into()),
            body: Some("Team payments notification".into()),
            ..Default::default()
        },
        enabled: true,
        version: 1,
    })
    .unwrap();
    snap.rules.push(team_payments_rule);

    // O 12:00 czasu warszawskiego (10:00 UTC) Anna ma dla critical kanał `[sms]`
    let daytime_utc = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let alert = payments_critical_alert(daytime_utc);

    let reqs = route_alert(&snap, &alert, "http://localhost:5173");
    // Mimo że pasują DWIE reguły (`anna-payments-critical` oraz `team-payments-all`),
    // Anna dostaje dokładnie JEDNO zlecenie na kanale SMS!
    let anna_sms: Vec<_> = reqs
        .iter()
        .filter(|r| r.person_id == "anna" && r.current_channel() == Some(Channel::Sms))
        .collect();
    assert_eq!(anna_sms.len(), 1);
}

#[test]
fn manual_runbook_is_attached_in_full_to_email_and_ui_but_only_link_in_sms() {
    // Scenario: "SMS z linkiem" & "Treść dopasowana do kanału"
    // GIVEN alert z dopasowanym runbookiem MANUAL (`payments-high-error-rate`)
    // WHEN wysyłany jest SMS / Email / UI
    // THEN SMS zawiera tytuł alertu i link do jego szczegółów w UI, bez pełnej treści runbooka,
    //      natomiast Email i UI zawierają pełną treść i dołączony runbook MANUAL.
    let snap = load_repo_snapshot();
    let daytime_utc = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let alert = payments_critical_alert(daytime_utc);

    let reqs = route_alert(&snap, &alert, "http://localhost:5173");
    assert_eq!(reqs.len(), 1);
    let req = &reqs[0];
    // Łańcuch Anny w dzień: Sms -> Email -> Ui
    assert_eq!(req.chain, vec![Channel::Sms, Channel::Email, Channel::Ui]);

    let sms = &req.rendered[&Channel::Sms];
    let email = &req.rendered[&Channel::Email];
    let ui = &req.rendered[&Channel::Ui];

    let expected_link = "http://localhost:5173/alerts/0192f0a0-0000-7000-8000-0000000000a2";
    assert_eq!(sms.link.as_deref(), Some(expected_link));
    assert!(sms.body.contains("HighErrorRate on payments-api"));
    assert!(sms.body.contains(expected_link));
    // SMS NIE zawiera pełnej treści runbooka
    assert!(!sms.body.contains("Open the payments dashboard"));
    assert!(!sms.body.contains("#payments-incidents"));

    // Email i UI zawierają pełną treść runbooka MANUAL
    assert!(email.body.contains("Open the payments dashboard"));
    assert!(email.body.contains("#payments-incidents"));
    assert_eq!(email.link.as_deref(), Some(expected_link));

    assert!(ui.body.contains("Open the payments dashboard"));
    assert!(ui.body.contains("#payments-incidents"));
    assert_eq!(ui.link, None);
}

#[test]
fn acked_and_resolved_transitions_produce_no_deliveries() {
    let snap = load_repo_snapshot();
    let mut alert = workflow_alert_fixture();

    alert.transition = Transition::Acked;
    assert!(route_alert(&snap, &alert, "http://localhost:5173").is_empty());

    alert.transition = Transition::Resolved;
    assert!(route_alert(&snap, &alert, "http://localhost:5173").is_empty());

    alert.transition = Transition::Reopened;
    assert_eq!(route_alert(&snap, &alert, "http://localhost:5173").len(), 4);
}

#[test]
fn fallback_chain_always_ends_with_ui_without_duplicates() {
    assert_eq!(
        default_fallback_chain(Channel::Sms, &[Channel::Email]),
        vec![Channel::Sms, Channel::Email, Channel::Ui]
    );
    assert_eq!(
        default_fallback_chain(Channel::Email, &[Channel::Email, Channel::Ui]),
        vec![Channel::Email, Channel::Ui]
    );
    assert_eq!(
        default_fallback_chain(Channel::Ui, &[Channel::Email]),
        vec![Channel::Ui]
    );
    assert_eq!(
        default_fallback_chain(Channel::Sms, &[Channel::Ui, Channel::Email]),
        vec![Channel::Sms, Channel::Email, Channel::Ui]
    );
}

#[test]
fn hot_reload_in_memory_store_applies_rule_and_people_changes_without_restart() {
    // Scenarios:
    // - "Edycja reguły w UI" / "Edycja pliku" (specs/alert-rules/spec.md: zmiana progu notional w regule Rates)
    // - "Dodanie osoby do zespołu" (specs/recipient-routing/spec.md: dodanie osoby do zespołu w pliku people.yaml)
    let store = router::new_store();
    let initial_snap = load_repo_snapshot();
    store.store(std::sync::Arc::new(initial_snap));

    // Przygotowujemy alert trade.rejected.v2 z notional = 5_000_000 (poniżej progu 10_000_000 w rates-big-rejections)
    let at = Utc.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
    let trade_alert = AlertChanged {
        alert_id: Uuid::new_v4(),
        fingerprint: "trade.rejected.v2|T-100".into(),
        episode: 1,
        transition: Transition::Opened,
        state: AlertState::Open,
        severity: Severity::Warning,
        count: 1,
        event: CanonicalEvent {
            event_id: Uuid::new_v4(),
            schema: "trade.rejected.v2".into(),
            source: "kafka:trades".into(),
            severity: Severity::Warning,
            title: "Rejected EUR 5000000".into(),
            fingerprint: "trade.rejected.v2|T-100".into(),
            resolved: false,
            fields: Map::new(),
            raw: json!({
                "payload": {
                    "tradeId": "T-100",
                    "desk": "rates",
                    "notional": 5_000_000,
                    "currency": "EUR",
                    "reason": "LIMIT_BREACH"
                }
            }),
            received_at: at,
            mapping_version: 1,
        },
        actor: None,
        at,
    };

    // Przed zmianą progu: notional 5M < 10M -> brak dostarczeń
    assert!(route_alert(&store.load_full(), &trade_alert, "http://localhost:5173").is_empty());

    // Zmiana progu notional w regule Rates na 1_000_000 (hot reload snapshotu)
    let mut updated_snap = (*store.load_full()).clone();
    let updated_rates_yaml = r#"
id: rates-big-rejections
name: "Rates: large rejected trades"
team: rates
condition:
  all:
    - field: schema
      op: glob
      value: "trade.rejected.*"
    - field: raw.payload.desk
      op: eq
      value: rates
    - field: raw.payload.notional
      op: gt
      value: 1000000
recipients:
  - type: team
    id: rates
severity_override: critical
presentation:
  title: "Rejected {{raw.payload.currency}} {{raw.payload.notional}} — {{raw.payload.reason}}"
  body: null
  fields:
    - raw.payload.tradeId
    - raw.payload.desk
  highlight: {}
  color: null
enabled: true
version: 2
"#;
    let new_rates_def = parse_and_validate_rule_yaml(updated_rates_yaml).unwrap();
    let new_rates_rule = CompiledRule::compile(new_rates_def).unwrap();
    for r in &mut updated_snap.rules {
        if r.def.id == "rates-big-rejections" {
            *r = new_rates_rule.clone();
        }
    }
    store.store(std::sync::Arc::new(updated_snap));

    // Po hot reloadzie reguły: alert pasuje i trafia do członków zespołu rates (piotr, anna)
    let reqs_after_rule_reload =
        route_alert(&store.load_full(), &trade_alert, "http://localhost:5173");
    let recipients_before: std::collections::HashSet<String> = reqs_after_rule_reload
        .iter()
        .map(|r| r.person_id.clone())
        .collect();
    assert!(recipients_before.contains("piotr"));
    assert!(recipients_before.contains("anna"));
    assert!(!recipients_before.contains("kasia"));
    // Sprawdzenie wyrenderowanego tytułu wg szablonu reguły Rates
    assert_eq!(
        reqs_after_rule_reload[0].rendered[&reqs_after_rule_reload[0].chain[0]].title,
        "Rejected EUR 5000000 — LIMIT_BREACH"
    );

    // Hot reload people.yaml: dodanie Kasi do zespołu rates bez restartu
    let mut snap_with_kasia = (*store.load_full()).clone();
    snap_with_kasia
        .teams
        .get_mut("rates")
        .unwrap()
        .members
        .push("kasia".into());
    store.store(std::sync::Arc::new(snap_with_kasia));

    let reqs_after_people_reload =
        route_alert(&store.load_full(), &trade_alert, "http://localhost:5173");
    let recipients_after: std::collections::HashSet<String> = reqs_after_people_reload
        .iter()
        .map(|r| r.person_id.clone())
        .collect();
    assert!(recipients_after.contains("piotr"));
    assert!(recipients_after.contains("anna"));
    assert!(recipients_after.contains("kasia"));
}

#[test]
fn pricing_warnings_rule_routes_to_ui_as_primary_channel() {
    let snap = load_repo_snapshot();
    let now = Utc::now();

    let mut fields = serde_json::Map::new();
    fields.insert("service".into(), serde_json::json!("pricing-engine-7"));
    let event = CanonicalEvent {
        event_id: Uuid::new_v4(),
        schema: "pricing.log.v1".into(),
        source: "http".into(),
        severity: Severity::Warning,
        title: "Pricing latency > 1s".into(),
        fingerprint: "pricing.log.v1|pricing-engine-7|latency".into(),
        resolved: false,
        fields,
        raw: serde_json::json!({
            "service": "pricing-engine-7",
            "level": "WARN",
            "message": "Pricing latency > 1s"
        }),
        received_at: now,
        mapping_version: 1,
    };
    let alert = AlertChanged {
        alert_id: Uuid::new_v4(),
        fingerprint: event.fingerprint.clone(),
        episode: 1,
        transition: Transition::Opened,
        state: AlertState::Open,
        severity: Severity::Warning,
        count: 1,
        event,
        actor: None,
        at: now,
    };

    let reqs = route_alert(&snap, &alert, "http://localhost:5173");
    assert_eq!(reqs.len(), 1, "pricing-warnings kieruje do zespołu pricing (tomek) na kanał UI");
    let req = &reqs[0];
    assert_eq!(req.person_id, "tomek");
    assert_eq!(req.origin, DeliveryOrigin::Rule { rule_id: "pricing-warnings".into() });
    assert_eq!(req.chain, vec![Channel::Ui]);
    assert_eq!(req.rendered[&Channel::Ui].title, "Pricing warning: Pricing latency > 1s");

    // Severity INFO poniżej progu `gte warning` nie powinno dopasować reguły.
    let mut info_alert = alert;
    info_alert.severity = Severity::Info;
    info_alert.event.severity = Severity::Info;
    assert!(route_alert(&snap, &info_alert, "http://localhost:5173").is_empty());
}

