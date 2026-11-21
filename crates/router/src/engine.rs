//! Czysty silnik routingu alertów: `AlertChanged` + `RouterSnapshot` -> `Vec<DeliveryRequest>`
//! (design D7, D9, D10; specs `alert-rules`, `recipient-routing`, `notification-delivery`).

use std::collections::{BTreeMap, HashMap};

use chrono::{DateTime, Datelike, Duration as ChronoDuration, NaiveDate, Timelike, Utc, Weekday};
use contracts::{
    AlertChanged, Channel, DeliveryOrigin, DeliveryRequest, PersonDef, PreferenceDef, Recipient,
    Rendered, RuleDef, RunbookKind, Severity, TeamDef, Transition,
};
use serde_json::Value;
use uuid::Uuid;

use crate::config::{default_fallback_chain, CompiledManualRunbook, CompiledRule, RouterSnapshot};

fn severity_rank(s: Severity) -> u8 {
    match s {
        Severity::Info => 0,
        Severity::Warning => 1,
        Severity::Error => 2,
        Severity::Critical => 3,
    }
}

/// Parsuje zapis `"HH:MM"` na liczbę minut od północy (`0..=1440`).
fn parse_hhmm(s: &str) -> Option<u32> {
    let (h_str, m_str) = s.trim().split_once(':')?;
    let h: u32 = h_str.parse().ok()?;
    let m: u32 = m_str.parse().ok()?;
    if h == 24 && m == 0 {
        return Some(1440);
    }
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Zwraca dzień miesiąca ostatniej niedzieli danego miesiąca (`month`: 1..=12).
fn last_sunday_of_month(year: i32, month: u32) -> u32 {
    let last_day = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    };
    for day in (1..=last_day).rev() {
        if let Some(d) = NaiveDate::from_ymd_opt(year, month, day) {
            if d.weekday() == Weekday::Sun {
                return day;
            }
        }
    }
    last_day
}

/// Sprawdza czy w podanym czasie UTC obowiązuje czas letni (EU DST: od ostatniej niedzieli marca 01:00 UTC
/// do ostatniej niedzieli października 01:00 UTC).
fn is_eu_dst(at: DateTime<Utc>) -> bool {
    let year = at.year();
    let march_sun = last_sunday_of_month(year, 3);
    let oct_sun = last_sunday_of_month(year, 10);

    let Some(dst_start) = NaiveDate::from_ymd_opt(year, 3, march_sun)
        .and_then(|d| d.and_hms_opt(1, 0, 0))
        .map(|ndt| ndt.and_utc())
    else {
        return false;
    };
    let Some(dst_end) = NaiveDate::from_ymd_opt(year, 10, oct_sun)
        .and_then(|d| d.and_hms_opt(1, 0, 0))
        .map(|ndt| ndt.and_utc())
    else {
        return false;
    };

    at >= dst_start && at < dst_end
}

/// Wyznacza przesunięcie strefy czasowej IANA (lub `+HH:MM` / `UTC+X`) w minutach względem UTC.
fn timezone_offset_minutes(at: DateTime<Utc>, timezone: &str) -> i64 {
    let tz = timezone.trim();
    match tz {
        "UTC" | "Etc/UTC" | "GMT" | "Z" => 0,
        "Europe/London" | "Europe/Dublin" | "Europe/Lisbon" => {
            if is_eu_dst(at) {
                60
            } else {
                0
            }
        }
        "Europe/Warsaw" | "Europe/Berlin" | "Europe/Paris" | "Europe/Zurich" | "Europe/Madrid"
        | "Europe/Rome" | "Europe/Amsterdam" | "Europe/Brussels" | "Europe/Vienna"
        | "Europe/Prague" | "Europe/Stockholm" | "CET" | "CEST" => {
            if is_eu_dst(at) {
                120
            } else {
                60
            }
        }
        "Europe/Helsinki" | "Europe/Athens" | "Europe/Bucharest" | "Europe/Kyiv" | "EET" => {
            if is_eu_dst(at) {
                180
            } else {
                120
            }
        }
        "America/New_York" | "US/Eastern" => -300,
        "America/Chicago" | "US/Central" => -360,
        "America/Denver" | "US/Mountain" => -420,
        "America/Los_Angeles" | "US/Pacific" => -480,
        "Asia/Tokyo" => 540,
        "Asia/Singapore" | "Asia/Hong_Kong" => 480,
        "Asia/Kolkata" => 330,
        other => {
            let cleaned = other.strip_prefix("UTC").unwrap_or(other);
            if let Some(sign) = cleaned.chars().next() {
                if sign == '+' || sign == '-' {
                    let mult = if sign == '-' { -1 } else { 1 };
                    let rest = &cleaned[1..];
                    if let Some((h_s, m_s)) = rest.split_once(':') {
                        if let (Ok(h), Ok(m)) = (h_s.parse::<i64>(), m_s.parse::<i64>()) {
                            return mult * (h * 60 + m);
                        }
                    } else if let Ok(h) = rest.parse::<i64>() {
                        return mult * h * 60;
                    }
                }
            }
            if is_eu_dst(at) {
                120
            } else {
                60
            }
        }
    }
}

