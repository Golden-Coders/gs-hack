//! Testy weryfikujące sekcje 9.1, 9.2 i 9.3 (`crates/sender`) oraz scenariusze ze specyfikacji `notification-delivery`.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use contracts::{Channel, DeliveryRequest, DeliveryStatus, Rendered, UiEventKind};
use sender::adapters::{BoxedAsyncRead, BoxedAsyncWrite, SmtpConnector};
use sender::store::BoxFuture;
use sender::{
    format_email_body, format_phone_body, format_sms_body, ChannelAdapter, EmailAdapter, InMemoryBackend,
    PhoneMockAdapter, ProcessOutcome, SendError, SenderConfig, SenderEngine, SmsMockAdapter, UiAdapter,
};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use uuid::Uuid;

/// Wewnątrzpamięciowy serwer SMTP (odpowiednik Mailpit) na strumieniach `tokio::io::duplex`,
/// przechwytujący pełną sesję SMTP z `EmailAdapter` bez wymagania gniazd TCP w sandboxie.
struct DuplexSmtpServer {
    captured: Arc<Mutex<Vec<String>>>,
}

impl DuplexSmtpServer {
    fn new() -> (Arc<Self>, Arc<Mutex<Vec<String>>>) {
        let captured = Arc::new(Mutex::new(Vec::new()));
        (
            Arc::new(Self {
                captured: captured.clone(),
            }),
            captured,
        )
    }
}

impl SmtpConnector for DuplexSmtpServer {
    fn connect(&self) -> BoxFuture<'_, Result<(BoxedAsyncRead, BoxedAsyncWrite), SendError>> {
        let (client_stream, server_stream) = tokio::io::duplex(64 * 1024);
        let captured_conn = self.captured.clone();

        tokio::spawn(async move {
            let (reader, mut writer) = tokio::io::split(server_stream);
            let mut lines = BufReader::new(reader).lines();
            let _ = writer.write_all(b"220 mock.smtp.local ESMTP\r\n").await;

            let mut in_data = false;
            let mut data_buf = String::new();

            while let Ok(Some(line)) = lines.next_line().await {
                if in_data {
                    if line == "." {
                        in_data = false;
                        captured_conn.lock().unwrap().push(std::mem::take(&mut data_buf));
                        let _ = writer.write_all(b"250 2.0.0 Ok: queued\r\n").await;
                    } else {
                        let clean = line.strip_prefix('.').unwrap_or(&line);
                        data_buf.push_str(clean);
                        data_buf.push('\n');
                    }
                    continue;
                }

                let upper = line.to_ascii_uppercase();
                if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                    let _ = writer.write_all(b"250-mock.smtp.local\r\n250 SIZE 10485760\r\n").await;
                } else if upper.starts_with("MAIL FROM:") || upper.starts_with("RCPT TO:") || upper.starts_with("RSET") {
                    let _ = writer.write_all(b"250 2.1.0 Ok\r\n").await;
                } else if upper == "DATA" {
                    in_data = true;
                    let _ = writer.write_all(b"354 End data with <CR><LF>.<CR><LF>\r\n").await;
                } else if upper.starts_with("QUIT") {
                    let _ = writer.write_all(b"221 2.0.0 Bye\r\n").await;
                    break;
                } else {
                    let _ = writer.write_all(b"250 Ok\r\n").await;
                }
            }
        });

        let (r, w) = tokio::io::split(client_stream);
        Box::pin(async move { Ok((Box::pin(r) as BoxedAsyncRead, Box::pin(w) as BoxedAsyncWrite)) })
    }
}

fn build_test_engine(
    backend: Arc<InMemoryBackend>,
    smtp: Arc<dyn SmtpConnector>,
    config: SenderConfig,
) -> SenderEngine {
    let mut adapters: BTreeMap<Channel, Arc<dyn ChannelAdapter>> = BTreeMap::new();
    adapters.insert(Channel::Ui, Arc::new(UiAdapter::new(backend.clone())));
    adapters.insert(
        Channel::Email,
        Arc::new(EmailAdapter::with_connector(backend.clone(), smtp)),
    );
    adapters.insert(Channel::Sms, Arc::new(SmsMockAdapter::new(backend.clone())));
    adapters.insert(Channel::Phone, Arc::new(PhoneMockAdapter::new(backend.clone())));
    SenderEngine::with_adapters(backend, adapters, config)
}

