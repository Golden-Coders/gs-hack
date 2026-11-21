//! Współbieżny konsument Kafki z zachowaniem kolejności per klucz i commitem po kolei per partycja.
//!
//! Zastępuje wzorzec `recv → await handle → commit` na jednym zadaniu (przepustowość = 1/RTT)
//! pulą **pasów** (lanes): każdy pas to osobne zadanie tokio z ograniczoną kolejką, które przetwarza
//! wiadomości SEKWENCYJNIE, a pasy pracują równolegle.
//!
//! # Gwarancje
//!
//! - **Kolejność per klucz (w obrębie procesu):** pas = `hash(klucz wiadomości) % lanes` (bez klucza:
//!   `hash(partycja)`). Hash nie zależy od topiku, więc wiadomości o tym samym kluczu z różnych topików
//!   jednej subskrypcji (np. `events.normalized` i `alerts.commands` kluczowane fingerprintem) trafiają
//!   do tego samego pasa i są przetwarzane w kolejności odbioru. Przypisanie partycji jest wspólne dla
//!   topików subskrypcji (strategia `range`, patrz [`concurrent_consumer`]).
//! - **At-least-once:** offset jest „gotowy” dopiero po `Ok` z [`LaneHandler::handle`] (skutki zapisane).
//!   Commit `offset+1` następuje ściśle w kolejności per (topik, partycja) przez [`OffsetTracker`] –
//!   wolna wiadomość o niższym offsecie blokuje commit wyższych. Commity w pracy są asynchroniczne,
//!   przy rewokacji i shutdownie – synchroniczne. Skutki uboczne handlera muszą być idempotentne
//!   (po awarii/rebalansie wiadomości mogą się powtórzyć).
//! - **Błąd handlera = ponowienie TEJ SAMEJ wiadomości** w tym samym pasie po `retry_backoff`; kolejne
//!   wiadomości pasa czekają (kolejność zachowana, brak przeskakiwania). Wiadomość „trucizna” to sprawa
//!   handlera (ma zwrócić `Ok(())` i zalogować) – moduł nie interpretuje payloadu.
//! - **Backpressure:** semafor `max_in_flight` (pobrany PRZED `recv`) + ograniczone kolejki pasów.
//!   Pętla główna nigdy nie trzyma w pamięci więcej niż `max_in_flight` wiadomości.
//!   Jeden gorący klucz blokuje swój pas, a po zapełnieniu kolejki – całą pętlę (head-of-line blocking).
//!
//! # Rebalans (rewokacja partycji)
//!
//! W rdkafka callbacki rebalansu wołane są synchronicznie z wątku pollującego, czyli wewnątrz `recv()`
//! w zadaniu pętli głównej. Dlatego pasy same robią `mark_done` + commit (wspólny stan za `Mutex`),
//! a [`RevokeBarrierContext::pre_rebalance`] (`Rebalance::Revoke`) wykonuje **barierę** (blokującą,
//! `block_in_place` w runtime multi-thread, z timeoutem `revoke_timeout`): pętla główna stoi (jest w
//! `recv`), do każdego pasa trafia znacznik bariery po wszystkich zakolejkowanych wiadomościach, pas po
//! dojściu do niego woła [`LaneHandler::on_revoke`] i potwierdza; potem synchroniczny commit gotowych
//! offsetów odebranych partycji i usunięcie ich z trackera – dopiero wtedy callback wraca i broker
//! oddaje partycje nowemu właścicielowi. Po timeoucie bariera loguje ostrzeżenie i kontynuuje: wiadomości
//! odebranych partycji nie zostaną skommitowane (generacje partycji odrzucają spóźnione potwierdzenia),
//! a duplikaty chroni idempotencja skutków. Po `Assign` nic nie trzeba (stan pasów doczytuje się leniwie).
//!
//! # Shutdown
//!
//! Sygnał `shutdown`: koniec pobierania, bariera na wszystkich pasach (limit `drain_timeout`),
//! `on_revoke`, commit synchroniczny, `Ok(())`. Po przekroczeniu limitu pasy są przerywane
//! (niedokończone wiadomości wrócą po restarcie).
//!
//! # Ograniczenia
//!
//! - Kolejność gwarantowana tylko w obrębie jednego procesu; po rebalansie klucz może trafić do innego
//!   procesu (bariera sprawia, że stary właściciel skończył pracę przed oddaniem partycji, o ile
//!   zmieścił się w `revoke_timeout`).
//! - Zakładany jest protokół eager (domyślny `range`); cooperative wymagałby bariery tylko dla
//!   odebranych partycji (API bariery to wspiera, ale `Rebalance::Revoke` w tym trybie nie był testowany).
//! - Handler, który nigdy nie kończy, blokuje swój pas i commit swojej partycji.
//!
//! # Zmienne środowiskowe ([`ConcurrentConfig::from_env`], prefiks np. `NORMALIZER`)
//!
//! `{PREFIX}_LANES`, `{PREFIX}_MAX_IN_FLIGHT`, `{PREFIX}_RETRY_BACKOFF_MS`,
//! `{PREFIX}_REVOKE_TIMEOUT_MS`, `{PREFIX}_DRAIN_TIMEOUT_MS`.