/// Sprawdza czy podany czas UTC mieści się w przedziale godzin `"HH:MM-HH:MM"` w strefie czasowej osoby.
/// Obsługuje zarówno przedziały dzienne (`"08:00-20:00"`), jak i nocne przechodzące przez północ (`"20:00-08:00"`).
pub fn is_within_hours(at: DateTime<Utc>, timezone: &str, hours_spec: &str) -> bool {
    let offset_mins = timezone_offset_minutes(at, timezone);
    let local = at + ChronoDuration::minutes(offset_mins);
    let current_mins = local.hour() * 60 + local.minute();

    let Some((start_str, end_str)) = hours_spec.split_once('-') else {
        return false;
    };
    let (Some(start_mins), Some(end_mins)) = (parse_hhmm(start_str), parse_hhmm(end_str)) else {
        return false;
    };

    if start_mins == end_mins {
        return true;
    }
    if start_mins < end_mins {
        current_mins >= start_mins && current_mins < end_mins
    } else {
        current_mins >= start_mins || current_mins < end_mins
    }
}

/// Szuka kanałów w liście preferencji dla zadanego `severity` i czasu `at` w strefie `timezone`.
/// Preferuje wpisy z dopasowanym przedziałem `hours: Some(...)` przed wpisami całodobowymi `hours: None`.
fn match_preferences(
    prefs: &[PreferenceDef],
    severity: Severity,
    at: DateTime<Utc>,
    timezone: &str,
) -> Option<Vec<Channel>> {
    for pref in prefs {
        if pref.severity == severity {
            if let Some(hours) = &pref.hours {
                if !hours.trim().is_empty() && is_within_hours(at, timezone, hours) {
                    return Some(pref.channels.clone());
                }
            }
        }
    }
    for pref in prefs {
        if pref.severity == severity {
            let no_hours = pref
                .hours
                .as_ref()
                .map(|h| h.trim().is_empty())
                .unwrap_or(true);
            if no_hours {
                return Some(pref.channels.clone());
            }
        }
    }
    None
}

/// Wyznacza listę podstawowych kanałów dla osoby wg jej preferencji, domyślnych preferencji zespołu,
/// lub ostatecznie `[Channel::Ui]` (spec `recipient-routing`).
pub fn resolve_person_channels(
    person: &PersonDef,
    team_context: Option<&str>,
    teams: &HashMap<String, TeamDef>,
    severity: Severity,
    at: DateTime<Utc>,
) -> Vec<Channel> {
    if let Some(channels) = match_preferences(&person.preferences, severity, at, &person.timezone) {
        if !channels.is_empty() {
            return channels;
        }
    }

    if let Some(tid) = team_context {
        if let Some(team) = teams.get(tid) {
            if let Some(channels) =
                match_preferences(&team.preferences, severity, at, &person.timezone)
            {
                if !channels.is_empty() {
                    return channels;
                }
            }
        }
    }

    let mut member_teams: Vec<&TeamDef> = teams
        .values()
        .filter(|t| t.members.iter().any(|m| m == &person.id))
        .collect();
    member_teams.sort_by(|a, b| a.id.cmp(&b.id));
    for team in member_teams {
        if let Some(channels) = match_preferences(&team.preferences, severity, at, &person.timezone)
        {
            if !channels.is_empty() {
                return channels;
            }
        }
    }

    vec![Channel::Ui]
}