fn fixture_delivery_request() -> DeliveryRequest {
    let raw = include_str!("../../contracts/fixtures/topics/deliveries.requested.json");
    serde_json::from_str(raw).expect("valid deliveries.requested fixture")
}

/// Pomocnicza pętla symulująca konsumenta `sender-retry`:
/// pobiera wiadomości z kolejki `deliveries.retry` i przetwarza je przez `SenderEngine` aż do opróżnienia kolejki.
async fn drain_retries(engine: &SenderEngine, backend: &InMemoryBackend) {
    loop {
        let retries = backend.take_retry_queue();
        if retries.is_empty() {
            break;
        }
        for req in retries {
            engine.process_request(req).await.expect("retry processing");
        }
    }
}

#[tokio::test]
async fn sms_mock_adapter_stores_message_with_link_and_emits_ui_event() {
    let backend = Arc::new(InMemoryBackend::new());
    let (smtp, _) = DuplexSmtpServer::new();
    let engine = build_test_engine(backend.clone(), smtp, SenderConfig::default());

    let req = fixture_delivery_request();
    let outcome = engine.process_request(req.clone()).await.unwrap();

    match outcome {
        ProcessOutcome::Delivered(rec) => {
            assert_eq!(rec.id, req.delivery_id);
            assert_eq!(rec.channel, Channel::Sms);
            assert_eq!(rec.status, DeliveryStatus::Delivered);
            assert_eq!(rec.attempts, 1);
            assert!(rec.delivered_at.is_some());
            assert!(rec.last_error.is_none());
        }
        other => panic!("expected Delivered, got {other:?}"),
    }

    let outbox = backend.mock_outbox();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].channel, Channel::Sms);
    assert_eq!(outbox[0].to, "+48500100200");
    assert_eq!(outbox[0].person_id, "jan");
    assert_eq!(outbox[0].title, "WORKFLOW_FAILED order-settlement");
    assert_eq!(
        outbox[0].body,
        "validate-counterparty: TIMEOUT http://localhost:5173/alerts/0192f0a0-0000-7000-8000-0000000000a1"
    );
    assert_eq!(outbox[0].alert_id, Some(req.alert_id));

    let events = backend.ui_events();
    assert!(events.iter().any(|e| e.kind == UiEventKind::MockOutbox));
    assert!(events.iter().any(|e| e.kind == UiEventKind::DeliveryStatus));
}

#[tokio::test]
async fn phone_mock_adapter_stores_call_in_mock_outbox() {
    let backend = Arc::new(InMemoryBackend::new());
    let adapter = PhoneMockAdapter::new(backend.clone());

    let mut req = fixture_delivery_request();
    req.chain = vec![Channel::Phone, Channel::Ui];
    req.contacts.insert(Channel::Phone, "+48500100200".to_string());
    let rendered = Rendered {
        title: "WORKFLOW_FAILED order-settlement".to_string(),
        body: String::new(),
        link: None,
    };

    let res = adapter.send(&req, &rendered, None).await.unwrap();
    assert_eq!(res, "mock_outbox:1");

    let outbox = backend.mock_outbox();
    assert_eq!(outbox.len(), 1);
    assert_eq!(outbox[0].channel, Channel::Phone);
    assert_eq!(outbox[0].to, "+48500100200");
    assert_eq!(outbox[0].body, "Call: WORKFLOW_FAILED order-settlement");
}

