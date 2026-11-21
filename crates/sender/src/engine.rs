//! Silnik dostarczania powiadomień: idempotencja (D9), retry z backoffem (D8),
//! przechodzenie po łańcuchu fallbacku oraz kierowanie do DLQ (sekcje 9.2 i 9.3).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use contracts::{Channel, DeliveryRequest, DeliveryStatus, Rendered, UiEvent, UiEventKind};
use uuid::Uuid;

use crate::adapters::{
    ChannelAdapter, EmailAdapter, PhoneMockAdapter, SendError, SmsMockAdapter, UiAdapter,
};
use crate::store::{ClaimOutcome, DeliveryBackend, DeliveryRecord, UpsertDelivery};

/// Domyślne odstępy backoffu między próbami (1 s, 5 s, 30 s) zgodnie z design D8.
pub const DEFAULT_BACKOFF_MS: [u64; 3] = [1_000, 5_000, 30_000];

/// Domyślna maksymalna liczba prób na jednym kanale przed przejściem do fallbacku / DLQ.
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

/// Konfiguracja zachowania retry i backoffu w `SenderEngine`.
#[derive(Debug, Clone)]
pub struct SenderConfig {
    /// Maksymalna liczba prób na pojedynczym kanale (domyślnie 3).
    pub max_attempts: u32,
    /// Odstępy czasu dla kolejnych retry w milisekundach (domyślnie `[1000, 5000, 30000]`).
    pub backoff_ms: Vec<u64>,
}

impl Default for SenderConfig {
    fn default() -> Self {
        Self {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            backoff_ms: DEFAULT_BACKOFF_MS.to_vec(),
        }
    }
}

impl SenderConfig {
    /// Odczytuje konfigurację z env (`SENDER_MAX_ATTEMPTS`, `SENDER_RETRY_DELAYS_MS`) lub używa wartości domyślnych z D8.
    pub fn from_env() -> Self {
        let max_attempts = std::env::var("SENDER_MAX_ATTEMPTS")
            .ok()
            .and_then(|v| v.parse::<u32>().ok())
            .filter(|&v| v >= 1)
            .unwrap_or(DEFAULT_MAX_ATTEMPTS);

        let backoff_ms = std::env::var("SENDER_RETRY_DELAYS_MS")
            .ok()
            .map(|s| {
                s.split(',')
                    .filter_map(|p| p.trim().parse::<u64>().ok())
                    .collect::<Vec<_>>()
            })
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| DEFAULT_BACKOFF_MS.to_vec());

        Self {
            max_attempts,
            backoff_ms,
        }
    }

    /// Zwraca opóźnienie przed kolejną próbą po nieudanej próbie numer `attempts_done` (1-based).
    pub fn backoff_for_attempt(&self, attempts_done: u32) -> Duration {
        if self.backoff_ms.is_empty() {
            return Duration::from_secs(1);
        }
        let idx = (attempts_done.saturating_sub(1) as usize).min(self.backoff_ms.len() - 1);
        Duration::from_millis(self.backoff_ms[idx])
    }
}

/// Wynik przetworzenia pojedynczego kroku `DeliveryRequest`.
#[derive(Debug, Clone, PartialEq)]
pub enum ProcessOutcome {
    /// Dostarczenie zakończyło się sukcesem na wskazanym kanale.
    Delivered(DeliveryRecord),
    /// Dostarczenie dla tego `(alert_id, episode, person_id, channel)` było już wcześniej w stanie `delivered`.
    AlreadyDelivered(DeliveryRecord),
    /// Inny worker (np. równoległy `sender` / `sender-retry`) już wysyła ten sam klucz idempotencji.
    InProgress(DeliveryRecord),
    /// Błąd tymczasowy – zaplanowano ponowną próbę na `deliveries.retry`.
    Retrying {
        record: DeliveryRecord,
        request: DeliveryRequest,
    },
    /// Wyczerpano wszystkie kanały w łańcuchu – zlecenie trafiło do `deliveries.dlq`.
    Dlq {
        record: DeliveryRecord,
        request: DeliveryRequest,
    },
    /// Łańcuch kanałów był pusty.
    EmptyChain,
}

/// Główny silnik dostarczania powiadomień.
#[derive(Clone)]
pub struct SenderEngine {
    backend: Arc<dyn DeliveryBackend>,
    adapters: Arc<BTreeMap<Channel, Arc<dyn ChannelAdapter>>>,
    config: SenderConfig,
}

