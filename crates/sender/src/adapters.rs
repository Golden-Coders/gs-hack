//! Interfejs adaptera kanału (`ChannelAdapter`) i implementacje dla MVP:
//! `UiAdapter`, `EmailAdapter`, `SmsMockAdapter`, `PhoneMockAdapter` (design D8, sekcja 9.1).

use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use contracts::{Channel, DeliveryRequest, Rendered, UiEvent, UiEventKind};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
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
    match rendered
        .link
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
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
    match rendered
        .link
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
    {
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
                .map_err(|e| {
                    tracing::warn!(error = %e, "checking ui channel availability failed");
                    SendError::Transient("ui channel availability check failed".to_string())
                })?;
            if !available {
                return Err(SendError::Transient("ui channel unavailable".to_string()));
            }
            Ok("ui:delivered".to_string())
        })
    }
}

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
                .map_err(|e| {
                    tracing::warn!(error = %e, "checking sms channel availability failed");
                    SendError::Transient("sms channel availability check failed".to_string())
                })?;
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
                .map_err(|e| {
                    tracing::warn!(error = %e, "saving sms to mock_outbox failed");
                    SendError::Transient("failed to persist sms notification".to_string())
                })?;

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
                .map_err(|e| {
                    tracing::warn!(error = %e, "checking phone channel availability failed");
                    SendError::Transient("phone channel availability check failed".to_string())
                })?;
            if !available {
                return Err(SendError::Transient(
                    "phone gateway unavailable".to_string(),
                ));
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
                .map_err(|e| {
                    tracing::warn!(error = %e, "saving phone call to mock_outbox failed");
                    SendError::Transient("failed to persist phone notification".to_string())
                })?;

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
                .map_err(|_| {
                    tracing::warn!(smtp_addr = %addr, "smtp connect timed out");
                    SendError::Transient("smtp connect timed out".to_string())
                })?
                .map_err(|e| {
                    tracing::warn!(smtp_addr = %addr, error = %e, "smtp connect failed");
                    SendError::Transient("smtp connect failed".to_string())
                })?;
            let (r, w) = stream.into_split();
            Ok((
                Box::pin(r) as BoxedAsyncRead,
                Box::pin(w) as BoxedAsyncWrite,
            ))
        })
    }
}

const DEFAULT_SMTP_POOL_MAX_IDLE: usize = 16;
const MAX_SMTP_LINE_BYTES: usize = 4_096;
const MAX_SMTP_CONTINUATION_LINES: usize = 64;
const MAX_SMTP_RESPONSE_BYTES: usize = 32_768;

struct PooledSmtpSession {
    reader: BufReader<BoxedAsyncRead>,
    writer: BoxedAsyncWrite,
}