#[tokio::test]
async fn email_adapter_sends_email_over_smtp() {
    let (smtp, captured_emails) = DuplexSmtpServer::new();
    let backend = Arc::new(InMemoryBackend::new());
    let engine = build_test_engine(backend.clone(), smtp, SenderConfig::default());

    let mut req = fixture_delivery_request();
    req.chain = vec![Channel::Email, Channel::Ui];
    req.chain_index = 0;

    let outcome = engine.process_request(req.clone()).await.unwrap();
    match outcome {
        ProcessOutcome::Delivered(rec) => {
            assert_eq!(rec.channel, Channel::Email);
            assert_eq!(rec.status, DeliveryStatus::Delivered);
            assert_eq!(rec.attempts, 1);
        }
        other => panic!("expected Delivered via Email, got {other:?}"),
    }

    let emails = captured_emails.lock().unwrap();
    assert_eq!(emails.len(), 1);
    let raw_mail = &emails[0];
    assert!(raw_mail.contains("Subject: WORKFLOW_FAILED order-settlement"), "raw mail: {raw_mail}");
    assert!(raw_mail.contains("To: <jan@example.com>"), "raw mail: {raw_mail}");
    assert!(
        raw_mail.contains("Counterparty service did not respond in 5000 ms"),
        "raw mail: {raw_mail}"
    );
}

#[tokio::test]
async fn idempotency_prevents_duplicate_delivery_for_same_episode_and_channel() {
    let backend = Arc::new(InMemoryBackend::new());
    let (smtp, _) = DuplexSmtpServer::new();
    let engine = build_test_engine(backend.clone(), smtp, SenderConfig::default());

    let req = fixture_delivery_request();
    let first = engine.process_request(req.clone()).await.unwrap();
    assert!(matches!(first, ProcessOutcome::Delivered(_)));
    assert_eq!(backend.mock_outbox().len(), 1);

    // Ponowne przetworzenie tego samego zlecenia (np. po restarcie konsumenta przed commitem offsetu)
    let mut duplicate_req = req.clone();
    duplicate_req.delivery_id = Uuid::new_v4();
    let second = engine.process_request(duplicate_req).await.unwrap();
    assert!(matches!(second, ProcessOutcome::AlreadyDelivered(_)));

    // Nadal dokładnie 1 wysłany SMS i 1 wiersz w bazie deliveries!
    assert_eq!(backend.mock_outbox().len(), 1);
    assert_eq!(backend.deliveries().len(), 1);

    // Nowy epizod tego samego alertu (episode = 2) -> wysyłany normalnie!
    let mut next_episode_req = req;
    next_episode_req.delivery_id = Uuid::new_v4();
    next_episode_req.episode = 2;
    let third = engine.process_request(next_episode_req).await.unwrap();
    assert!(matches!(third, ProcessOutcome::Delivered(_)));
    assert_eq!(backend.mock_outbox().len(), 2);
    assert_eq!(backend.deliveries().len(), 2);
}