impl SenderEngine {
    /// Tworzy silnik z domyślnym zestawem adapterów (`ui`, `email`, `sms`, `phone`).
    pub fn new(
        backend: Arc<dyn DeliveryBackend>,
        smtp_host: impl Into<String>,
        smtp_port: u16,
        config: SenderConfig,
    ) -> Self {
        let mut adapters: BTreeMap<Channel, Arc<dyn ChannelAdapter>> = BTreeMap::new();
        adapters.insert(Channel::Ui, Arc::new(UiAdapter::new(backend.clone())));
        adapters.insert(
            Channel::Email,
            Arc::new(EmailAdapter::new(backend.clone(), smtp_host, smtp_port)),
        );
        adapters.insert(Channel::Sms, Arc::new(SmsMockAdapter::new(backend.clone())));
        adapters.insert(
            Channel::Phone,
            Arc::new(PhoneMockAdapter::new(backend.clone())),
        );

        Self {
            backend,
            adapters: Arc::new(adapters),
            config,
        }
    }

    /// Tworzy silnik z niestandardową mapą adapterów (przydatne przy podmianie dostawcy lub w testach).
    pub fn with_adapters(
        backend: Arc<dyn DeliveryBackend>,
        adapters: BTreeMap<Channel, Arc<dyn ChannelAdapter>>,
        config: SenderConfig,
    ) -> Self {
        Self {
            backend,
            adapters: Arc::new(adapters),
            config,
        }
    }

    pub fn backend(&self) -> &Arc<dyn DeliveryBackend> {
        &self.backend
    }

    pub fn config(&self) -> &SenderConfig {
        &self.config
    }

    async fn emit_delivery_status(&self, record: &DeliveryRecord) -> anyhow::Result<()> {
        let payload = serde_json::to_value(record)?;
        let event = UiEvent {
            alert_id: Some(record.alert_id),
            kind: UiEventKind::DeliveryStatus,
            payload,
            at: Utc::now(),
        };
        self.backend.publish_ui_event(&event).await
    }

    /// Przetwarza `DeliveryRequest` z `deliveries.requested` lub `deliveries.retry`.
    ///
    /// - Sprawdza klucz idempotencji `delivery:{alert_id}:{episode}:{person_id}:{channel}`.
    ///   Jeśli status to `delivered`, natychmiast kończy bez ponownej wysyłki.
    /// - Wywołuje odpowiedni `ChannelAdapter`.
    /// - Przy sukcesie ustawia `status = delivered`, `delivered_at = now()` i publikuje `ui.events`.
    /// - Przy błędzie `Transient`: jeśli `attempts < max_attempts`, zapisuje `status = retrying`
    ///   i publikuje na `deliveries.retry` z wyliczonym `not_before`.
    /// - Przy wyczerpaniu prób lub błędzie `Permanent`: jeśli w `chain` jest kolejny kanał,
    ///   ustawia `status = fallback` na bieżącym kanale i od razu przechodzi do kolejnego kanału;
    ///   gdy zawiodą wszystkie kanały w łańcuchu, ustawia `status = dlq` i publikuje na `deliveries.dlq`.
    pub async fn process_request(
        &self,
        mut req: DeliveryRequest,
    ) -> anyhow::Result<ProcessOutcome> {
        if req.chain.is_empty() || req.chain_index >= req.chain.len() {
            return Ok(ProcessOutcome::EmptyChain);
        }

        loop {
            let Some(channel) = req.current_channel() else {
                return Ok(ProcessOutcome::EmptyChain);
            };

            let idem_key =
                contracts::keys::delivery(req.alert_id, req.episode, &req.person_id, channel);

            let attempt_num = req.attempt + 1;

            let pending_record = match self
                .backend
                .claim_delivery(UpsertDelivery {
                    idempotency_key: &idem_key,
                    request: &req,
                    channel,
                    status: DeliveryStatus::Pending,
                    attempts: attempt_num as i32,
                    last_error: req.last_error.as_deref(),
                    delivered_at: None,
                })
                .await?
            {
                ClaimOutcome::Claimed(rec) => rec,
                ClaimOutcome::AlreadyDelivered(existing) => {
                    tracing::info!(
                        alert_id = %req.alert_id,
                        episode = req.episode,
                        person_id = %req.person_id,
                        ?channel,
                        delivery_id = %existing.id,
                        "skipping duplicate delivery: already delivered"
                    );
                    return Ok(ProcessOutcome::AlreadyDelivered(existing));
                }
                ClaimOutcome::InProgress(existing) => {
                    tracing::info!(
                        alert_id = %req.alert_id,
                        episode = req.episode,
                        person_id = %req.person_id,
                        ?channel,
                        delivery_id = %existing.id,
                        "skipping concurrent delivery: already in progress by another worker"
                    );
                    return Ok(ProcessOutcome::InProgress(existing));
                }
            };
            req.delivery_id = pending_record.id;

            let rendered = req
                .rendered
                .get(&channel)
                .cloned()
                .or_else(|| req.rendered.values().next().cloned())
                .unwrap_or_else(|| Rendered {
                    title: format!("Alert {}", req.alert_id),
                    body: String::new(),
                    link: None,
                });
            let to = req.contacts.get(&channel).map(String::as_str);

            let send_result = match self.adapters.get(&channel) {
                Some(adapter) => adapter.send(&req, &rendered, to).await,
                None => Err(SendError::Permanent(format!(
                    "no adapter for channel {channel:?}"
                ))),
            };

            match send_result {
                Ok(_provider_ref) => {
                    req.attempt = attempt_num;
                    req.not_before = None;
                    req.last_error = None;

                    let delivered_record = self
                        .backend
                        .upsert_delivery(UpsertDelivery {
                            idempotency_key: &idem_key,
                            request: &req,
                            channel,
                            status: DeliveryStatus::Delivered,
                            attempts: attempt_num as i32,
                            last_error: None,
                            delivered_at: Some(Utc::now()),
                        })
                        .await?;

                    self.emit_delivery_status(&delivered_record).await?;
                    return Ok(ProcessOutcome::Delivered(delivered_record));
                }

                Err(SendError::Transient(err_msg)) if attempt_num < self.config.max_attempts => {
                    let delay = self.config.backoff_for_attempt(attempt_num);
                    let chrono_delay = chrono::Duration::from_std(delay)
                        .unwrap_or_else(|_| chrono::Duration::seconds(1));
                    let not_before = Utc::now() + chrono_delay;

                    req.attempt = attempt_num;
                    req.not_before = Some(not_before);
                    req.last_error = Some(err_msg.clone());

                    let retrying_record = self
                        .backend
                        .upsert_delivery(UpsertDelivery {
                            idempotency_key: &idem_key,
                            request: &req,
                            channel,
                            status: DeliveryStatus::Retrying,
                            attempts: attempt_num as i32,
                            last_error: Some(&err_msg),
                            delivered_at: None,
                        })
                        .await?;

                    self.emit_delivery_status(&retrying_record).await?;
                    self.backend.publish_retry(&req).await?;

                    return Ok(ProcessOutcome::Retrying {
                        record: retrying_record,
                        request: req,
                    });
                }

                Err(err) => {
                    let err_msg = err.message().to_string();
                    req.attempt = attempt_num;
                    req.not_before = None;
                    req.last_error = Some(err_msg.clone());

                    if let Some(next_idx) = next_distinct_channel_index(&req.chain, req.chain_index)
                    {
                        let fallback_record = self
                            .backend
                            .upsert_delivery(UpsertDelivery {
                                idempotency_key: &idem_key,
                                request: &req,
                                channel,
                                status: DeliveryStatus::Fallback,
                                attempts: attempt_num as i32,
                                last_error: Some(&err_msg),
                                delivered_at: None,
                            })
                            .await?;

                        self.emit_delivery_status(&fallback_record).await?;

                        req.delivery_id = Uuid::new_v4();
                        req.chain_index = next_idx;
                        req.attempt = 0;
                        req.not_before = None;
                        req.last_error = None;
                        continue;
                    } else {
                        let dlq_record = self
                            .backend
                            .upsert_delivery(UpsertDelivery {
                                idempotency_key: &idem_key,
                                request: &req,
                                channel,
                                status: DeliveryStatus::Dlq,
                                attempts: attempt_num as i32,
                                last_error: Some(&err_msg),
                                delivered_at: None,
                            })
                            .await?;

                        self.emit_delivery_status(&dlq_record).await?;
                        self.backend.publish_dlq(&req).await?;

                        return Ok(ProcessOutcome::Dlq {
                            record: dlq_record,
                            request: req,
                        });
                    }
                }
            }
        }
    }

