//! loadgen – logika generatora ruchu (właściciel: P1, design D14).
//!
//! Ten plik nie wykonuje żadnych wysyłek: konfiguracja z env, wybór kanału
//! (HTTP / Kafka), mieszanka schematów i sterowanie odsetkiem duplikatów.
//! Wysyłki są w `main.rs`.
//!
//! Duplikat = payload bez zmian (ten sam fingerprint co poprzedni bazowy).
//! Unikat = zmienione pole fingerprintu z sufiksem `-<n>` (dla `trade.rejected.v2`: `tradeId = T-<n>`).

use std::{
    collections::HashSet,
    fmt::Display,
    str::FromStr,
    time::Duration,
};

use anyhow::bail;
use serde_json::Value;

/// Fixtures z `crates/contracts` (koperty `{format, schema, payload}`).
const WORKFLOW_FIXTURE: &str = include_str!("../../contracts/fixtures/events/workflow_failed.json");
const ALERTMANAGER_FIXTURE: &str = include_str!("../../contracts/fixtures/events/alertmanager_high_error_rate.json");
const TRADE_FIXTURE: &str = include_str!("../../contracts/fixtures/events/trade_rejected.json");

/// Tryb wysyłki z `MODE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Http,
    Kafka,
    Both,
}

impl FromStr for Mode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "http" => Ok(Mode::Http),
            "kafka" => Ok(Mode::Kafka),
            "both" => Ok(Mode::Both),
            other => Err(format!("nieznany tryb {other:?}, dozwolone: http|kafka|both")),
        }
    }
}

/// Kanał pojedynczego zdarzenia.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Http,
    Kafka,
}

/// Kanał dla i-tego zdarzenia. `both` naprzemiennie: parzyste HTTP, nieparzyste Kafka.
pub fn channel_for(mode: Mode, seq: u64) -> Channel {
    match mode {
        Mode::Http => Channel::Http,
        Mode::Kafka => Channel::Kafka,
        Mode::Both if seq.is_multiple_of(2) => Channel::Http,
        Mode::Both => Channel::Kafka,
    }
}

/// Schematy z fixtures. Kafka obsługuje tylko `Trade` (jedyne źródło w seedzie to `trades`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Schema {
    Workflow = 0,
    Alertmanager = 1,
    Trade = 2,
}

impl Schema {
    /// Kolejność rotacji dla HTTP.
    const HTTP_ROTATION: [Schema; 3] = [Schema::Workflow, Schema::Alertmanager, Schema::Trade];

    pub fn name(self) -> &'static str {
        match self {
            Schema::Workflow => "workflow.v1",
            Schema::Alertmanager => "alertmanager",
            Schema::Trade => "trade.rejected.v2",
        }
    }

    /// Ścieżka do pola fingerprintu w koperty fixtures.
    fn fingerprint_path(self) -> &'static [&'static str] {
        match self {
            Schema::Workflow => &["payload", "workflowName"],
            Schema::Alertmanager => &["payload", "alertname"],
            Schema::Trade => &["payload", "payload", "tradeId"],
        }
    }
}

/// Zdarzenie gotowe do wysłania.
#[derive(Debug, Clone)]
pub struct Event {
    pub channel: Channel,
    pub schema: Schema,
    /// HTTP: cała koperta `{format, schema, payload}`. Kafka: surowy payload `trade.rejected.v2`.
    pub body: Value,
    /// Identyfikator fingerprintu (`schema|wartość pola`) – do policzenia unikalnych.
    pub identity: String,
    /// `true` gdy payload jest bazowy, niezmieniony (powtórka fingerprintu).
    pub duplicate: bool,
}

/// Prosty generator liczb pseudolosowych (xorshift64) – bez zależności `rand`.
#[derive(Debug, Clone)]
pub struct XorShift(u64);