#[tokio::test]
async fn disabled_sms_retries_with_backoff_then_falls_back_to_email() {
    let (smtp, captured_emails) = DuplexSmtpServer::new();
    let backend = Arc::new(InMemoryBackend::new());
    // Wyłączamy kanał SMS (symulacja awarii bramki SMS)
    backend.set_channel_available(Channel::Sms, false);

    let cfg = SenderConfig {
        max_attempts: 3,
        backoff_ms: vec![1_000, 5_000, 30_000],
    };
    let engine = build_test_engine(backend.clone(), smtp, cfg);

    let req = fixture_delivery_request();

    // Próba 1 na SMS -> status = retrying, attempt = 1
    let step1 = engine.process_request(req.clone()).await.unwrap();
    let retry1 = match step1 {
        ProcessOutcome::Retrying { record, request } => {
            assert_eq!(record.channel, Channel::Sms);
            assert_eq!(record.status, DeliveryStatus::Retrying);
            assert_eq!(record.attempts, 1);
            assert_eq!(record.last_error.as_deref(), Some("sms gateway unavailable"));
            assert_eq!(request.attempt, 1);
            assert!(request.not_before.is_some());
            request
        }
        other => panic!("expected Retrying after attempt 1, got {other:?}"),
    };

    // Próba 2 na SMS -> status = retrying, attempt = 2 (z większym backoffem)
    let _ = backend.take_retry_queue();
    let step2 = engine.process_request(retry1.clone()).await.unwrap();
    let retry2 = match step2 {
        ProcessOutcome::Retrying { record, request } => {
            assert_eq!(record.channel, Channel::Sms);
            assert_eq!(record.status, DeliveryStatus::Retrying);
            assert_eq!(record.attempts, 2);
            assert_eq!(request.attempt, 2);
            // Backoff dla próby 2 (5 s) jest późniejszy niż dla próby 1 (1 s)
            assert!(request.not_before.unwrap() > retry1.not_before.unwrap());
            request
        }
        other => panic!("expected Retrying after attempt 2, got {other:?}"),
    };

    // Próba 3 na SMS -> wyczerpanie prób na SMS -> status SMS = fallback (attempts = 3) -> przejście na Email -> Delivered!
    let _ = backend.take_retry_queue();
    let step3 = engine.process_request(retry2).await.unwrap();
    match step3 {
        ProcessOutcome::Delivered(email_rec) => {
            assert_eq!(email_rec.channel, Channel::Email);
            assert_eq!(email_rec.status, DeliveryStatus::Delivered);
            assert_eq!(email_rec.attempts, 1);
            assert!(email_rec.delivered_at.is_some());
        }
        other => panic!("expected Delivered on fallback Email channel, got {other:?}"),
    }

    // Weryfikacja wierszy w deliveries:
    // 1. wiersz: sms (status = fallback, attempts = 3, last_error = "sms gateway unavailable")
    // 2. wiersz: email (status = delivered, attempts = 1, last_error = None)
    let deliveries = backend.deliveries();
    assert_eq!(deliveries.len(), 2);
    assert_eq!(deliveries[0].channel, Channel::Sms);
    assert_eq!(deliveries[0].status, DeliveryStatus::Fallback);
    assert_eq!(deliveries[0].attempts, 3);
    assert_eq!(deliveries[0].last_error.as_deref(), Some("sms gateway unavailable"));

    assert_eq!(deliveries[1].channel, Channel::Email);
    assert_eq!(deliveries[1].status, DeliveryStatus::Delivered);
    assert_eq!(deliveries[1].attempts, 1);
    assert_eq!(deliveries[1].last_error, None);

    // Email faktycznie dotarł do serwera SMTP
    assert_eq!(captured_emails.lock().unwrap().len(), 1);
    assert!(backend.dlq_queue().is_empty());
}

