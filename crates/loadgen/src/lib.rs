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
