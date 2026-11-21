//! Interfejs adaptera kanału (`ChannelAdapter`) i implementacje dla MVP:
//! `UiAdapter`, `EmailAdapter`, `SmsMockAdapter`, `PhoneMockAdapter` (design D8, sekcja 9.1).

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use contracts::{Channel, DeliveryRequest, Rendered, UiEvent, UiEventKind};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;

use crate::store::{BoxFuture, DeliveryBackend};

/// Błąd wysyłki przez adapter kanału (design D8).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SendError {
    /// Błąd tymczasowy (np. niedostępna bramka SMS, błąd połączenia SMTP) -> retry z backoffem.
    #[error("{0}")]
    Transient(String),
    /// Błąd trwały (np. brak lub niepoprawny adres e-mail/numer telefonu) -> natychmiastowy fallback.
    #[error("{0}")]
    Permanent(String),
}

impl SendError {
    pub fn message(&self) -> &str {
        match self {
            SendError::Transient(msg) | SendError::Permanent(msg) => msg,
        }
    }
}

/// Wspólny interfejs adapterów kanałów dostarczenia (design D8).
///
/// Dodanie realnego dostawcy (np. Twilio zamiast `SmsMockAdapter`) wymaga
/// wyłącznie nowej implementacji tego traitu i rejestracji w `SenderEngine`.
pub trait ChannelAdapter: Send + Sync {
    /// Rodzaj obsługiwanego kanału.
    fn kind(&self) -> Channel;

    /// Wysyła powiadomienie. Zwraca identyfikator referencyjny dostawcy lub błąd `SendError`.
    fn send<'a>(
        &'a self,
        req: &'a DeliveryRequest,
        rendered: &'a Rendered,
        to: Option<&'a str>,
    ) -> BoxFuture<'a, Result<String, SendError>>;
}

/// Formatuje treść SMS z dołączonym linkiem do alertu w UI (zgodnie z `openapi.yaml` i specyfikacją).
pub fn format_sms_body(rendered: &Rendered) -> String {
    match rendered.link.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(link) => {
            if rendered.body.trim().is_empty() {
                format!("{} {link}", rendered.title)
            } else if rendered.body.contains(link) {
                rendered.body.clone()
            } else {
                format!("{} {link}", rendered.body)
            }
        }
        None => {
            if rendered.body.trim().is_empty() {
                rendered.title.clone()
            } else {
                rendered.body.clone()
            }
        }
    }
}

/// Formatuje komunikat głosowy dla symulowanego połączenia telefonicznego.
pub fn format_phone_body(rendered: &Rendered) -> String {
    if rendered.body.trim().is_empty() {
        format!("Call: {}", rendered.title)
    } else {
        rendered.body.clone()
    }
}

/// Formatuje treść wiadomości e-mail (dołączając link na końcu, jeśli nie występuje w treści).
pub fn format_email_body(rendered: &Rendered) -> String {
    match rendered.link.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
        Some(link) if !rendered.body.contains(link) => {
            if rendered.body.trim().is_empty() {
                link.to_string()
            } else {
                format!("{}\n\n{link}", rendered.body)
            }
        }
        _ => rendered.body.clone(),
    }
}

// ============================================================================
// UiAdapter (Channel::Ui)
// ============================================================================

pub struct UiAdapter {
    backend: Arc<dyn DeliveryBackend>,
}

impl UiAdapter {
    pub fn new(backend: Arc<dyn DeliveryBackend>) -> Self {
        Self { backend }
    }
}

impl ChannelAdapter for UiAdapter {
    fn kind(&self) -> Channel {
        Channel::Ui
    }

    fn send<'a>(
        &'a self,
        _req: &'a DeliveryRequest,
        _rendered: &'a Rendered,
        _to: Option<&'a str>,
    ) -> BoxFuture<'a, Result<String, SendError>> {
        Box::pin(async move {
            let available = self
                .backend
                .is_channel_available(Channel::Ui)
                .await
                .map_err(|e| SendError::Transient(format!("checking ui channel availability: {e}")))?;
            if !available {
                return Err(SendError::Transient("ui channel unavailable".to_string()));
            }
            Ok("ui:delivered".to_string())
        })
    }
}

// ============================================================================
// SmsMockAdapter (Channel::Sms) & PhoneMockAdapter (Channel::Phone)
// ============================================================================

pub struct SmsMockAdapter {
    backend: Arc<dyn DeliveryBackend>,
}