#[tokio::test]
async fn all_channels_disabled_exhausts_chain_and_sends_to_dlq_then_resend_succeeds() {
    let backend = Arc::new(InMemoryBackend::new());
    let (smtp, _) = DuplexSmtpServer::new();
    // Wyłączamy wszystkie kanały w łańcuchu [sms, email, ui]
    backend.set_channel_available(Channel::Sms, false);
    backend.set_channel_available(Channel::Email, false);
    backend.set_channel_available(Channel::Ui, false);

    let cfg = SenderConfig {
        max_attempts: 3,
        backoff_ms: vec![10, 20, 30],
    };
    let engine = build_test_engine(backend.clone(), smtp, cfg);

    let req = fixture_delivery_request();
    engine.process_request(req.clone()).await.unwrap();
    drain_retries(&engine, &backend).await;

    // Wszystkie 3 kanały wyczerpane:
    // sms -> fallback (3 próby)
    // email -> fallback (3 próby)
    // ui -> dlq (3 próby)
    let deliveries = backend.deliveries();
    assert_eq!(deliveries.len(), 3);

    assert_eq!(deliveries[0].channel, Channel::Sms);
    assert_eq!(deliveries[0].status, DeliveryStatus::Fallback);
    assert_eq!(deliveries[0].attempts, 3);
    assert_eq!(deliveries[0].last_error.as_deref(), Some("sms gateway unavailable"));

    assert_eq!(deliveries[1].channel, Channel::Email);
    assert_eq!(deliveries[1].status, DeliveryStatus::Fallback);
    assert_eq!(deliveries[1].attempts, 3);
    assert_eq!(deliveries[1].last_error.as_deref(), Some("email channel unavailable"));

    assert_eq!(deliveries[2].channel, Channel::Ui);
    assert_eq!(deliveries[2].status, DeliveryStatus::Dlq);
    assert_eq!(deliveries[2].attempts, 3);
    assert_eq!(deliveries[2].last_error.as_deref(), Some("ui channel unavailable"));

    // Wiadomość trafiła na topik deliveries.dlq
    let dlq = backend.dlq_queue();
    assert_eq!(dlq.len(), 1);
    assert_eq!(dlq[0].alert_id, req.alert_id);
    assert_eq!(dlq[0].chain_index, 2);
    assert_eq!(dlq[0].attempt, 3);
    assert_eq!(dlq[0].last_error.as_deref(), Some("ui channel unavailable"));

    // Teraz przywracamy działanie kanału UI i wywołujemy resend dla nieudanego dostarczenia (POST /deliveries/{id}/resend)
    let dlq_delivery_id = deliveries[2].id;
    backend.set_channel_available(Channel::Ui, true);

    let resent_req = engine
        .resend_delivery(dlq_delivery_id)
        .await
        .unwrap()
        .expect("dlq delivery exists");
    assert_eq!(resent_req.delivery_id, dlq_delivery_id);
    assert_eq!(resent_req.attempt, 0);
    assert_eq!(backend.requested_queue().len(), 1);

    let resend_outcome = engine.process_request(resent_req).await.unwrap();
    match resend_outcome {
        ProcessOutcome::Delivered(rec) => {
            assert_eq!(rec.id, dlq_delivery_id);
            assert_eq!(rec.channel, Channel::Ui);
            assert_eq!(rec.status, DeliveryStatus::Delivered);
            assert_eq!(rec.attempts, 1);
            assert!(rec.delivered_at.is_some());
        }
        other => panic!("expected Delivered after resend, got {other:?}"),
    }
}

#[tokio::test]
async fn permanent_error_immediately_falls_back_without_retries() {
    let backend = Arc::new(InMemoryBackend::new());
    let (smtp, _) = DuplexSmtpServer::new();
    let engine = build_test_engine(backend.clone(), smtp, SenderConfig::default());

    let mut req = fixture_delivery_request();
    // Usuwamy numer telefonu dla SMS i podajemy niepoprawny adres email -> oba dają błąd Permanent
    req.contacts.remove(&Channel::Sms);
    req.contacts.insert(Channel::Email, "not-an-email-address".to_string());

    let outcome = engine.process_request(req).await.unwrap();
    match outcome {
        ProcessOutcome::Delivered(rec) => {
            // Natychmiastowy fallback: SMS (1 próba) -> Email (1 próba) -> UI (dostarczone!)
            assert_eq!(rec.channel, Channel::Ui);
            assert_eq!(rec.status, DeliveryStatus::Delivered);
            assert_eq!(rec.attempts, 1);
        }
        other => panic!("expected immediate fallback to UI, got {other:?}"),
    }

    // Kolejka retry musi być pusta (błędy Permanent nie są ponawiane na tym samym kanale)
    assert!(backend.retry_queue().is_empty());

    let deliveries = backend.deliveries();
    assert_eq!(deliveries.len(), 3);
    assert_eq!(deliveries[0].channel, Channel::Sms);
    assert_eq!(deliveries[0].status, DeliveryStatus::Fallback);
    assert_eq!(deliveries[0].attempts, 1);

    assert_eq!(deliveries[1].channel, Channel::Email);
    assert_eq!(deliveries[1].status, DeliveryStatus::Fallback);
    assert_eq!(deliveries[1].attempts, 1);

    assert_eq!(deliveries[2].channel, Channel::Ui);
    assert_eq!(deliveries[2].status, DeliveryStatus::Delivered);
    assert_eq!(deliveries[2].attempts, 1);
}