use std::collections::HashMap;
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, MutexGuard, Weak};
use std::time::{Duration, Instant};

use anyhow::Context;
use rdkafka::config::ClientConfig;
use rdkafka::consumer::{BaseConsumer, CommitMode, Consumer, ConsumerContext, Rebalance, StreamConsumer};
use rdkafka::message::{Message, OwnedMessage};
use rdkafka::{ClientContext, Offset, TopicPartitionList};
use tokio::sync::{mpsc, watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinHandle;

use super::offsets::OffsetTracker;

/// Handler jednego pasa. Pas ma własną instancję (z `make_handler(lane)`), więc stan `&mut self`
/// (np. cache dedupa) nie wymaga synchronizacji; handlery bezstanowe mogą być `Clone` na współdzielonych zasobach.
pub trait LaneHandler: Send + 'static {
    /// Przetwarza jedną wiadomość. `Err` = ponowienie TEJ SAMEJ wiadomości w tym pasie (z backoffem),
    /// bez przeskakiwania – kolejność zachowana.
    fn handle(&mut self, msg: &OwnedMessage) -> impl Future<Output = anyhow::Result<()>> + Send;

    /// Okresowo (co `ConcurrentConfig::tick`, np. flush wsadowy dedupa). Domyślnie nic.
    fn on_tick(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }

    /// Przed oddaniem partycji (rewokacja) i przy shutdownie, po przetworzeniu kolejki pasa. Domyślnie nic.
    fn on_revoke(&mut self) -> impl Future<Output = ()> + Send {
        async {}
    }
}

/// Konfiguracja współbieżnego konsumenta.
#[derive(Debug, Clone)]
pub struct ConcurrentConfig {
    /// Liczba pasów (równoległość przetwarzania; kolejność jest per klucz w pasie).
    pub lanes: usize,
    /// Łączny limit wiadomości w locie (semafor) – backpressure.
    pub max_in_flight: usize,
    /// Okres `LaneHandler::on_tick` (`None` = wyłączone).
    pub tick: Option<Duration>,
    /// Odstęp między ponowieniami wiadomości, której handler zwrócił `Err`.
    pub retry_backoff: Duration,
    /// Limit czekania bariery rewokacji na zakończenie pracy pasów.
    pub revoke_timeout: Duration,
    /// Limit drenowania pasów przy shutdownie.
    pub drain_timeout: Duration,
}

impl ConcurrentConfig {
    /// Wartości z env: `{prefix}_LANES`, `{prefix}_MAX_IN_FLIGHT`, `{prefix}_RETRY_BACKOFF_MS`,
    /// `{prefix}_REVOKE_TIMEOUT_MS`, `{prefix}_DRAIN_TIMEOUT_MS`. Domyślnie `max_in_flight = max(64, 8 * lanes)`.
    pub fn from_env(prefix: &str, default_lanes: usize) -> Self {
        let get = |suffix: &str| -> Option<usize> {
            std::env::var(format!("{prefix}_{suffix}")).ok().and_then(|v| v.parse::<usize>().ok()).filter(|&v| v >= 1)
        };
        let lanes = get("LANES").unwrap_or(default_lanes).max(1);
        let ms = |suffix: &str, default: u64| Duration::from_millis(get(suffix).map_or(default, |v| v as u64));
        Self {
            lanes,
            max_in_flight: get("MAX_IN_FLIGHT").unwrap_or_else(|| (8 * lanes).max(64)),
            tick: None,
            retry_backoff: ms("RETRY_BACKOFF_MS", 200),
            revoke_timeout: ms("REVOKE_TIMEOUT_MS", 10_000),
            drain_timeout: ms("DRAIN_TIMEOUT_MS", 5_000),
        }
    }

    /// Ustawia okres ticka (np. `1 s` dla flushu dedupa).
    pub fn with_tick(mut self, tick: Duration) -> Self {
        self.tick = Some(tick);
        self
    }

    fn normalized(mut self) -> Self {
        self.lanes = self.lanes.max(1);
        self.max_in_flight = self.max_in_flight.max(1);
        self
    }
}

/// Abstrakcja commitu (prawdziwy konsument albo atrapa w testach).
/// `next_offset` to już `offset + 1` (pozycja następnej wiadomości do odczytu).
pub trait OffsetCommitter: Send + Sync {
    fn commit(&self, topic: &str, partition: i32, next_offset: i64, mode: CommitMode);
}

fn commit_with<C: ConsumerContext>(
    consumer: &impl Consumer<C>,
    topic: &str,
    partition: i32,
    next_offset: i64,
    mode: CommitMode,
) {
    let mut tpl = TopicPartitionList::new();
    if let Err(e) = tpl.add_partition_offset(topic, partition, Offset::Offset(next_offset)) {
        tracing::warn!(error = %e, topic, partition, next_offset, "failed to build TopicPartitionList");
        return;
    }
    if let Err(e) = consumer.commit(&tpl, mode) {
        tracing::warn!(error = %e, topic, partition, next_offset, "failed to commit offset");
    }
}