/// Dopasowuje runbooki MANUAL dla danego kontekstu zdarzenia i zbioru dopasowanych reguł.
pub fn match_manual_runbooks<'a>(
    manual_runbooks: &'a [CompiledManualRunbook],
    matched_rule_ids: &[String],
    ctx: &Value,
) -> Vec<&'a CompiledManualRunbook> {
    manual_runbooks
        .iter()
        .filter(|rb| {
            if rb.def.kind != RunbookKind::Manual {
                return false;
            }
            let by_rule = !rb.def.rule_ids.is_empty()
                && rb
                    .def
                    .rule_ids
                    .iter()
                    .any(|rid| matched_rule_ids.contains(rid));
            let by_cond = rb
                .compiled
                .as_ref()
                .is_some_and(|compiled| compiled.evaluate(ctx));

            match (!rb.def.rule_ids.is_empty(), rb.compiled.is_some()) {
                (true, true) => by_rule || by_cond,
                (true, false) => by_rule,
                (false, true) => by_cond,
                (false, false) => false,
            }
        })
        .collect()
}

/// Renderuje tytuł i treść dla wszystkich kanałów z łańcucha fallbacku (spec `notification-delivery`).
/// - Email i UI zawierają pełną treść z prezentacji reguły oraz pełną wyrenderowaną treść dopasowanych runbooków MANUAL.
/// - SMS zawiera skróconą treść i link do szczegółów alertu w UI, BEZ pełnej treści runbooka MANUAL.
/// - Phone zawiera zwięzły skrypt głosowy.
pub fn render_for_channels(
    rule: &RuleDef,
    alert: &AlertChanged,
    ctx: &Value,
    manual_runbooks: &[&CompiledManualRunbook],
    chain: &[Channel],
    public_ui_url: &str,
) -> BTreeMap<Channel, Rendered> {
    let link = format!(
        "{}/alerts/{}",
        public_ui_url.trim_end_matches('/'),
        alert.alert_id
    );

    let title = rule
        .presentation
        .title
        .as_deref()
        .filter(|t| !t.trim().is_empty())
        .and_then(|tpl| condition::render_template(tpl, ctx).ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| alert.event.title.clone());

    let base_body = rule
        .presentation
        .body
        .as_deref()
        .filter(|b| !b.trim().is_empty())
        .and_then(|tpl| condition::render_template(tpl, ctx).ok())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| alert.event.title.clone());

    let mut full_body_with_runbooks = base_body.clone();
    for rb in manual_runbooks {
        if let Some(rb_body_tpl) = &rb.def.body {
            let rendered_rb = condition::render_template(rb_body_tpl, ctx)
                .unwrap_or_else(|_| rb_body_tpl.clone());
            full_body_with_runbooks.push_str("\n\n---\nRunbook: ");
            full_body_with_runbooks.push_str(&rb.def.title);
            full_body_with_runbooks.push('\n');
            full_body_with_runbooks.push_str(rendered_rb.trim());
        }
    }

    let short_body = if base_body.chars().count() > 120 {
        let truncated: String = base_body.chars().take(117).collect();
        format!("{truncated}...")
    } else {
        base_body.clone()
    };
    let sms_body = format!("{title} - {short_body} ({link})");

    let mut rendered = BTreeMap::new();
    for &ch in chain {
        let item = match ch {
            Channel::Sms => Rendered {
                title: title.clone(),
                body: sms_body.clone(),
                link: Some(link.clone()),
            },
            Channel::Email => Rendered {
                title: title.clone(),
                body: full_body_with_runbooks.clone(),
                link: Some(link.clone()),
            },
            Channel::Ui => Rendered {
                title: title.clone(),
                body: full_body_with_runbooks.clone(),
                link: None,
            },
            Channel::Phone => Rendered {
                title: title.clone(),
                body: format!("{title}: {short_body}"),
                link: None,
            },
        };
        rendered.insert(ch, item);
    }
    rendered
}