impl SmsMockAdapter {
    pub fn new(backend: Arc<dyn DeliveryBackend>) -> Self {
        Self { backend }
    }
}

impl ChannelAdapter for SmsMockAdapter {
    fn kind(&self) -> Channel {
        Channel::Sms
    }

    fn send<'a>(
        &'a self,
        req: &'a DeliveryRequest,
        rendered: &'a Rendered,
        to: Option<&'a str>,
    ) -> BoxFuture<'a, Result<String, SendError>> {
        Box::pin(async move {
            let available = self
                .backend
                .is_channel_available(Channel::Sms)
                .await
                .map_err(|e| SendError::Transient(format!("checking sms channel availability: {e}")))?;
            if !available {
                return Err(SendError::Transient("sms gateway unavailable".to_string()));
            }

            let recipient = to
                .or_else(|| req.contacts.get(&Channel::Sms).map(String::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| SendError::Permanent("missing phone number".to_string()))?;

            let body = format_sms_body(rendered);
            let msg = self
                .backend
                .insert_mock_outbox(
                    Channel::Sms,
                    recipient,
                    &req.person_id,
                    &rendered.title,
                    &body,
                    Some(req.alert_id),
                )
                .await
                .map_err(|e| SendError::Transient(format!("saving sms to mock_outbox: {e}")))?;

            if let Ok(payload) = serde_json::to_value(&msg) {
                let event = UiEvent {
                    alert_id: Some(req.alert_id),
                    kind: UiEventKind::MockOutbox,
                    payload,
                    at: Utc::now(),
                };
                let _ = self.backend.publish_ui_event(&event).await;
            }

            Ok(format!("mock_outbox:{}", msg.id))
        })
    }
}

pub struct PhoneMockAdapter {
    backend: Arc<dyn DeliveryBackend>,
}

impl PhoneMockAdapter {
    pub fn new(backend: Arc<dyn DeliveryBackend>) -> Self {
        Self { backend }
    }
}

impl ChannelAdapter for PhoneMockAdapter {
    fn kind(&self) -> Channel {
        Channel::Phone
    }

    fn send<'a>(
        &'a self,
        req: &'a DeliveryRequest,
        rendered: &'a Rendered,
        to: Option<&'a str>,
    ) -> BoxFuture<'a, Result<String, SendError>> {
        Box::pin(async move {
            let available = self
                .backend
                .is_channel_available(Channel::Phone)
                .await
                .map_err(|e| SendError::Transient(format!("checking phone channel availability: {e}")))?;
            if !available {
                return Err(SendError::Transient("phone gateway unavailable".to_string()));
            }

            let recipient = to
                .or_else(|| req.contacts.get(&Channel::Phone).map(String::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| SendError::Permanent("missing phone number".to_string()))?;

            let body = format_phone_body(rendered);
            let msg = self
                .backend
                .insert_mock_outbox(
                    Channel::Phone,
                    recipient,
                    &req.person_id,
                    &rendered.title,
                    &body,
                    Some(req.alert_id),
                )
                .await
                .map_err(|e| SendError::Transient(format!("saving phone call to mock_outbox: {e}")))?;

            if let Ok(payload) = serde_json::to_value(&msg) {
                let event = UiEvent {
                    alert_id: Some(req.alert_id),
                    kind: UiEventKind::MockOutbox,
                    payload,
                    at: Utc::now(),
                };
                let _ = self.backend.publish_ui_event(&event).await;
            }

            Ok(format!("mock_outbox:{}", msg.id))
        })
    }
}

// ============================================================================
// EmailAdapter (Channel::Email) -> SMTP (Mailpit)
// ============================================================================

pub type BoxedAsyncRead = Pin<Box<dyn AsyncRead + Send>>;
pub type BoxedAsyncWrite = Pin<Box<dyn AsyncWrite + Send>>;

/// Fabryka połączeń strumieniowych SMTP (domyślnie `TcpStream::connect` do Mailpit; w testach może używać `tokio::io::duplex`).
pub trait SmtpConnector: Send + Sync {
    fn connect(&self) -> BoxFuture<'_, Result<(BoxedAsyncRead, BoxedAsyncWrite), SendError>>;
}

pub struct TcpSmtpConnector {
    pub host: String,
    pub port: u16,
}