/// Commit przez `StreamConsumer` (słaby uchwyt – bez cyklu Arc: konsument → kontekst → dispatcher → konsument).
struct StreamCommitter<C: ConsumerContext + 'static>(Weak<StreamConsumer<C>>);

impl<C: ConsumerContext + 'static> OffsetCommitter for StreamCommitter<C> {
    fn commit(&self, topic: &str, partition: i32, next_offset: i64, mode: CommitMode) {
        if let Some(consumer) = self.0.upgrade() {
            commit_with(&*consumer, topic, partition, next_offset, mode);
        }
    }
}

/// Commit przez `BaseConsumer` przekazany do callbacku rebalansu.
struct BaseCommitter<'a, C: ConsumerContext>(&'a BaseConsumer<C>);

impl<C: ConsumerContext> OffsetCommitter for BaseCommitter<'_, C> {
    fn commit(&self, topic: &str, partition: i32, next_offset: i64, mode: CommitMode) {
        commit_with(self.0, topic, partition, next_offset, mode);
    }
}

type PartKey = (String, i32);

/// Stan commitów wspólny dla pasów i bariery.
#[derive(Default)]
struct State {
    trackers: HashMap<String, OffsetTracker>,
    /// Generacja posiadania partycji; potwierdzenia ze starą generacją (po rewokacji) są ignorowane.
    gens: HashMap<PartKey, u64>,
    /// Najwyższy gotowy offset (ciągły) – do synchronicznego commitu przy rewokacji/shutdownie.
    ready: HashMap<PartKey, i64>,
    next_gen: u64,
}

struct Shared {
    state: Mutex<State>,
    committer: Arc<dyn OffsetCommitter>,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn track(&self, topic: &str, partition: i32, offset: i64) -> u64 {
        let mut guard = self.lock();
        let st = &mut *guard;
        let gen = match st.gens.get(&(topic.to_owned(), partition)) {
            Some(&g) => g,
            None => {
                st.next_gen += 1;
                st.gens.insert((topic.to_owned(), partition), st.next_gen);
                st.next_gen
            }
        };
        st.trackers.entry(topic.to_owned()).or_default().track(partition, offset);
        gen
    }

    /// Wiadomość przetworzona (`Ok`). Commit asynchroniczny odbywa się pod mutexem, żeby commity
    /// z różnych pasów tej samej partycji nie mogły się wyprzedzić (offset nigdy nie cofa się).
    fn complete(&self, topic: &str, partition: i32, offset: i64, gen: u64) {
        let mut guard = self.lock();
        let st = &mut *guard;
        let key = (topic.to_owned(), partition);
        if st.gens.get(&key) != Some(&gen) {
            return; // partycja odebrana (rewokacja) – nie commitujemy cudzych offsetów
        }
        let Some(off) = st.trackers.get_mut(topic).and_then(|t| t.mark_done(partition, offset)) else {
            return;
        };
        st.ready.insert(key, off);
        self.committer.commit(topic, partition, off + 1, CommitMode::Async);
    }

    /// Zapomina partycje z `scope` (`None` = wszystkie) i zwraca ich gotowe offsety do commitu synchronicznego.
    fn take(&self, scope: Option<&[PartKey]>) -> Vec<(String, i32, i64)> {
        let mut guard = self.lock();
        let st = &mut *guard;
        let keys: Vec<PartKey> = match scope {
            Some(s) => s.to_vec(),
            None => st.gens.keys().cloned().collect(),
        };
        let mut out = Vec::new();
        for key in keys {
            st.gens.remove(&key);
            if let Some(t) = st.trackers.get_mut(&key.0) {
                t.remove_partition(key.1);
            }
            if let Some(off) = st.ready.remove(&key) {
                out.push((key.0, key.1, off));
            }
        }
        out
    }

    #[cfg(test)]
    fn is_tracked(&self, topic: &str, partition: i32) -> bool {
        self.lock().gens.contains_key(&(topic.to_owned(), partition))
    }
}

struct Job {
    msg: OwnedMessage,
    gen: u64,
    _permit: Option<OwnedSemaphorePermit>,
}

enum LaneMsg {
    Job(Job),
    /// Bariera: pas woła `on_revoke` i potwierdza (po przetworzeniu wszystkiego, co było przed nią w kolejce).
    Barrier(std::sync::mpsc::Sender<()>),
}

/// Pas dla wiadomości: hash klucza (niezależny od topiku) albo partycji, gdy brak klucza.
fn lane_for(msg: &OwnedMessage, lanes: usize) -> usize {
    let mut h = DefaultHasher::new();
    match msg.key() {
        Some(k) => k.hash(&mut h),
        None => msg.partition().hash(&mut h),
    }
    (h.finish() % lanes as u64) as usize
}