impl XorShift {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    /// Liczba z [0, 1).
    pub fn next_f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Generator zdarzeń: rotacja schematów (HTTP), losowanie duplikatów, unikalne sufiksy.
pub struct Generator {
    rng: XorShift,
    dup_ratio: f64,
    /// Sufiks unikatów. Startuje od czasu uruchomienia (ms), żeby kolejne przebiegi
    /// nie generowały fingerprintów zgodnych z poprzednim przebiegiem.
    next_suffix: u64,
    http_count: u64,
    bases: [Value; 3],
    base_identities: [String; 3],
    /// Wszystkie wygenerowane identyfikatory (do raportu „unikalne fingerprinty”).
    identities: HashSet<String>,
    generated: u64,
    duplicates: u64,
}

impl Generator {
    /// `seed` – losowość duplikatów; `suffix_start` – początek sufiksów unikatów.
    pub fn new(dup_ratio: f64, seed: u64, suffix_start: u64) -> Self {
        let parse = |s: &str| -> Value { serde_json::from_str(s).expect("fixture z contracts jest poprawnym JSON") };
        let bases = [parse(WORKFLOW_FIXTURE), parse(ALERTMANAGER_FIXTURE), parse(TRADE_FIXTURE)];
        let base_identities = [Schema::Workflow, Schema::Alertmanager, Schema::Trade]
            .map(|s| identity(s, &bases[s as usize]));
        Self {
            rng: XorShift::new(seed),
            dup_ratio,
            next_suffix: suffix_start,
            http_count: 0,
            bases,
            base_identities,
            identities: HashSet::new(),
            generated: 0,
            duplicates: 0,
        }
    }

    /// Generuje następne zdarzenie dla podanego kanału.
    pub fn next_event(&mut self, channel: Channel) -> Event {
        let schema = match channel {
            Channel::Kafka => Schema::Trade,
            Channel::Http => {
                let s = Schema::HTTP_ROTATION[(self.http_count % 3) as usize];
                self.http_count += 1;
                s
            }
        };
        let duplicate = self.rng.next_f64() < self.dup_ratio;
        let mut envelope = self.bases[schema as usize].clone();
        let identity = if duplicate {
            self.duplicates += 1;
            self.base_identities[schema as usize].clone()
        } else {
            let n = self.next_suffix;
            self.next_suffix += 1;
            set_fingerprint(schema, &mut envelope, n);
            identity(schema, &envelope)
        };
        self.generated += 1;
        self.identities.insert(identity.clone());

        let body = match channel {
            Channel::Http => envelope,
            Channel::Kafka => envelope.get("payload").cloned().unwrap_or(Value::Null),
        };
        Event { channel, schema, body, identity, duplicate }
    }

    pub fn generated(&self) -> u64 {
        self.generated
    }

    pub fn duplicates(&self) -> u64 {
        self.duplicates
    }