impl SmtpConnector for TcpSmtpConnector {
    fn connect(&self) -> BoxFuture<'_, Result<(BoxedAsyncRead, BoxedAsyncWrite), SendError>> {
        let addr = format!("{}:{}", self.host, self.port);
        Box::pin(async move {
            let stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr))
                .await
                .map_err(|_| SendError::Transient(format!("smtp connect timeout to {addr}")))?
                .map_err(|e| SendError::Transient(format!("smtp connect failed to {addr}: {e}")))?;
            let (r, w) = stream.into_split();
            Ok((Box::pin(r) as BoxedAsyncRead, Box::pin(w) as BoxedAsyncWrite))
        })
    }
}

const DEFAULT_SMTP_POOL_MAX_IDLE: usize = 16;

struct PooledSmtpSession {
    lines: tokio::io::Lines<BufReader<BoxedAsyncRead>>,
    writer: BoxedAsyncWrite,
}

impl PooledSmtpSession {
    async fn open(connector: &dyn SmtpConnector) -> Result<Self, SendError> {
        let (reader, mut writer) = connector.connect().await?;
        let mut lines = BufReader::new(reader).lines();

        let (code, greeting) = read_smtp_response(&mut lines).await?;
        if code != 220 {
            return Err(SendError::Transient(format!("unexpected smtp greeting: {greeting}")));
        }

        writer
            .write_all(b"EHLO alerting.local\r\n")
            .await
            .map_err(|e| SendError::Transient(format!("smtp write EHLO: {e}")))?;
        let (code, resp) = read_smtp_response(&mut lines).await?;
        if code != 250 {
            return Err(SendError::Transient(format!("smtp EHLO rejected: {resp}")));
        }

        Ok(Self { lines, writer })
    }

    async fn send_message(&mut self, from: &str, to: &str, subject: &str, body: &str) -> Result<String, SendError> {
        self.writer
            .write_all(format!("MAIL FROM:<{from}>\r\n").as_bytes())
            .await
            .map_err(|e| SendError::Transient(format!("smtp write MAIL FROM: {e}")))?;
        let (code, resp) = read_smtp_response(&mut self.lines).await?;
        if code != 250 {
            return Err(SendError::Transient(format!("smtp MAIL FROM rejected: {resp}")));
        }

        self.writer
            .write_all(format!("RCPT TO:<{to}>\r\n").as_bytes())
            .await
            .map_err(|e| SendError::Transient(format!("smtp write RCPT TO: {e}")))?;
        let (code, resp) = read_smtp_response(&mut self.lines).await?;
        if (500..600).contains(&code) {
            return Err(SendError::Permanent(format!("smtp recipient rejected: {resp}")));
        }
        if code != 250 && code != 251 {
            return Err(SendError::Transient(format!("smtp RCPT TO failed: {resp}")));
        }

        self.writer
            .write_all(b"DATA\r\n")
            .await
            .map_err(|e| SendError::Transient(format!("smtp write DATA: {e}")))?;
        let (code, resp) = read_smtp_response(&mut self.lines).await?;
        if code != 354 {
            return Err(SendError::Transient(format!("smtp DATA rejected: {resp}")));
        }

        let clean_subject = subject.replace(['\r', '\n'], " ");
        let mut payload = format!(
            "From: Alerting Service <{from}>\r\nTo: <{to}>\r\nSubject: {clean_subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n"
        );
        for line in body.lines() {
            if line.starts_with('.') {
                payload.push('.');
            }
            payload.push_str(line);
            payload.push_str("\r\n");
        }
        payload.push_str(".\r\n");

        self.writer
            .write_all(payload.as_bytes())
            .await
            .map_err(|e| SendError::Transient(format!("smtp write body: {e}")))?;
        let (code, resp) = read_smtp_response(&mut self.lines).await?;
        if code != 250 {
            return Err(SendError::Transient(format!("smtp message rejected: {resp}")));
        }

        Ok("smtp:250".to_string())
    }

    async fn quit(mut self) {
        let _ = self.writer.write_all(b"QUIT\r\n").await;
    }
}

pub struct EmailAdapter {
    backend: Arc<dyn DeliveryBackend>,
    connector: Arc<dyn SmtpConnector>,
    from_address: String,
    pool: Arc<tokio::sync::Mutex<Vec<PooledSmtpSession>>>,
    max_idle: usize,
}

impl EmailAdapter {
    pub fn new(backend: Arc<dyn DeliveryBackend>, smtp_host: impl Into<String>, smtp_port: u16) -> Self {
        Self {
            backend,
            connector: Arc::new(TcpSmtpConnector {
                host: smtp_host.into(),
                port: smtp_port,
            }),
            from_address: "alerts@alerting.local".to_string(),
            pool: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            max_idle: DEFAULT_SMTP_POOL_MAX_IDLE,
        }
    }