/// Przetwarza wiadomość z ponowieniami; `false` = przerwane (abort) bez sukcesu.
async fn process<H: LaneHandler>(
    handler: &mut H,
    job: &Job,
    backoff: Duration,
    abort: &mut watch::Receiver<bool>,
) -> bool {
    loop {
        if *abort.borrow() {
            return false;
        }
        match handler.handle(&job.msg).await {
            Ok(()) => return true,
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    topic = job.msg.topic(),
                    partition = job.msg.partition(),
                    offset = job.msg.offset(),
                    "handler failed; retrying the same message in its lane"
                );
                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = abort.changed() => {}
                }
            }
        }
    }
}

async fn lane_loop<H: LaneHandler>(
    mut handler: H,
    mut rx: mpsc::Receiver<LaneMsg>,
    shared: Arc<Shared>,
    tick: Option<Duration>,
    backoff: Duration,
    mut abort: watch::Receiver<bool>,
) {
    let mut ticker = tick.map(|d| {
        let mut i = tokio::time::interval(d);
        i.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        i
    });
    loop {
        let next = tokio::select! {
            m = rx.recv() => m,
            _ = async {
                match ticker.as_mut() {
                    Some(t) => { t.tick().await; }
                    None => std::future::pending::<()>().await,
                }
            } => {
                handler.on_tick().await;
                continue;
            }
        };
        match next {
            None => break,
            Some(LaneMsg::Barrier(ack)) => {
                handler.on_revoke().await;
                let _ = ack.send(());
            }
            Some(LaneMsg::Job(job)) => {
                if !process(&mut handler, &job, backoff, &mut abort).await {
                    break;
                }
                shared.complete(job.msg.topic(), job.msg.partition(), job.msg.offset(), job.gen);
            }
        }
    }
}

/// Rozdziela wiadomości na pasy i wykonuje barierę. Część „czysta” (bez brokera) – testowalna kanałami.
pub(crate) struct Dispatcher {
    lanes: Vec<mpsc::Sender<LaneMsg>>,
    shared: Arc<Shared>,
    abort: watch::Sender<bool>,
    _handles: Vec<JoinHandle<()>>,
}

impl Dispatcher {
    pub(crate) fn start<H: LaneHandler>(
        config: &ConcurrentConfig,
        committer: Arc<dyn OffsetCommitter>,
        make_handler: impl Fn(usize) -> H,
    ) -> Arc<Self> {
        let shared = Arc::new(Shared { state: Mutex::new(State::default()), committer });
        let (abort, abort_rx) = watch::channel(false);
        let capacity = config.max_in_flight.div_ceil(config.lanes).max(2);
        let mut lanes = Vec::with_capacity(config.lanes);
        let mut handles = Vec::with_capacity(config.lanes);
        for idx in 0..config.lanes {
            let (tx, rx) = mpsc::channel(capacity);
            lanes.push(tx);
            handles.push(tokio::spawn(lane_loop(
                make_handler(idx),
                rx,
                shared.clone(),
                config.tick,
                config.retry_backoff,
                abort_rx.clone(),
            )));
        }
        Arc::new(Self { lanes, shared, abort, _handles: handles })
    }

    /// Śledzi offset i przekazuje wiadomość do jej pasa (czeka, gdy kolejka pasa jest pełna).
    /// `Err` = pas nie żyje (np. panika handlera).
    pub(crate) async fn dispatch(&self, msg: OwnedMessage, permit: Option<OwnedSemaphorePermit>) -> Result<(), ()> {
        let gen = self.shared.track(msg.topic(), msg.partition(), msg.offset());
        let lane = lane_for(&msg, self.lanes.len());
        self.lanes[lane].send(LaneMsg::Job(Job { msg, gen, _permit: permit })).await.map_err(|_| ())
    }