#[tokio::test]
async fn custom_channel_adapter_can_replace_sms_mock_without_touching_engine() {
    struct CustomSmsProvider {
        sent_to: Arc<Mutex<Vec<String>>>,
    }

    impl ChannelAdapter for CustomSmsProvider {
        fn kind(&self) -> Channel {
            Channel::Sms
        }

        fn send<'a>(
            &'a self,
            _req: &'a DeliveryRequest,
            rendered: &'a Rendered,
            to: Option<&'a str>,
        ) -> sender::store::BoxFuture<'a, Result<String, SendError>> {
            let recipient = to.unwrap_or_default().to_string();
            let text = format_sms_body(rendered);
            self.sent_to.lock().unwrap().push(format!("{recipient}:{text}"));
            Box::pin(async move { Ok("twilio:SM12345".to_string()) })
        }
    }

    let backend = Arc::new(InMemoryBackend::new());
    let sent_to = Arc::new(Mutex::new(Vec::new()));
    let mut adapters: BTreeMap<Channel, Arc<dyn ChannelAdapter>> = BTreeMap::new();
    adapters.insert(
        Channel::Sms,
        Arc::new(CustomSmsProvider {
            sent_to: sent_to.clone(),
        }),
    );
    adapters.insert(Channel::Ui, Arc::new(UiAdapter::new(backend.clone())));

    let engine = SenderEngine::with_adapters(backend.clone(), adapters, SenderConfig::default());
    let req = fixture_delivery_request();

    let outcome = engine.process_request(req).await.unwrap();
    assert!(matches!(outcome, ProcessOutcome::Delivered(_)));
    let captured = sent_to.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert!(captured[0].starts_with("+48500100200:validate-counterparty: TIMEOUT http://localhost:5173/alerts/"));
}

#[test]
fn formatting_helpers_handle_links_and_empty_bodies() {
    let r = Rendered {
        title: "HighErrorRate".to_string(),
        body: "5xx > 10%".to_string(),
        link: Some("http://localhost:5173/alerts/123".to_string()),
    };
    assert_eq!(format_sms_body(&r), "5xx > 10% http://localhost:5173/alerts/123");
    assert_eq!(format_email_body(&r), "5xx > 10%\n\nhttp://localhost:5173/alerts/123");
    assert_eq!(format_phone_body(&r), "5xx > 10%");

    let empty_body = Rendered {
        title: "WORKFLOW_FAILED order-settlement".to_string(),
        body: String::new(),
        link: None,
    };
    assert_eq!(format_phone_body(&empty_body), "Call: WORKFLOW_FAILED order-settlement");
}

#[tokio::test]
async fn concurrent_sender_and_retry_on_same_key_sends_only_once() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct SlowCountingSms {
        calls: Arc<AtomicUsize>,
    }

    impl ChannelAdapter for SlowCountingSms {
        fn kind(&self) -> Channel {
            Channel::Sms
        }

        fn send<'a>(
            &'a self,
            _req: &'a DeliveryRequest,
            _rendered: &'a Rendered,
            _to: Option<&'a str>,
        ) -> futures::future::BoxFuture<'a, Result<String, SendError>> {
            let calls = self.calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                tokio::time::sleep(std::time::Duration::from_millis(60)).await;
                Ok("slow-sms:ok".to_string())
            })
        }
    }

    let backend = Arc::new(InMemoryBackend::new());
    let calls = Arc::new(AtomicUsize::new(0));
    let mut adapters: BTreeMap<Channel, Arc<dyn ChannelAdapter>> = BTreeMap::new();
    adapters.insert(Channel::Sms, Arc::new(SlowCountingSms { calls: calls.clone() }));
    adapters.insert(Channel::Ui, Arc::new(UiAdapter::new(backend.clone())));

    let engine = SenderEngine::with_adapters(backend.clone(), adapters, SenderConfig::default());
    let req1 = fixture_delivery_request();
    let mut req2 = req1.clone();
    req2.attempt = 1; // symuluje równoległą próbę z `sender-retry` na tym samym kluczu

    let (r1, r2) = tokio::join!(engine.process_request(req1), engine.process_request(req2));
    let o1 = r1.unwrap();
    let o2 = r2.unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 1, "równoległe przetwarzanie nie może wysłać dwukrotnie");
    let outcomes = [o1, o2];
    assert!(outcomes.iter().any(|o| matches!(o, ProcessOutcome::Delivered(_))));
    assert!(outcomes
        .iter()
        .any(|o| matches!(o, ProcessOutcome::InProgress(_) | ProcessOutcome::AlreadyDelivered(_))));
}