/// Główna funkcja routingu: dla `AlertChanged` (`Opened` lub `Reopened`) zwraca zdeduplikowane
/// zlecenia `DeliveryRequest` dla wszystkich dopasowanych reguł i ich odbiorców.
pub fn route_alert(
    snapshot: &RouterSnapshot,
    alert: &AlertChanged,
    public_ui_url: &str,
) -> Vec<DeliveryRequest> {
    if !matches!(alert.transition, Transition::Opened | Transition::Reopened) {
        return vec![];
    }

    let ctx = condition::event_context(&alert.event);

    let matched_rules: Vec<&CompiledRule> = snapshot
        .rules
        .iter()
        .filter(|r| r.def.enabled && r.compiled.evaluate(&ctx))
        .collect();

    if matched_rules.is_empty() {
        return vec![];
    }

    let matched_rule_ids: Vec<String> = matched_rules.iter().map(|r| r.def.id.clone()).collect();

    let matched_manual_rbs =
        match_manual_runbooks(&snapshot.manual_runbooks, &matched_rule_ids, &ctx);

    let mut requests_by_key: HashMap<String, usize> = HashMap::new();
    let mut requests: Vec<DeliveryRequest> = Vec::new();

    for rule in matched_rules {
        let effective_severity = rule.def.severity_override.unwrap_or(alert.severity);

        let mut expanded_recipients: Vec<(String, Option<String>)> = Vec::new();
        for recipient in &rule.def.recipients {
            match recipient {
                Recipient::Person(person_id) => {
                    expanded_recipients.push((person_id.clone(), rule.def.team.clone()));
                }
                Recipient::Team(team_id) => {
                    if let Some(team) = snapshot.teams.get(team_id) {
                        for member_id in &team.members {
                            expanded_recipients.push((member_id.clone(), Some(team_id.clone())));
                        }
                    } else {
                        tracing::warn!(
                            rule_id = %rule.def.id,
                            team_id = %team_id,
                            "rule references unknown team"
                        );
                    }
                }
            }
        }

        for (person_id, team_ctx) in expanded_recipients {
            let person = match snapshot.people.get(&person_id) {
                Some(p) => p,
                None => {
                    tracing::warn!(
                        rule_id = %rule.def.id,
                        person_id = %person_id,
                        "rule references unknown person, using default UI profile"
                    );
                    continue;
                }
            };

            let primary_channels = resolve_person_channels(
                person,
                team_ctx.as_deref(),
                &snapshot.teams,
                effective_severity,
                alert.at,
            );

            for primary_channel in primary_channels {
                let dedup_key = contracts::keys::delivery(
                    alert.alert_id,
                    alert.episode,
                    &person.id,
                    primary_channel,
                );

                let chain = default_fallback_chain(primary_channel, &person.fallback);
                let rendered = render_for_channels(
                    &rule.def,
                    alert,
                    &ctx,
                    &matched_manual_rbs,
                    &chain,
                    public_ui_url,
                );

                if let Some(&existing_idx) = requests_by_key.get(&dedup_key) {
                    if severity_rank(effective_severity)
                        > severity_rank(requests[existing_idx].severity)
                    {
                        requests[existing_idx].severity = effective_severity;
                        requests[existing_idx].origin = DeliveryOrigin::Rule {
                            rule_id: rule.def.id.clone(),
                        };
                        requests[existing_idx].rendered = rendered;
                    }
                    continue;
                }

                let req = DeliveryRequest {
                    delivery_id: Uuid::now_v7(),
                    alert_id: alert.alert_id,
                    episode: alert.episode,
                    person_id: person.id.clone(),
                    severity: effective_severity,
                    origin: DeliveryOrigin::Rule {
                        rule_id: rule.def.id.clone(),
                    },
                    chain,
                    chain_index: 0,
                    contacts: person.contacts.clone(),
                    rendered,
                    received_at: alert.event.received_at,
                    attempt: 0,
                    not_before: None,
                    last_error: None,
                };

                let idx = requests.len();
                requests.push(req);
                requests_by_key.insert(dedup_key, idx);
            }
        }
    }

    requests
}