    pub fn with_connector(backend: Arc<dyn DeliveryBackend>, connector: Arc<dyn SmtpConnector>) -> Self {
        Self {
            backend,
            connector,
            from_address: "alerts@alerting.local".to_string(),
            pool: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            max_idle: DEFAULT_SMTP_POOL_MAX_IDLE,
        }
    }

    async fn send_pooled(&self, recipient: &str, subject: &str, body: &str) -> Result<String, SendError> {
        // Najpierw próbujemy reużyć bezczynną sesję z puli (bez narzutu TCP + EHLO).
        let pooled = { self.pool.lock().await.pop() };
        if let Some(mut session) = pooled {
            match tokio::time::timeout(
                Duration::from_secs(10),
                session.send_message(&self.from_address, recipient, subject, body),
            )
            .await
            {
                Ok(Ok(res)) => {
                    self.return_to_pool(session).await;
                    return Ok(res);
                }
                Ok(Err(err @ SendError::Permanent(_))) => {
                    // Błąd trwały (np. odrzucony adres) nie oznacza martwego połączenia — zamykamy czysto.
                    session.quit().await;
                    return Err(err);
                }
                Ok(Err(SendError::Transient(_))) | Err(_) => {
                    // Sesja z puli mogła wygasnąć (np. restart Mailpit lub idle timeout) — ponawiamy raz na nowym połączeniu.
                    drop(session);
                }
            }
        }

        let mut session = tokio::time::timeout(Duration::from_secs(10), PooledSmtpSession::open(self.connector.as_ref()))
            .await
            .map_err(|_| SendError::Transient("smtp connect/ehlo timed out".to_string()))??;

        let res = tokio::time::timeout(
            Duration::from_secs(10),
            session.send_message(&self.from_address, recipient, subject, body),
        )
        .await
        .map_err(|_| SendError::Transient("smtp transaction timed out".to_string()))??;

        self.return_to_pool(session).await;
        Ok(res)
    }

    async fn return_to_pool(&self, session: PooledSmtpSession) {
        let mut guard = self.pool.lock().await;
        if guard.len() < self.max_idle {
            guard.push(session);
        } else {
            drop(guard);
            session.quit().await;
        }
    }
}

fn validate_email_address(addr: &str) -> Result<&str, SendError> {
    let trimmed = addr.trim();
    if trimmed.is_empty() || trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(SendError::Permanent(format!("invalid email address '{addr}'")));
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(SendError::Permanent(format!("invalid email address '{addr}'")));
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') || !domain.contains('.') {
        return Err(SendError::Permanent(format!("invalid email address '{addr}'")));
    }
    Ok(trimmed)
}

async fn read_smtp_response<R: AsyncRead + Unpin>(
    lines: &mut tokio::io::Lines<BufReader<R>>,
) -> Result<(u16, String), SendError> {
    let mut full = String::new();
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|e| SendError::Transient(format!("smtp read error: {e}")))?
            .ok_or_else(|| SendError::Transient("smtp connection closed unexpectedly".to_string()))?;

        if line.len() < 3 {
            return Err(SendError::Transient(format!("malformed smtp response: {line}")));
        }
        let code: u16 = line[..3]
            .parse()
            .map_err(|_| SendError::Transient(format!("invalid smtp status code: {line}")))?;
        full.push_str(&line);

        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok((code, full));
        }
        full.push('\n');
    }
}

impl ChannelAdapter for EmailAdapter {
    fn kind(&self) -> Channel {
        Channel::Email
    }

    fn send<'a>(
        &'a self,
        req: &'a DeliveryRequest,
        rendered: &'a Rendered,
        to: Option<&'a str>,
    ) -> BoxFuture<'a, Result<String, SendError>> {
        Box::pin(async move {
            let available = self
                .backend
                .is_channel_available(Channel::Email)
                .await
                .map_err(|e| SendError::Transient(format!("checking email channel availability: {e}")))?;
            if !available {
                return Err(SendError::Transient("email channel unavailable".to_string()));
            }

            let raw_to = to
                .or_else(|| req.contacts.get(&Channel::Email).map(String::as_str))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .ok_or_else(|| SendError::Permanent("missing email address".to_string()))?;

            let recipient = validate_email_address(raw_to)?;
            let body = format_email_body(rendered);

            self.send_pooled(recipient, &rendered.title, &body).await
        })
    }
}