#[tokio::test]
async fn ui_as_primary_channel_delivers_and_emits_ui_event() {
    let backend = Arc::new(InMemoryBackend::new());
    let engine = SenderEngine::new(backend.clone(), "localhost", 1025, SenderConfig::default());
    let mut req = fixture_delivery_request();
    req.chain = vec![Channel::Ui];
    req.chain_index = 0;

    let outcome = engine.process_request(req.clone()).await.unwrap();
    let ProcessOutcome::Delivered(rec) = outcome else {
        panic!("expected Delivered for UI primary channel, got {outcome:?}");
    };
    assert_eq!(rec.channel, Channel::Ui);
    assert_eq!(rec.status, DeliveryStatus::Delivered);
    assert!(rec.delivered_at.is_some());
    assert!(backend.mock_outbox().is_empty(), "kanał UI nie zapisuje do mock_outbox");
    assert!(backend
        .ui_events()
        .iter()
        .any(|e| e.kind == UiEventKind::DeliveryStatus && e.alert_id == Some(req.alert_id)));
}

#[tokio::test]
async fn smtp_pool_reuses_connections_across_emails() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct CountingDuplexSmtp {
        inner: Arc<DuplexSmtpServer>,
        connects: Arc<AtomicUsize>,
    }

    impl SmtpConnector for CountingDuplexSmtp {
        fn connect(&self) -> BoxFuture<'_, Result<(BoxedAsyncRead, BoxedAsyncWrite), SendError>> {
            self.connects.fetch_add(1, Ordering::SeqCst);
            self.inner.connect()
        }
    }

    let backend = Arc::new(InMemoryBackend::new());
    let (inner, captured) = DuplexSmtpServer::new();
    let connects = Arc::new(AtomicUsize::new(0));
    let connector = Arc::new(CountingDuplexSmtp {
        inner,
        connects: connects.clone(),
    });
    let engine = build_test_engine(backend.clone(), connector, SenderConfig::default());

    for ep in 1..=5 {
        let mut req = fixture_delivery_request();
        req.episode = ep;
        req.chain = vec![Channel::Email, Channel::Ui];
        req.chain_index = 0;
        let out = engine.process_request(req).await.unwrap();
        assert!(matches!(out, ProcessOutcome::Delivered(_)));
    }

    assert_eq!(captured.lock().unwrap().len(), 5, "wszystkie 5 emaili dostarczone");
    assert_eq!(
        connects.load(Ordering::SeqCst),
        1,
        "pula SMTP powinna reużyć jedno połączenie dla sekwencyjnych wysyłek"
    );
}

#[test]
fn offset_tracker_commits_strictly_in_order_per_partition() {
    let mut tracker = sender::OffsetTracker::default();
    tracker.track(0, 100);
    tracker.track(0, 101);
    tracker.track(0, 102);
    tracker.track(1, 50);

    // Offset 102 kończy się pierwszy: nie wolno commitować, bo 100 i 101 nadal w locie.
    assert_eq!(tracker.mark_done(0, 102), None);
    // Partycja 1 jest niezależna.
    assert_eq!(tracker.mark_done(1, 50), Some(50));
    // Offset 100 kończy się: można commitować 100 (101 nadal w locie).
    assert_eq!(tracker.mark_done(0, 100), Some(100));
    // Offset 101 kończy się: okno przeskakuje od razu do 102.
    assert_eq!(tracker.mark_done(0, 101), Some(102));
    assert_eq!(tracker.in_flight(), 0);
}