    /// Przygotowuje i zleca ponowne wysłanie nieudanego dostarczenia (`POST /deliveries/{id}/resend`).
    ///
    /// Zwraca `Ok(None)`, jeśli dostarczenie o podanym `delivery_id` nie istnieje.
    pub async fn resend_delivery(
        &self,
        delivery_id: Uuid,
    ) -> anyhow::Result<Option<DeliveryRequest>> {
        let Some((record, mut req)) = self.backend.get_delivery_with_request(delivery_id).await?
        else {
            return Ok(None);
        };

        req.delivery_id = record.id;
        if let Some(pos) = req.chain.iter().position(|&c| c == record.channel) {
            req.chain_index = pos;
        }
        req.attempt = 0;
        req.not_before = None;
        req.last_error = None;

        let idem_key = contracts::keys::delivery(
            record.alert_id,
            record.episode,
            &record.person_id,
            record.channel,
        );
        let updated = self
            .backend
            .upsert_delivery(UpsertDelivery {
                idempotency_key: &idem_key,
                request: &req,
                channel: record.channel,
                status: DeliveryStatus::Pending,
                attempts: 0,
                last_error: None,
                delivered_at: None,
            })
            .await?;

        self.emit_delivery_status(&updated).await?;
        self.backend.publish_requested(&req).await?;
        Ok(Some(req))
    }
}

fn next_distinct_channel_index(chain: &[Channel], current_index: usize) -> Option<usize> {
    let tried = &chain[..=current_index];
    ((current_index + 1)..chain.len()).find(|&idx| !tried.contains(&chain[idx]))
}
