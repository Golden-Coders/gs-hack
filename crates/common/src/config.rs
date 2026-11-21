//! Konfiguracja z env. Wartości domyślne pasują do uruchomienia lokalnego
//! (`docker compose up redpanda postgres mailpit` + `cargo run -p <bin>`).

#[derive(Debug, Clone)]
pub struct Config {
    /// `DATABASE_URL`
    pub database_url: String,
    /// `KAFKA_BROKERS`
    pub kafka_brokers: String,
    /// `CONFIG_DIR` – katalog z plikami konfiguracji (seed + hot reload).
    pub config_dir: String,
    /// `PUBLIC_UI_URL` – baza linków do alertów (np. w SMS).
    pub public_ui_url: String,
    /// `SMTP_HOST`, `SMTP_PORT`
    pub smtp_host: String,
    pub smtp_port: u16,
    /// `ACTION_HOST_ALLOWLIST` – hosty dozwolone dla akcji HTTP (po przecinku).
    pub action_host_allowlist: Vec<String>,
    /// `API_TOKEN` – statyczny token API.
    pub api_token: String,
}

fn env_or(name: &str, default: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| default.to_string())
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            database_url: env_or(
                "DATABASE_URL",
                "postgres://alerting:alerting@localhost:5432/alerting",
            ),
            kafka_brokers: env_or("KAFKA_BROKERS", "localhost:19092"),
            config_dir: env_or("CONFIG_DIR", "config"),
            public_ui_url: env_or("PUBLIC_UI_URL", "http://localhost:5173"),
            smtp_host: env_or("SMTP_HOST", "localhost"),
            smtp_port: env_or("SMTP_PORT", "1025").parse().unwrap_or(1025),
            action_host_allowlist: env_or("ACTION_HOST_ALLOWLIST", "mock-targets,localhost")
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            api_token: env_or("API_TOKEN", "dev-token"),
        }
    }

    /// Port HTTP danej binarki: zmienna `<PREFIX>_PORT` albo wartość domyślna.
    pub fn port(prefix: &str, default: u16) -> u16 {
        std::env::var(format!("{prefix}_PORT"))
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(default)
    }
}