impl PooledSmtpSession {
    async fn open(connector: &dyn SmtpConnector) -> Result<Self, SendError> {
        let (reader, mut writer) = connector.connect().await?;
        let mut reader = BufReader::new(reader);

        let (code, greeting) = read_smtp_response(&mut reader).await?;
        if code != 220 {
            tracing::warn!(code, response = %greeting, "unexpected smtp greeting");
            return Err(SendError::Transient(format!(
                "unexpected smtp greeting (code {code})"
            )));
        }

        writer
            .write_all(b"EHLO alerting.local\r\n")
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "smtp write EHLO failed");
                SendError::Transient("smtp write EHLO failed".to_string())
            })?;
        let (code, resp) = read_smtp_response(&mut reader).await?;
        if code != 250 {
            tracing::warn!(code, response = %resp, "smtp EHLO rejected");
            return Err(SendError::Transient(format!(
                "smtp EHLO rejected (code {code})"
            )));
        }

        Ok(Self { reader, writer })
    }

    async fn send_message(
        &mut self,
        from: &str,
        to: &str,
        subject: &str,
        body: &str,
    ) -> Result<String, SendError> {
        self.writer
            .write_all(format!("MAIL FROM:<{from}>\r\n").as_bytes())
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "smtp write MAIL FROM failed");
                SendError::Transient("smtp write MAIL FROM failed".to_string())
            })?;
        let (code, resp) = read_smtp_response(&mut self.reader).await?;
        if code != 250 {
            tracing::warn!(code, response = %resp, "smtp MAIL FROM rejected");
            return Err(SendError::Transient(format!(
                "smtp MAIL FROM rejected (code {code})"
            )));
        }

        self.writer
            .write_all(format!("RCPT TO:<{to}>\r\n").as_bytes())
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "smtp write RCPT TO failed");
                SendError::Transient("smtp write RCPT TO failed".to_string())
            })?;
        let (code, resp) = read_smtp_response(&mut self.reader).await?;
        if (500..600).contains(&code) {
            tracing::warn!(code, response = %resp, "smtp recipient rejected");
            return Err(SendError::Permanent(format!(
                "smtp recipient rejected (code {code})"
            )));
        }
        if code != 250 && code != 251 {
            tracing::warn!(code, response = %resp, "smtp RCPT TO failed");
            return Err(SendError::Transient(format!(
                "smtp RCPT TO failed (code {code})"
            )));
        }

        self.writer.write_all(b"DATA\r\n").await.map_err(|e| {
            tracing::warn!(error = %e, "smtp write DATA failed");
            SendError::Transient("smtp write DATA failed".to_string())
        })?;
        let (code, resp) = read_smtp_response(&mut self.reader).await?;
        if code != 354 {
            tracing::warn!(code, response = %resp, "smtp DATA rejected");
            return Err(SendError::Transient(format!(
                "smtp DATA rejected (code {code})"
            )));
        }

        let clean_subject = subject.replace(['\r', '\n'], " ");
        let mut payload = format!(
            "From: Alerting Service <{from}>\r\nTo: <{to}>\r\nSubject: {clean_subject}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n"
        );
        let normalized_body = body.replace("\r\n", "\n").replace('\r', "\n");
        for line in normalized_body.split('\n') {
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
            .map_err(|e| {
                tracing::warn!(error = %e, "smtp write body failed");
                SendError::Transient("smtp write body failed".to_string())
            })?;
        let (code, resp) = read_smtp_response(&mut self.reader).await?;
        if code != 250 {
            tracing::warn!(code, response = %resp, "smtp message rejected");
            return Err(SendError::Transient(format!(
                "smtp message rejected (code {code})"
            )));
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
    pub fn new(
        backend: Arc<dyn DeliveryBackend>,
        smtp_host: impl Into<String>,
        smtp_port: u16,
    ) -> Self {
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

    pub fn with_connector(
        backend: Arc<dyn DeliveryBackend>,
        connector: Arc<dyn SmtpConnector>,
    ) -> Self {
        Self {
            backend,
            connector,
            from_address: "alerts@alerting.local".to_string(),
            pool: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            max_idle: DEFAULT_SMTP_POOL_MAX_IDLE,
        }
    }

    async fn send_pooled(
        &self,
        recipient: &str,
        subject: &str,
        body: &str,
    ) -> Result<String, SendError> {
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
                    session.quit().await;
                    return Err(err);
                }
                Ok(Err(SendError::Transient(_))) | Err(_) => {
                    drop(session);
                }
            }
        }

        let mut session = tokio::time::timeout(
            Duration::from_secs(10),
            PooledSmtpSession::open(self.connector.as_ref()),
        )
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

const FORBIDDEN_EMAIL_CHARS: &[char] = &['<', '>', ',', ';', ':', '"', '\\'];

fn validate_email_address(addr: &str) -> Result<&str, SendError> {
    let trimmed = addr.trim();
    if trimmed.is_empty()
        || trimmed
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || FORBIDDEN_EMAIL_CHARS.contains(&c))
    {
        return Err(SendError::Permanent("invalid email address".to_string()));
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return Err(SendError::Permanent("invalid email address".to_string()));
    };
    if local.is_empty() || domain.is_empty() || domain.contains('@') || !domain.contains('.') {
        return Err(SendError::Permanent("invalid email address".to_string()));
    }
    Ok(trimmed)
}