    pub fn unique_identities(&self) -> usize {
        self.identities.len()
    }
}

/// Identyfikator fingerprintu z koperty (`schema|wartość pola`).
fn identity(schema: Schema, envelope: &Value) -> String {
    let mut cur = envelope;
    for key in schema.fingerprint_path() {
        cur = &cur[*key];
    }
    format!("{}|{}", schema.name(), cur.as_str().unwrap_or_default())
}

/// Ustawia pole fingerprintu na wartość unikalną dla `n`.
fn set_fingerprint(schema: Schema, envelope: &mut Value, n: u64) {
    let path = schema.fingerprint_path();
    let (last, parents) = path.split_last().expect("ścieżka fingerprintu niepusta");
    let mut cur = envelope;
    for key in parents {
        cur = &mut cur[*key];
    }
    let value = match schema {
        Schema::Trade => format!("T-{n}"),
        _ => format!("{}-{n}", cur[*last].as_str().unwrap_or_default()),
    };
    cur[*last] = Value::String(value);
}

/// Konfiguracja przebiegu z env (`RATE`, `DURATION_SECS`, `DUP_RATIO`, `MODE`, `INGEST_URL`, `SOURCE_TOPIC`).
#[derive(Debug, Clone)]
pub struct Settings {
    pub rate: u64,
    pub duration: Duration,
    pub dup_ratio: f64,
    pub mode: Mode,
    pub ingest_url: String,
    pub source_topic: String,
}

impl Settings {
    pub fn from_env() -> anyhow::Result<Self> {
        let settings = Self {
            rate: env_or("RATE", "100")?,
            duration: Duration::from_secs(env_or("DURATION_SECS", "10")?),
            dup_ratio: env_or("DUP_RATIO", "0.5")?,
            mode: env_or("MODE", "both")?,
            ingest_url: env_or("INGEST_URL", "http://localhost:8081")?,
            source_topic: env_or("SOURCE_TOPIC", "trades")?,
        };
        if settings.rate == 0 {
            bail!("RATE musi być > 0");
        }
        if !(0.0..=1.0).contains(&settings.dup_ratio) {
            bail!("DUP_RATIO musi być w zakresie 0.0–1.0, jest {}", settings.dup_ratio);
        }
        Ok(settings)
    }
}

fn env_or<T>(name: &str, default: &str) -> anyhow::Result<T>
where
    T: FromStr,
    T::Err: Display,
{
    let raw = std::env::var(name).unwrap_or_else(|_| default.to_string());
    raw.parse::<T>().map_err(|e| anyhow::anyhow!("{name}={raw:?}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAPPINGS: [&str; 3] = [
        include_str!("../../../config/mappings/workflow.v1.yaml"),
        include_str!("../../../config/mappings/alertmanager.yaml"),
        include_str!("../../../config/mappings/trade.rejected.v2.yaml"),
    ];

    fn has_fingerprint_line(mapping: &str, path: &str) -> bool {
        mapping.lines().any(|l| l.trim() == format!("- {path}"))
    }

    #[test]
    fn unique_mode_gives_unique_identities() {
        let mut gen = Generator::new(0.0, 42, 0);
        let events: Vec<Event> = (0..300).map(|_| gen.next_event(Channel::Http)).collect();
        assert!(events.iter().all(|e| !e.duplicate));
        assert_eq!(gen.unique_identities(), 300);
        assert_eq!(events.iter().map(|e| e.identity.clone()).collect::<HashSet<_>>().len(), 300);
        assert_eq!(gen.duplicates(), 0);
        assert_eq!(gen.generated(), 300);
    }

    #[test]
    fn full_duplicate_mode_repeats_base_identity_per_schema() {
        let mut gen = Generator::new(1.0, 7, 0);
        let events: Vec<Event> = (0..300).map(|_| gen.next_event(Channel::Http)).collect();
        assert!(events.iter().all(|e| e.duplicate));
        assert_eq!(gen.duplicates(), 300);
        assert_eq!(gen.unique_identities(), 3, "po jednym fingerprincie na schemat");
        for e in &events {
            let expected = match e.schema {
                Schema::Workflow => "workflow.v1|order-settlement",
                Schema::Alertmanager => "alertmanager|HighErrorRate",
                Schema::Trade => "trade.rejected.v2|T-998877",
            };
            assert_eq!(e.identity, expected);
        }
    }

    #[test]
    fn half_duplicate_ratio_is_close_to_half() {
        let mut gen = Generator::new(0.5, 12345, 0);
        for _ in 0..2000 {
            gen.next_event(Channel::Http);
        }
        let ratio = gen.duplicates() as f64 / gen.generated() as f64;
        assert!((0.4..=0.6).contains(&ratio), "odsetek duplikatów {ratio}");
    }

    #[test]
    fn http_rotates_all_three_schemas() {
        let mut gen = Generator::new(0.5, 1, 0);
        let schemas: HashSet<&'static str> = (0..300)
            .map(|_| gen.next_event(Channel::Http))
            .map(|e| e.schema.name())
            .collect();
        assert_eq!(
            schemas,
            HashSet::from(["workflow.v1", "alertmanager", "trade.rejected.v2"])
        );
    }

    #[test]
    fn kafka_channel_sends_only_trade_raw_payload() {
        let mut gen = Generator::new(0.5, 1, 0);
        for _ in 0..300 {
            let e = gen.next_event(Channel::Kafka);
            assert_eq!(e.channel, Channel::Kafka);
            assert_eq!(e.schema, Schema::Trade);
            // Surowy payload: bez koperty `format` / `schema`.
            assert!(e.body.get("format").is_none());
            assert!(e.body.get("schema").is_none());
            assert!(e.body.pointer("/payload/tradeId").is_some());
            assert!(e.body.get("meta").is_some());
        }
    }

    #[test]
    fn http_mix_has_all_schemas_in_300_events() {
        let mut gen = Generator::new(0.5, 3, 0);
        let mut seen = HashSet::new();
        for _ in 0..300 {
            let e = gen.next_event(Channel::Http);
            assert_eq!(e.body["schema"], e.schema.name());
            seen.insert(e.schema.name());
        }
        assert_eq!(seen.len(), 3);
    }

    #[test]
    fn unique_events_change_fingerprint_field_per_mapping() {
        // Pola fingerprintu muszą zgadzać się z mappingami z config/.
        assert!(has_fingerprint_line(MAPPINGS[0], "$.workflowName"));
        assert!(has_fingerprint_line(MAPPINGS[1], "$.alertname"));
        assert!(has_fingerprint_line(MAPPINGS[2], "$.payload.tradeId"));

        let mut gen = Generator::new(0.0, 9, 5000);
        // HTTP: pierwsze trzy zdarzenia to workflow, alertmanager, trade (kolejność rotacji).
        let wf = gen.next_event(Channel::Http);
        assert_eq!(wf.identity, "workflow.v1|order-settlement-5000");
        assert_eq!(wf.body["payload"]["workflowName"], "order-settlement-5000");
        assert_eq!(wf.body["payload"]["type"], "WORKFLOW_FAILED", "reszta payloadu bez zmian");

        let am = gen.next_event(Channel::Http);
        assert_eq!(am.identity, "alertmanager|HighErrorRate-5001");
        assert_eq!(am.body["payload"]["alertname"], "HighErrorRate-5001");

        let tr = gen.next_event(Channel::Http);
        assert_eq!(tr.identity, "trade.rejected.v2|T-5002");
        assert_eq!(tr.body["payload"]["payload"]["tradeId"], "T-5002");
        assert_eq!(tr.body["payload"]["payload"]["desk"], "rates", "reszta payloadu bez zmian");

        // Kafka: surowy payload, tradeId w tym samym miejscu co w kopercie.
        let k = gen.next_event(Channel::Kafka);
        assert_eq!(k.identity, "trade.rejected.v2|T-5003");
        assert_eq!(k.body["payload"]["tradeId"], "T-5003");
    }

    #[test]
    fn http_envelope_is_valid_json_with_format_schema_payload() {
        let mut gen = Generator::new(0.5, 11, 0);
        for _ in 0..30 {
            let e = gen.next_event(Channel::Http);
            let text = serde_json::to_string(&e.body).expect("serializacja");
            let parsed: Value = serde_json::from_str(&text).expect("poprawny JSON");
            assert_eq!(parsed["format"], "json");
            assert_eq!(parsed["schema"], e.schema.name());
            assert!(parsed["payload"].is_object());
        }
    }

    #[test]
    fn channel_for_both_alternates() {
        let kinds: Vec<Channel> = (0..6).map(|s| channel_for(Mode::Both, s)).collect();
        assert_eq!(
            kinds,
            vec![Channel::Http, Channel::Kafka, Channel::Http, Channel::Kafka, Channel::Http, Channel::Kafka]
        );
        assert!((0..10).all(|s| channel_for(Mode::Http, s) == Channel::Http));
        assert!((0..10).all(|s| channel_for(Mode::Kafka, s) == Channel::Kafka));
    }

    #[test]
    fn mode_parsing() {
        assert_eq!("http".parse::<Mode>(), Ok(Mode::Http));
        assert_eq!("kafka".parse::<Mode>(), Ok(Mode::Kafka));
        assert_eq!("both".parse::<Mode>(), Ok(Mode::Both));
        assert!("HTTP".parse::<Mode>().is_err());
    }

    /// Jeden test na całe env – zmienne procesu są globalne, więc nie mogą się ścigać.
    #[test]
    fn settings_from_env_defaults_and_parsing() {
        const VARS: [&str; 6] = ["RATE", "DURATION_SECS", "DUP_RATIO", "MODE", "INGEST_URL", "SOURCE_TOPIC"];
        let clear = || VARS.iter().for_each(|v| std::env::remove_var(v));
        clear();

        let s = Settings::from_env().expect("domyślne env");
        assert_eq!(s.rate, 100);
        assert_eq!(s.duration, Duration::from_secs(10));
        assert_eq!(s.dup_ratio, 0.5);
        assert_eq!(s.mode, Mode::Both);
        assert_eq!(s.ingest_url, "http://localhost:8081");
        assert_eq!(s.source_topic, "trades");

        std::env::set_var("MODE", "kafka");
        std::env::set_var("RATE", "250");
        std::env::set_var("DURATION_SECS", "3");
        std::env::set_var("DUP_RATIO", "0.25");
        let s = Settings::from_env().expect("poprawne env");
        assert_eq!(s.mode, Mode::Kafka);
        assert_eq!(s.rate, 250);
        assert_eq!(s.duration, Duration::from_secs(3));
        assert_eq!(s.dup_ratio, 0.25);

        std::env::set_var("MODE", "udp");
        assert!(Settings::from_env().is_err(), "nieznany MODE");

        std::env::set_var("MODE", "http");
        std::env::set_var("RATE", "0");
        assert!(Settings::from_env().is_err(), "RATE=0");

        std::env::set_var("RATE", "10");
        std::env::set_var("DUP_RATIO", "1.5");
        assert!(Settings::from_env().is_err(), "DUP_RATIO poza zakresem");

        clear();
    }
}