    /// Bariera rewokacji (BLOKUJĄCA – woła się ją z `block_in_place`/`spawn_blocking`/wątku, nie z zadania
    /// asynchronicznego bez `block_in_place`): czeka, aż pasy przetworzą zakolejkowane wiadomości,
    /// woła `on_revoke`, commituje synchronicznie gotowe offsety `scope` (`None` = wszystkie partycje)
    /// i usuwa te partycje z trackera. Zwraca `false`, gdy pasy nie zdążyły w `timeout`.
    pub(crate) fn barrier(&self, scope: Option<&[PartKey]>, timeout: Duration, sync: &dyn OffsetCommitter) -> bool {
        let deadline = Instant::now() + timeout;
        let (tx, rx) = std::sync::mpsc::channel();
        let mut expected = 0usize;
        let mut clean = true;
        for lane in &self.lanes {
            let mut msg = LaneMsg::Barrier(tx.clone());
            loop {
                match lane.try_send(msg) {
                    Ok(()) => {
                        expected += 1;
                        break;
                    }
                    Err(mpsc::error::TrySendError::Closed(_)) => break,
                    Err(mpsc::error::TrySendError::Full(m)) => {
                        if Instant::now() >= deadline {
                            clean = false;
                            break;
                        }
                        msg = m;
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
            }
        }
        drop(tx);
        for _ in 0..expected {
            let left = deadline.saturating_duration_since(Instant::now());
            if rx.recv_timeout(left).is_err() {
                clean = false;
                break;
            }
        }
        if !clean {
            tracing::warn!(?timeout, "revoke barrier timed out; continuing (at-least-once, effects are idempotent)");
        }
        for (topic, partition, offset) in self.shared.take(scope) {
            sync.commit(&topic, partition, offset + 1, CommitMode::Sync);
        }
        clean
    }

    /// Shutdown: bariera na wszystkich pasach (z limitem), potem przerwanie pasów.
    pub(crate) async fn shutdown(self: &Arc<Self>, timeout: Duration, sync: Arc<dyn OffsetCommitter>) {
        let this = self.clone();
        let result = tokio::task::spawn_blocking(move || this.barrier(None, timeout, &*sync)).await;
        if !matches!(result, Ok(true)) {
            tracing::warn!("consumer drain incomplete at shutdown; unfinished messages will be redelivered");
        }
        let _ = self.abort.send(true);
    }
}

/// Uchwyt, przez który kontekst rebalansu widzi dispatcher działającej pętli.
#[derive(Default)]
struct RevokeHook {
    target: Mutex<Option<(Arc<Dispatcher>, Duration)>>,
}

impl RevokeHook {
    fn set(&self, value: Option<(Arc<Dispatcher>, Duration)>) {
        *self.target.lock().unwrap_or_else(|e| e.into_inner()) = value;
    }

    fn revoke<C: ConsumerContext>(&self, base: &BaseConsumer<C>, revoked: &TopicPartitionList) {
        let target = self.target.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let Some((dispatcher, timeout)) = target else { return };
        let parts: Vec<PartKey> = revoked.elements().iter().map(|e| (e.topic().to_owned(), e.partition())).collect();
        let committer = BaseCommitter(base);
        let work = || dispatcher.barrier(Some(&parts), timeout, &committer);
        // Callback jest wołany z zadania tokio: w runtime multi-thread oddajemy wątek robocy,
        // żeby pasy mogły pracować; w current_thread bariera zadziała tylko do timeoutu.
        match tokio::runtime::Handle::try_current() {
            Ok(h) if h.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
                tokio::task::block_in_place(work);
            }
            _ => {
                work();
            }
        }
    }
}

/// Kontekst konsumenta realizujący barierę przy rewokacji partycji (patrz dokumentacja modułu).
#[derive(Clone, Default)]
pub struct RevokeBarrierContext {
    hook: Arc<RevokeHook>,
}

impl ClientContext for RevokeBarrierContext {}

impl ConsumerContext for RevokeBarrierContext {
    fn pre_rebalance(&self, base_consumer: &BaseConsumer<Self>, rebalance: &Rebalance<'_>) {
        if let Rebalance::Revoke(tpl) = rebalance {
            self.hook.revoke(base_consumer, tpl);
        }
    }
}

struct HookGuard(Arc<RevokeHook>);

impl Drop for HookGuard {
    fn drop(&mut self) {
        self.0.set(None);
    }
}

/// Konsument dla [`run_concurrent`]: ustawienia jak `kafka::consumer()` (bez auto-commitu,
/// `earliest`, `session.timeout.ms=10000`) + kontekst bariery rewokacji.
///
/// `partition.assignment.strategy=range` jawnie: partycja N wszystkich topików subskrypcji trafia do
/// tego samego członka grupy, więc wiadomości o tym samym kluczu (np. fingerprint w `events.normalized`
/// i `alerts.commands`) są przetwarzane w jednym procesie – warunek kolejności per klucz między topikami.
pub fn concurrent_consumer(
    brokers: &str,
    group: &str,
    topics: &[&str],
) -> anyhow::Result<StreamConsumer<RevokeBarrierContext>> {
    let consumer: StreamConsumer<RevokeBarrierContext> = ClientConfig::new()
        .set("bootstrap.servers", brokers)
        .set("group.id", group)
        .set("enable.auto.commit", "false")
        .set("auto.offset.reset", "earliest")
        .set("session.timeout.ms", "10000")
        .set("partition.assignment.strategy", "range")
        .create_with_context(RevokeBarrierContext::default())
        .context("creating concurrent kafka consumer")?;
    consumer.subscribe(topics).context("subscribing")?;
    Ok(consumer)
}

async fn shutdown_requested(rx: &mut watch::Receiver<bool>) {
    loop {
        if *rx.borrow() {
            return;
        }
        if rx.changed().await.is_err() {
            return;
        }
    }
}

/// Pętla współbieżnego konsumowania. Wraca `Ok(())` po `shutdown` (po drenowaniu i commicie).
pub async fn run_concurrent<H: LaneHandler>(
    consumer: StreamConsumer<RevokeBarrierContext>,
    config: ConcurrentConfig,
    mut shutdown: watch::Receiver<bool>,
    make_handler: impl Fn(usize) -> H,
) -> anyhow::Result<()> {
    let config = config.normalized();
    let consumer = Arc::new(consumer);
    let committer: Arc<StreamCommitter<RevokeBarrierContext>> = Arc::new(StreamCommitter(Arc::downgrade(&consumer)));
    let dispatcher = Dispatcher::start(&config, committer.clone(), make_handler);
    let hook = consumer.context().hook.clone();
    hook.set(Some((dispatcher.clone(), config.revoke_timeout)));
    let _guard = HookGuard(hook);
    let sem = Arc::new(Semaphore::new(config.max_in_flight));
    tracing::info!(lanes = config.lanes, max_in_flight = config.max_in_flight, "concurrent consumer started");

    let outcome: anyhow::Result<()> = loop {
        // Permit PRZED recv: nie zbieramy więcej niż `max_in_flight` wiadomości.
        let permit = tokio::select! {
            biased;
            _ = shutdown_requested(&mut shutdown) => break Ok(()),
            p = sem.clone().acquire_owned() => match p { Ok(p) => p, Err(_) => break Ok(()) },
        };
        let msg = tokio::select! {
            biased;
            _ = shutdown_requested(&mut shutdown) => break Ok(()),
            r = consumer.recv() => match r {
                Ok(m) => m.detach(),
                Err(e) => {
                    drop(permit);
                    tracing::warn!(error = %e, "kafka recv error");
                    tokio::time::sleep(Duration::from_millis(200)).await;
                    continue;
                }
            },
        };
        tokio::select! {
            biased;
            _ = shutdown_requested(&mut shutdown) => break Ok(()),
            r = dispatcher.dispatch(msg, Some(permit)) => {
                if r.is_err() {
                    break Err(anyhow::anyhow!("consumer lane terminated unexpectedly"));
                }
            }
        }
    };

    tracing::info!("concurrent consumer shutting down, draining lanes");
    dispatcher.shutdown(config.drain_timeout, committer).await;
    outcome
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use rdkafka::message::Timestamp;

    use super::*;

    const T: &str = "t";

    fn msg(topic: &str, partition: i32, offset: i64, key: &str) -> OwnedMessage {
        OwnedMessage::new(
            Some(b"x".to_vec()),
            Some(key.as_bytes().to_vec()),
            topic.to_owned(),
            Timestamp::NotAvailable,
            partition,
            offset,
            None,
        )
    }

    #[derive(Default)]
    struct MockCommitter {
        log: Mutex<Vec<(String, i32, i64, bool)>>,
    }

    impl MockCommitter {
        fn entries(&self) -> Vec<(String, i32, i64, bool)> {
            self.log.lock().unwrap().clone()
        }
        fn max_next(&self, partition: i32) -> Option<i64> {
            self.entries().iter().filter(|e| e.1 == partition).map(|e| e.2).max()
        }
    }

    impl OffsetCommitter for MockCommitter {
        fn commit(&self, topic: &str, partition: i32, next_offset: i64, mode: CommitMode) {
            self.log.lock().unwrap().push((
                topic.to_owned(),
                partition,
                next_offset,
                matches!(mode, CommitMode::Sync),
            ));
        }
    }

    type DelayFn = Arc<dyn Fn(&OwnedMessage) -> Duration + Send + Sync>;

    #[derive(Clone)]
    struct TestHandler {
        log: Arc<Mutex<Vec<(String, i64)>>>,
        delay: DelayFn,
        failures: Arc<Mutex<HashMap<i64, usize>>>,
        attempts: Arc<Mutex<HashMap<i64, usize>>>,
        revoked: Arc<AtomicUsize>,
        ticks: Arc<AtomicUsize>,
    }

    impl TestHandler {
        fn new(delay: impl Fn(&OwnedMessage) -> Duration + Send + Sync + 'static) -> Self {
            Self {
                log: Default::default(),
                delay: Arc::new(delay),
                failures: Default::default(),
                attempts: Default::default(),
                revoked: Default::default(),
                ticks: Default::default(),
            }
        }
        fn handled(&self) -> Vec<(String, i64)> {
            self.log.lock().unwrap().clone()
        }
    }

    impl LaneHandler for TestHandler {
        async fn handle(&mut self, msg: &OwnedMessage) -> anyhow::Result<()> {
            let off = msg.offset();
            *self.attempts.lock().unwrap().entry(off).or_default() += 1;
            tokio::time::sleep((self.delay)(msg)).await;
            {
                let mut f = self.failures.lock().unwrap();
                if let Some(n) = f.get_mut(&off) {
                    if *n > 0 {
                        *n -= 1;
                        anyhow::bail!("boom");
                    }
                }
            }
            let key = String::from_utf8_lossy(msg.key().unwrap_or_default()).into_owned();
            self.log.lock().unwrap().push((key, off));
            Ok(())
        }
        async fn on_tick(&mut self) {
            self.ticks.fetch_add(1, Ordering::SeqCst);
        }
        async fn on_revoke(&mut self) {
            self.revoked.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn config(lanes: usize) -> ConcurrentConfig {
        ConcurrentConfig {
            lanes,
            max_in_flight: 256,
            tick: None,
            retry_backoff: Duration::from_millis(50),
            revoke_timeout: Duration::from_secs(5),
            drain_timeout: Duration::from_secs(5),
        }
    }

    async fn wait_until(what: &str, cond: impl Fn() -> bool) {
        let start = Instant::now();
        while !cond() {
            assert!(start.elapsed() < Duration::from_secs(10), "timeout waiting for: {what}");
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    fn start(
        cfg: &ConcurrentConfig,
        h: &TestHandler,
    ) -> (Arc<Dispatcher>, Arc<MockCommitter>) {
        let committer = Arc::new(MockCommitter::default());
        let h = h.clone();
        let d = Dispatcher::start(cfg, committer.clone(), move |_| h.clone());
        (d, committer)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn preserves_order_per_key_with_random_delays() {
        let h = TestHandler::new(|m| Duration::from_millis(((m.offset() * 7 + 3) % 6) as u64));
        let (d, committer) = start(&config(4), &h);
        let keys: Vec<String> = (0..10).map(|i| format!("key-{i}")).collect();
        for off in 0..200i64 {
            d.dispatch(msg(T, 0, off, &keys[(off as usize * 3) % keys.len()]), None).await.unwrap();
        }
        wait_until("all handled", || h.handled().len() == 200).await;
        let mut per_key: HashMap<String, Vec<i64>> = HashMap::new();
        for (k, o) in h.handled() {
            per_key.entry(k).or_default().push(o);
        }
        assert_eq!(per_key.len(), keys.len());
        for (k, offs) in per_key {
            assert!(offs.windows(2).all(|w| w[0] < w[1]), "order violated for {k}: {offs:?}");
        }
        wait_until("final commit", || committer.max_next(0) == Some(200)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn same_key_from_two_topics_keeps_receive_order() {
        let h = TestHandler::new(|m| Duration::from_millis((m.offset() % 3) as u64));
        let (d, _c) = start(&config(4), &h);
        for i in 0..40i64 {
            let topic = if i % 2 == 0 { "events.normalized" } else { "alerts.commands" };
            d.dispatch(msg(topic, 0, i / 2, "fp-1"), None).await.unwrap();
        }
        wait_until("all handled", || h.handled().len() == 40).await;
        // Kolejność przetworzenia = kolejność odbioru: 0,0,1,1,2,2,...
        let order: Vec<i64> = h.handled().into_iter().map(|(_, o)| o).collect();
        let expected: Vec<i64> = (0..40).map(|i| i / 2).collect();
        assert_eq!(order, expected);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn distinct_keys_are_processed_concurrently() {
        let lanes = 8;
        let h = TestHandler::new(|_| Duration::from_millis(50));
        let (d, _c) = start(&config(lanes), &h);
        // Klucze o różnych pasach, żeby test nie zależał od kolizji hasha.
        let mut keys: Vec<String> = Vec::new();
        let mut used = std::collections::HashSet::new();
        for i in 0.. {
            let k = format!("k{i}");
            if used.insert(lane_for(&msg(T, 0, 0, &k), lanes)) {
                keys.push(k);
            }
            if keys.len() == lanes {
                break;
            }
        }
        let start_at = Instant::now();
        for (i, k) in keys.iter().enumerate() {
            d.dispatch(msg(T, 0, i as i64, k), None).await.unwrap();
        }
        wait_until("all handled", || h.handled().len() == lanes).await;
        let elapsed = start_at.elapsed();
        assert!(elapsed < Duration::from_millis(250), "expected parallel run (<250ms vs 400ms sequential), got {elapsed:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn commit_does_not_pass_unfinished_message() {
        let h = TestHandler::new(|m| if m.offset() == 0 { Duration::from_millis(400) } else { Duration::ZERO });
        let (d, committer) = start(&config(5), &h);
        let keys = ["a", "b", "c", "d", "e"];
        // offset 0 to klucz "a" (wolny); pozostałe w innych pasach (jeśli kolidują – i tak czekają, więc dobieramy różne).
        let mut lane_used = std::collections::HashSet::new();
        let mut chosen = vec![];
        for k in keys.iter().chain(["f", "g", "h", "i", "j", "k"].iter()) {
            if lane_used.insert(lane_for(&msg(T, 0, 0, k), 5)) {
                chosen.push(*k);
            }
        }
        for (off, k) in chosen.iter().enumerate() {
            d.dispatch(msg(T, 0, off as i64, k), None).await.unwrap();
        }
        let n = chosen.len();
        wait_until("fast ones done", || h.handled().len() == n - 1).await;
        assert_eq!(committer.max_next(0), None, "no commit while offset 0 is still running");
        wait_until("all done", || h.handled().len() == n).await;
        wait_until("commit all", || committer.max_next(0) == Some(n as i64)).await;
        let nexts: Vec<i64> = committer.entries().iter().map(|e| e.2).collect();
        assert!(nexts.windows(2).all(|w| w[0] <= w[1]), "commits must be monotonic: {nexts:?}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn failed_message_is_retried_in_place_and_blocks_its_key_and_commit() {
        let mut cfg = config(4);
        cfg.retry_backoff = Duration::from_millis(100);
        let h = TestHandler::new(|_| Duration::ZERO);
        h.failures.lock().unwrap().insert(1, 2);
        let (d, committer) = start(&cfg, &h);
        for (off, k) in [(0, "a"), (1, "a"), (2, "a"), (3, "b"), (4, "b")] {
            d.dispatch(msg(T, 0, off, k), None).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
        let handled: Vec<i64> = h.handled().iter().map(|e| e.1).collect();
        assert!(!handled.contains(&1) && !handled.contains(&2), "key a must wait for offset 1: {handled:?}");
        assert!(committer.max_next(0).unwrap_or(0) <= 1, "commit must not jump over failing offset 1");
        wait_until("all handled", || h.handled().len() == 5).await;
        assert_eq!(*h.attempts.lock().unwrap().get(&1).unwrap(), 3);
        let a: Vec<i64> = h.handled().into_iter().filter(|e| e.0 == "a").map(|e| e.1).collect();
        assert_eq!(a, vec![0, 1, 2]);
        wait_until("commit all", || committer.max_next(0) == Some(5)).await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn revoke_barrier_waits_for_in_flight_calls_on_revoke_and_commits_sync() {
        let h = TestHandler::new(|_| Duration::from_millis(150));
        let (d, committer) = start(&config(3), &h);
        for off in 0..6i64 {
            d.dispatch(msg(T, 0, off, &format!("k{off}")), None).await.unwrap();
            d.dispatch(msg(T, 1, off, &format!("j{off}")), None).await.unwrap();
        }
        let sync = committer.clone();
        let dd = d.clone();
        let clean = tokio::task::spawn_blocking(move || {
            dd.barrier(Some(&[(T.to_owned(), 0)]), Duration::from_secs(5), &*sync)
        })
        .await
        .unwrap();
        assert!(clean);
        // Bariera wróciła => wszystko z kolejek pasów przetworzone, on_revoke w każdym pasie, commit sync przed powrotem.
        assert_eq!(h.handled().len(), 12);
        assert_eq!(h.revoked.load(Ordering::SeqCst), 3);
        let sync_commits: Vec<_> = committer.entries().into_iter().filter(|e| e.3).collect();
        assert_eq!(sync_commits, vec![(T.to_owned(), 0, 6, true)]);
        assert!(!d.shared.is_tracked(T, 0), "revoked partition must be forgotten");
        assert!(d.shared.is_tracked(T, 1), "other partition stays tracked");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn revoke_barrier_timeout_continues_and_ignores_late_completion() {
        let h = TestHandler::new(|_| Duration::from_millis(500));
        let (d, committer) = start(&config(1), &h);
        d.dispatch(msg(T, 0, 0, "a"), None).await.unwrap();
        let sync = committer.clone();
        let dd = d.clone();
        let started = Instant::now();
        let clean = tokio::task::spawn_blocking(move || {
            dd.barrier(Some(&[(T.to_owned(), 0)]), Duration::from_millis(100), &*sync)
        })
        .await
        .unwrap();
        assert!(!clean);
        assert!(started.elapsed() < Duration::from_millis(400));
        wait_until("late completion", || h.handled().len() == 1).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(committer.entries().is_empty(), "late ack of a revoked partition must not be committed");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_drains_commits_sync_and_calls_on_revoke() {
        let h = TestHandler::new(|_| Duration::from_millis(80));
        let (d, committer) = start(&config(2), &h);
        for off in 0..8i64 {
            d.dispatch(msg(T, 0, off, &format!("k{off}")), None).await.unwrap();
        }
        d.shutdown(Duration::from_secs(5), committer.clone()).await;
        assert_eq!(h.handled().len(), 8);
        assert_eq!(h.revoked.load(Ordering::SeqCst), 2);
        let last = committer.entries().into_iter().rfind(|e| e.3).expect("sync commit");
        assert_eq!(last, (T.to_owned(), 0, 8, true));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn on_tick_is_called_periodically_in_every_lane() {
        let mut cfg = config(3);
        cfg.tick = Some(Duration::from_millis(20));
        let h = TestHandler::new(|_| Duration::ZERO);
        let (_d, _c) = start(&cfg, &h);
        wait_until("ticks", || h.ticks.load(Ordering::SeqCst) >= 9).await;
    }

    #[test]
    fn from_env_defaults_and_overrides() {
        let c = ConcurrentConfig::from_env("CONC_TEST_NONE", 4);
        assert_eq!((c.lanes, c.max_in_flight), (4, 64));
        std::env::set_var("CONC_TEST_X_LANES", "16");
        std::env::set_var("CONC_TEST_X_MAX_IN_FLIGHT", "500");
        let c = ConcurrentConfig::from_env("CONC_TEST_X", 4);
        assert_eq!((c.lanes, c.max_in_flight), (16, 500));
    }

    #[test]
    fn lane_for_is_topic_independent_and_uses_partition_without_key() {
        let a = msg("x", 0, 0, "fp");
        let b = msg("y", 3, 9, "fp");
        assert_eq!(lane_for(&a, 8), lane_for(&b, 8));
        let k1 = OwnedMessage::new(None, None, "x".into(), Timestamp::NotAvailable, 5, 0, None);
        let k2 = OwnedMessage::new(None, None, "y".into(), Timestamp::NotAvailable, 5, 1, None);
        assert_eq!(lane_for(&k1, 8), lane_for(&k2, 8));
    }
}