async fn read_bounded_smtp_line<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
    max_bytes: usize,
) -> Result<Option<String>, SendError> {
    let mut buf = Vec::new();
    let mut limited = reader.take((max_bytes + 1) as u64);
    let n = limited.read_until(b'\n', &mut buf).await.map_err(|e| {
        tracing::warn!(error = %e, "smtp read error");
        SendError::Transient("smtp read error".to_string())
    })?;
    if n == 0 {
        return Ok(None);
    }
    if buf.len() > max_bytes {
        tracing::warn!(max_bytes, "smtp response line exceeded max bytes");
        return Err(SendError::Transient(
            "smtp response line too long".to_string(),
        ));
    }
    if buf.ends_with(b"\n") {
        buf.pop();
        if buf.ends_with(b"\r") {
            buf.pop();
        }
    }
    let line = String::from_utf8(buf).map_err(|e| {
        tracing::warn!(error = %e, "invalid utf-8 in smtp response");
        SendError::Transient("invalid utf-8 in smtp response".to_string())
    })?;
    Ok(Some(line))
}

async fn read_smtp_response<R: AsyncRead + Unpin>(
    reader: &mut BufReader<R>,
) -> Result<(u16, String), SendError> {
    let mut full = String::new();
    let mut line_count: usize = 0;

    loop {
        line_count += 1;
        if line_count > MAX_SMTP_CONTINUATION_LINES {
            tracing::warn!(
                max_lines = MAX_SMTP_CONTINUATION_LINES,
                "smtp response exceeded max continuation lines"
            );
            return Err(SendError::Transient(
                "smtp response too many lines".to_string(),
            ));
        }

        let line = read_bounded_smtp_line(reader, MAX_SMTP_LINE_BYTES)
            .await?
            .ok_or_else(|| {
                SendError::Transient("smtp connection closed unexpectedly".to_string())
            })?;

        let added_bytes = line.len() + usize::from(!full.is_empty());
        if full.len() + added_bytes > MAX_SMTP_RESPONSE_BYTES {
            tracing::warn!(
                max_bytes = MAX_SMTP_RESPONSE_BYTES,
                "smtp response exceeded max total bytes"
            );
            return Err(SendError::Transient("smtp response too large".to_string()));
        }

        if line.len() < 3 {
            tracing::warn!(line = %line, "malformed smtp response");
            return Err(SendError::Transient("malformed smtp response".to_string()));
        }
        let code: u16 = line[..3].parse().map_err(|_| {
            tracing::warn!(line = %line, "invalid smtp status code");
            SendError::Transient("invalid smtp status code".to_string())
        })?;

        if !full.is_empty() {
            full.push('\n');
        }
        full.push_str(&line);

        if line.as_bytes().get(3) != Some(&b'-') {
            return Ok((code, full));
        }
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
                .map_err(|e| {
                    tracing::warn!(error = %e, "checking email channel availability failed");
                    SendError::Transient("email channel availability check failed".to_string())
                })?;
            if !available {
                return Err(SendError::Transient(
                    "email channel unavailable".to_string(),
                ));
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_email_address_rejects_forbidden_chars_and_accepts_valid() {
        assert_eq!(
            validate_email_address("alice@example.com").unwrap(),
            "alice@example.com"
        );
        assert_eq!(
            validate_email_address("  bob+alerts@sub.example.org  ").unwrap(),
            "bob+alerts@sub.example.org"
        );

        let invalid_cases = [
            "",
            "   ",
            "no-at-sign.example.com",
            "@example.com",
            "alice@",
            "alice@localhost",
            "alice@a@b.com",
            "alice@example.com\r\nRCPT TO:<evil@example.com>",
            "alice@example.com\nX-Injected: yes",
            "alice <evil>@example.com",
            "<alice@example.com>",
            "alice@example.com>",
            "alice,bob@example.com",
            "alice;bob@example.com",
            "alice:bob@example.com",
            "\"alice\"@example.com",
            "alice\\@example.com",
        ];

        for addr in invalid_cases {
            let err = validate_email_address(addr)
                .expect_err(&format!("expected '{addr}' to be rejected"));
            assert_eq!(
                err,
                SendError::Permanent("invalid email address".to_string())
            );
        }
    }

    #[tokio::test]
    async fn smtp_send_message_normalizes_bare_cr_and_dot_stuffs() {
        let (client, server) = tokio::io::duplex(16 * 1024);
        let (client_r, client_w) = tokio::io::split(client);

        let server_task = tokio::spawn(async move {
            let (server_r, mut server_w) = tokio::io::split(server);
            let mut reader = BufReader::new(server_r);
            let mut line = String::new();

            for expected_reply in [
                b"250 Ok\r\n".as_slice(),
                b"250 Ok\r\n".as_slice(),
                b"354 End data with <CR><LF>.<CR><LF>\r\n".as_slice(),
            ] {
                line.clear();
                reader.read_line(&mut line).await.unwrap();
                server_w.write_all(expected_reply).await.unwrap();
            }

            let mut raw_data = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                let n = reader.read(&mut chunk).await.unwrap();
                assert!(n > 0, "unexpected EOF before DATA terminator");
                raw_data.extend_from_slice(&chunk[..n]);
                if raw_data.ends_with(b"\r\n.\r\n") {
                    break;
                }
            }
            server_w.write_all(b"250 2.0.0 Ok\r\n").await.unwrap();
            raw_data
        });

        let mut session = PooledSmtpSession {
            reader: BufReader::new(Box::pin(client_r) as BoxedAsyncRead),
            writer: Box::pin(client_w) as BoxedAsyncWrite,
        };

        let malicious_body = "hello\r.\rMAIL FROM:<attacker@evil.com>\r\n.leading-dot\n..two-dots";
        let res = session
            .send_message(
                "alerts@alerting.local",
                "jan@example.com",
                "Test",
                malicious_body,
            )
            .await
            .unwrap();
        assert_eq!(res, "smtp:250");

        let raw_wire = server_task.await.unwrap();
        let wire_str = String::from_utf8(raw_wire).expect("valid utf-8 wire payload");

        for (idx, b) in wire_str.as_bytes().iter().enumerate() {
            if *b == b'\r' {
                assert_eq!(
                    wire_str.as_bytes().get(idx + 1),
                    Some(&b'\n'),
                    "bare CR found at byte {idx} in wire payload: {wire_str:?}"
                );
            }
        }

        assert!(wire_str.contains("\r\nhello\r\n..\r\nMAIL FROM:<attacker@evil.com>\r\n..leading-dot\r\n...two-dots\r\n.\r\n"));
    }

    #[tokio::test]
    async fn read_smtp_response_rejects_oversized_line() {
        let oversized = format!("250 {}\r\n", "A".repeat(MAX_SMTP_LINE_BYTES + 10));
        let mut reader = BufReader::new(oversized.as_bytes());
        let err = read_smtp_response(&mut reader).await.unwrap_err();
        assert_eq!(
            err,
            SendError::Transient("smtp response line too long".to_string())
        );
    }

    #[tokio::test]
    async fn read_smtp_response_rejects_too_many_continuation_lines() {
        let mut payload = String::new();
        for i in 0..=MAX_SMTP_CONTINUATION_LINES {
            payload.push_str(&format!("250-line-{i}\r\n"));
        }
        payload.push_str("250 final\r\n");

        let mut reader = BufReader::new(payload.as_bytes());
        let err = read_smtp_response(&mut reader).await.unwrap_err();
        assert_eq!(
            err,
            SendError::Transient("smtp response too many lines".to_string())
        );
    }

    #[tokio::test]
    async fn read_smtp_response_rejects_oversized_total_response() {
        let mut payload = String::new();
        for _ in 0..19 {
            payload.push_str(&format!("250-{}\r\n", "B".repeat(2_000)));
        }
        payload.push_str(&format!("250 {}\r\n", "B".repeat(2_000)));

        let mut reader = BufReader::new(payload.as_bytes());
        let err = read_smtp_response(&mut reader).await.unwrap_err();
        assert_eq!(
            err,
            SendError::Transient("smtp response too large".to_string())
        );
    }

    #[tokio::test]
    async fn read_smtp_response_accepts_valid_multiline_response() {
        let payload = b"250-mock.smtp.local\r\n250 SIZE 10485760\r\n";
        let mut reader = BufReader::new(&payload[..]);
        let (code, body) = read_smtp_response(&mut reader).await.unwrap();
        assert_eq!(code, 250);
        assert_eq!(body, "250-mock.smtp.local\n250 SIZE 10485760");
    }
}
