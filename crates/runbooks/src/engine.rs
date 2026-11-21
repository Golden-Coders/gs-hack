//! Wykonanie runbooków (zadania 12.1, 12.2, design D9, D10).
//!
//! - `alerts.changed {opened|reopened}` → run dla każdego pasującego runbooka, raz na
//!   `(alert, epizod, runbook)` (`UNIQUE` w `runbook_runs`, więc redelivery nic nie dubluje).
//! - Kroki AUTOMATIC idą po kolei. Każde przejście to warunkowy `UPDATE` (`WHERE status = …`)
//!   + wpis w `audit_log` w jednej transakcji, potem `ui.events`.
//! - Krok jest oznaczany `running` PRZED wykonaniem. Po restarcie kroki `running` są
//!   wykonywane ponownie z tym samym `Idempotency-Key` (`keys::step`), więc system docelowy
//!   odrzuca duplikat, a kroki `succeeded` nie są powtarzane.
//! - Decyzja „co dalej” to czysta funkcja [`next`]; reszta to zapis.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use arc_swap::ArcSwap;
use common::db::sqlx::{self, PgPool};
use common::kafka::{self, rdkafka::producer::FutureProducer, rdkafka::Message};
use contracts::{
    topics, Actor, AlertChanged, Approval, Automation, RunbookCommand, RunbookCommandKind,
    RunbookDef, RunbookKind, StepDef, StepStatus, Transition, UiEvent, UiEventKind,
};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::registry::Executor;
use crate::store::Snapshot;
use crate::{matching, notify};

/// Co zrobić z runem, patrząc na kroki i ich bieżące stany.
#[derive(Debug, PartialEq)]
pub enum Next<'a> {
    /// Wszystkie kroki zakończone sukcesem albo pominięte.
    Completed,
    /// Sekwencja stoi (błąd, odrzucenie, czeka na człowieka).
    Stopped(StepStatus),
    /// Krok trzeba wstrzymać: `WaitingManual` albo `WaitingApproval`.
    Wait(&'a StepDef, i32, StepStatus),
    /// Krok do wykonania (`Pending`, albo `Running` po zatwierdzeniu lub restarcie).
    Execute(&'a StepDef, i32, StepStatus),
}

/// `states`: step_id → (status, attempt) bieżącej próby; brak = `Pending`, próba 0.
pub fn next<'a>(steps: &'a [StepDef], states: &HashMap<String, (StepStatus, i32)>) -> Next<'a> {
    for step in steps {
        let (status, attempt) = states
            .get(&step.id)
            .copied()
            .unwrap_or((StepStatus::Pending, 0));
        return match status {
            StepStatus::Succeeded | StepStatus::Skipped => continue,
            StepStatus::Pending if step.automation == Automation::Manual => {
                Next::Wait(step, attempt, StepStatus::WaitingManual)
            }
            StepStatus::Pending if step.approval == Approval::Required => {
                Next::Wait(step, attempt, StepStatus::WaitingApproval)
            }
            StepStatus::Pending | StepStatus::Running => Next::Execute(step, attempt, status),
            other => Next::Stopped(other),
        };
    }
    Next::Completed
}

fn run_status(next: &Next) -> &'static str {
    match next {
        Next::Completed => "completed",
        Next::Stopped(StepStatus::Failed) => "failed",
        Next::Stopped(StepStatus::Rejected) => "rejected",
        Next::Stopped(_) | Next::Wait(..) => "waiting",
        Next::Execute(..) => "running",
    }
}

pub fn status_str(s: StepStatus) -> String {
    serde_json::to_value(s)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

fn parse_status(s: &str) -> anyhow::Result<StepStatus> {
    serde_json::from_value(Value::String(s.to_string()))
        .with_context(|| format!("unknown step status `{s}`"))
}

/// Run wczytany z bazy (to, czego potrzeba do wykonania kroków).
#[derive(Clone)]
pub struct Run {
    pub run_id: Uuid,
    pub alert_id: Uuid,
    pub episode: i32,
    pub kind: RunbookKind,
    pub title: String,
    pub steps: Vec<StepDef>,
    pub context: Value,
}

pub struct Engine {
    pub pool: PgPool,
    pub producer: FutureProducer,
    pub executor: Executor,
    pub snapshot: Arc<ArcSwap<Snapshot>>,
    /// Baza linków do alertu w treści powiadomień `escalate`/`notify`.
    pub public_ui_url: String,
    /// Blokada per `run_id`, aby nakładające się `spawn_advance` nie wykonywały jednocześnie tego samego kroku `Running`.
    pub active_runs: Arc<tokio::sync::Mutex<HashSet<Uuid>>>,
}

struct RunGuard {
    active: Arc<tokio::sync::Mutex<HashSet<Uuid>>>,
    run_id: Uuid,
}

impl Drop for RunGuard {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.active.try_lock() {
            guard.remove(&self.run_id);
        } else {
            let active = self.active.clone();
            let run_id = self.run_id;
            tokio::spawn(async move {
                active.lock().await.remove(&run_id);
            });
        }
    }
}

async fn try_acquire_run(
    active: &Arc<tokio::sync::Mutex<HashSet<Uuid>>>,
    run_id: Uuid,
) -> Option<RunGuard> {
    let mut set = active.lock().await;
    if set.insert(run_id) {
        Some(RunGuard {
            active: active.clone(),
            run_id,
        })
    } else {
        None
    }
}

impl Engine {
    /// Konsument `alerts.changed` i `runbooks.commands` (jedna grupa, jedna subskrypcja).
    /// Offset commitowany po zapisaniu skutków; błąd bazy = ponowienie tej samej wiadomości.
    /// Kroki wykonują się w osobnych zadaniach, żeby wolna akcja nie blokowała partycji.
    pub async fn consume(self: Arc<Self>, brokers: String) -> anyhow::Result<()> {
        let consumer = kafka::consumer(
            &brokers,
            topics::groups::RUNBOOKS,
            &[topics::ALERTS_CHANGED, topics::RUNBOOKS_COMMANDS],
        )?;
        tracing::info!(
            "consuming {} and {}",
            topics::ALERTS_CHANGED,
            topics::RUNBOOKS_COMMANDS
        );
        loop {
            let msg = consumer.recv().await.context("receiving from kafka")?;
            let runs = if msg.topic() == topics::RUNBOOKS_COMMANDS {
                match kafka::decode::<RunbookCommand>(&msg) {
                    Ok(cmd) => self
                        .retrying("applying command", || self.apply_command(&cmd))
                        .await
                        .into_iter()
                        .collect(),
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping undecodable runbooks.commands message");
                        vec![]
                    }
                }
            } else {
                match kafka::decode::<AlertChanged>(&msg) {
                    Ok(alert) => {
                        self.retrying("creating runs", || self.on_alert(&alert))
                            .await
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "skipping undecodable alerts.changed message");
                        vec![]
                    }
                }
            };
            runs.into_iter()
                .for_each(|run_id| self.clone().spawn_advance(run_id));
            kafka::commit(&consumer, &msg)?;
        }
    }

    /// Ponawia operację na bazie do skutku (chwilowa awaria nie gubi wiadomości).
    async fn retrying<T, F, Fut>(&self, what: &str, op: F) -> T
    where
        F: Fn() -> Fut,
        Fut: std::future::Future<Output = anyhow::Result<T>>,
    {
        loop {
            match op().await {
                Ok(v) => return v,
                Err(e) => {
                    tracing::warn!(error = %e, "{what} failed, retrying");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }
        }
    }

    /// Komenda z UI. Zwraca run do popchnięcia dalej (`None`, gdy run nie istnieje).
    /// Krok w innym stanie niż oczekiwany = komenda ignorowana (ktoś był szybszy).
    pub async fn apply_command(&self, cmd: &RunbookCommand) -> anyhow::Result<Option<Uuid>> {
        use StepStatus::*;
        let Some(run) = self.load_run(cmd.run_id).await? else {
            tracing::warn!(run_id = %cmd.run_id, "command for unknown run ignored");
            return Ok(None);
        };
        let states = self.step_states(run.run_id).await?;
        let Some(&(status, attempt)) = states.get(&cmd.step_id) else {
            tracing::warn!(run_id = %cmd.run_id, step_id = cmd.step_id, "command for unknown step ignored");
            return Ok(None);
        };
        let (step, actor) = (cmd.step_id.as_str(), &cmd.actor);
        let applied = match &cmd.kind {
            RunbookCommandKind::Approve => {
                self.transition(
                    &run,
                    step,
                    attempt,
                    &[WaitingApproval],
                    Running,
                    None,
                    None,
                    actor,
                    Some("step_approved"),
                )
                .await?
            }
            RunbookCommandKind::Reject => {
                self.transition(
                    &run,
                    step,
                    attempt,
                    &[WaitingApproval],
                    Rejected,
                    None,
                    None,
                    actor,
                    Some("step_rejected"),
                )
                .await?
            }
            RunbookCommandKind::Done { note } => {
                self.transition(
                    &run,
                    step,
                    attempt,
                    &[WaitingManual, Failed],
                    Succeeded,
                    None,
                    note.as_deref(),
                    actor,
                    Some("step_done"),
                )
                .await?
            }
            RunbookCommandKind::Skip => {
                self.transition(
                    &run,
                    step,
                    attempt,
                    &[Failed, WaitingManual, WaitingApproval],
                    Skipped,
                    None,
                    None,
                    actor,
                    Some("step_skipped"),
                )
                .await?
            }
            RunbookCommandKind::Retry => {
                status == Failed
                    && self
                        .new_attempt(&run, step, attempt + 1, actor, "step_retry")
                        .await?
            }
            RunbookCommandKind::ManualMode => {
                self.manual_mode(&run, step, status, attempt, actor).await?
            }
        };
        if !applied {
            tracing::info!(run_id = %run.run_id, step_id = step, status = status_str(status), command = ?cmd.kind, "command ignored, step state changed");
            return Ok(None);
        }
        Ok(Some(run.run_id))
    }

    /// Nowa próba kroku (`pending`) – nowy klucz idempotencji, więc akcja wykona się naprawdę.
    async fn new_attempt(
        &self,
        run: &Run,
        step_id: &str,
        attempt: i32,
        actor: &Actor,
        audit_action: &str,
    ) -> anyhow::Result<bool> {
        let mut tx = self.pool.begin().await?;
        let inserted = sqlx::query(
            "INSERT INTO step_runs (run_id, step_id, attempt, status, risk)
             SELECT run_id, step_id, $3, 'pending', risk FROM step_runs WHERE run_id = $1 AND step_id = $2 AND attempt = $3 - 1
             ON CONFLICT DO NOTHING",
        )
        .bind(run.run_id)
        .bind(step_id)
        .bind(attempt)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !inserted {
            tx.rollback().await?;
            return Ok(false);
        }
        let entry = audit(
            &mut tx,
            run.alert_id,
            run.episode,
            Some(run.run_id),
            Some(step_id),
            actor,
            audit_action,
            None,
        )
        .await?;
        tx.commit().await?;
        let status = json!({ "run_id": run.run_id, "step_id": step_id, "attempt": attempt, "status": "pending", "result": null, "note": null });
        self.ui(run.alert_id, UiEventKind::StepStatus, status).await;
        self.ui(run.alert_id, UiEventKind::AuditEntry, entry).await;
        Ok(true)
    }

    /// Tryb ręczny: ten krok i wszystkie dalsze stają się manualne (automatyka zatrzymana).
    async fn manual_mode(
        &self,
        run: &Run,
        step_id: &str,
        status: StepStatus,
        attempt: i32,
        actor: &Actor,
    ) -> anyhow::Result<bool> {
        use StepStatus::*;
        if !matches!(status, Pending | Failed | WaitingApproval | WaitingManual) {
            return Ok(false);
        }
        let Some(from) = run.steps.iter().position(|s| s.id == step_id) else {
            return Ok(false);
        };
        let mut steps = run.steps.clone();
        steps[from..]
            .iter_mut()
            .for_each(|s| s.automation = Automation::Manual);
        sqlx::query("UPDATE runbook_runs SET steps = $2, updated_at = now() WHERE run_id = $1")
            .bind(run.run_id)
            .bind(serde_json::to_value(&steps)?)
            .execute(&self.pool)
            .await?;
        let run = Run {
            steps,
            ..run.clone()
        };
        match status {
            Failed => {
                self.new_attempt(&run, step_id, attempt + 1, actor, "manual_mode")
                    .await
            }
            WaitingApproval => {
                self.transition(
                    &run,
                    step_id,
                    attempt,
                    &[WaitingApproval],
                    WaitingManual,
                    None,
                    None,
                    actor,
                    Some("manual_mode"),
                )
                .await
            }
            _ => {
                let mut tx = self.pool.begin().await?;
                let entry = audit(
                    &mut tx,
                    run.alert_id,
                    run.episode,
                    Some(run.run_id),
                    Some(step_id),
                    actor,
                    "manual_mode",
                    None,
                )
                .await?;
                tx.commit().await?;
                self.ui(run.alert_id, UiEventKind::AuditEntry, entry).await;
                Ok(true)
            }
        }
    }

    pub fn spawn_advance(self: Arc<Self>, run_id: Uuid) {
        tokio::spawn(async move {
            if let Err(e) = self.advance(run_id).await {
                tracing::warn!(%run_id, error = %e, "advancing run failed");
            }
        });
    }

    /// Po restarcie: dokończ runy, które były w toku (kroki `running` i następne `pending`).
    pub async fn resume_all(self: &Arc<Self>) -> anyhow::Result<()> {
        let ids: Vec<Uuid> =
            sqlx::query_scalar("SELECT run_id FROM runbook_runs WHERE status = 'running'")
                .fetch_all(&self.pool)
                .await?;
        if !ids.is_empty() {
            tracing::info!(runs = ids.len(), "resuming runs after restart");
        }
        ids.into_iter()
            .for_each(|id| self.clone().spawn_advance(id));
        Ok(())
    }

    /// Tworzy runy dla pasujących runbooków. Zwraca id runów (także już istniejących – redelivery).
    pub async fn on_alert(&self, alert: &AlertChanged) -> anyhow::Result<Vec<Uuid>> {
        if !matches!(alert.transition, Transition::Opened | Transition::Reopened) {
            return Ok(vec![]);
        }
        let snapshot = self.snapshot.load();
        let ctx = matching::alert_context(alert);
        let mut runs = Vec::new();
        for (def, specificity) in matching::matching(&snapshot.runbooks, &snapshot.rules, alert) {
            let rendered = matching::render_runbook(def, &ctx);
            runs.push(self.create_run(alert, &rendered, specificity, &ctx).await?);
        }
        Ok(runs)
    }

    pub async fn create_run(
        &self,
        alert: &AlertChanged,
        def: &RunbookDef,
        specificity: i32,
        ctx: &Value,
    ) -> anyhow::Result<Uuid> {
        let (kind, status) = match def.kind {
            RunbookKind::Automatic => ("automatic", "running"),
            RunbookKind::Manual => ("manual", "manual"),
        };
        let mut tx = self.pool.begin().await?;
        let inserted: Option<Uuid> = sqlx::query_scalar(
            "INSERT INTO runbook_runs (run_id, alert_id, episode, runbook_id, runbook_version, kind, title, body, steps,
                                       context, alert_title, specificity, status)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)
             ON CONFLICT (alert_id, episode, runbook_id) DO NOTHING
             RETURNING run_id",
        )
        .bind(Uuid::now_v7())
        .bind(alert.alert_id)
        .bind(alert.episode)
        .bind(&def.id)
        .bind(def.version)
        .bind(kind)
        .bind(&def.title)
        .bind(&def.body)
        .bind(serde_json::to_value(&def.steps)?)
        .bind(ctx)
        .bind(&alert.event.title)
        .bind(specificity)
        .bind(status)
        .fetch_optional(&mut *tx)
        .await?;

        let Some(run_id) = inserted else {
            tx.rollback().await?;
            let existing = sqlx::query_scalar("SELECT run_id FROM runbook_runs WHERE alert_id = $1 AND episode = $2 AND runbook_id = $3")
                .bind(alert.alert_id)
                .bind(alert.episode)
                .bind(&def.id)
                .fetch_one(&self.pool)
                .await?;
            return Ok(existing);
        };

        let registry = &self.snapshot.load().registry;
        for step in &def.steps {
            let risk = step
                .action
                .as_deref()
                .and_then(|a| registry.get(a))
                .map(|a| serde_json::to_value(a.risk))
                .transpose()?;
            sqlx::query("INSERT INTO step_runs (run_id, step_id, attempt, status, risk) VALUES ($1, $2, 0, 'pending', $3)")
                .bind(run_id)
                .bind(&step.id)
                .bind(risk.as_ref().and_then(Value::as_str))
                .execute(&mut *tx)
                .await?;
        }
        let detail = format!("{} (v{})", def.title, def.version);
        let entry = audit(
            &mut tx,
            alert.alert_id,
            alert.episode,
            Some(run_id),
            None,
            &Actor::System,
            "run_started",
            Some(&detail),
        )
        .await?;
        tx.commit().await?;
        tracing::info!(%run_id, alert_id = %alert.alert_id, episode = alert.episode, runbook = def.id, "run started");
        self.ui(alert.alert_id, UiEventKind::AuditEntry, entry)
            .await;
        Ok(run_id)
    }

    pub async fn load_run(&self, run_id: Uuid) -> anyhow::Result<Option<Run>> {
        let row: Option<(Uuid, i32, String, String, Value, Value)> =
            sqlx::query_as("SELECT alert_id, episode, kind, title, steps, context FROM runbook_runs WHERE run_id = $1")
                .bind(run_id)
                .fetch_optional(&self.pool)
                .await?;
        let Some((alert_id, episode, kind, title, steps, context)) = row else {
            return Ok(None);
        };
        Ok(Some(Run {
            run_id,
            alert_id,
            episode,
            kind: serde_json::from_value(Value::String(kind))?,
            title,
            steps: serde_json::from_value(steps)?,
            context,
        }))
    }

    async fn step_states(
        &self,
        run_id: Uuid,
    ) -> anyhow::Result<HashMap<String, (StepStatus, i32)>> {
        let rows: Vec<(String, String, i32)> = sqlx::query_as(
            "SELECT DISTINCT ON (step_id) step_id, status, attempt FROM step_runs WHERE run_id = $1 ORDER BY step_id, attempt DESC",
        )
        .bind(run_id)
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|(id, status, attempt)| Ok((id, (parse_status(&status)?, attempt))))
            .collect()
    }

    /// Prowadzi run do pierwszego kroku, który wymaga człowieka, błędu albo końca.
    pub async fn advance(&self, run_id: Uuid) -> anyhow::Result<()> {
        let Some(_guard) = try_acquire_run(&self.active_runs, run_id).await else {
            tracing::debug!(%run_id, "run already advancing, skipping duplicate advance");
            return Ok(());
        };
        loop {
            let mut run = self
                .load_run(run_id)
                .await?
                .with_context(|| format!("run {run_id} not found"))?;
            if run.kind != RunbookKind::Automatic {
                return Ok(());
            }
            self.snapshot
                .load()
                .registry
                .enforce_approval(&mut run.steps);

            let states = self.step_states(run_id).await?;
            let next = next(&run.steps, &states);
            sqlx::query("UPDATE runbook_runs SET status = $2, updated_at = now() WHERE run_id = $1 AND status <> $2")
                .bind(run_id)
                .bind(run_status(&next))
                .execute(&self.pool)
                .await?;
            match next {
                Next::Completed | Next::Stopped(_) => return Ok(()),
                Next::Wait(step, attempt, status) => {
                    self.transition(
                        &run,
                        &step.id,
                        attempt,
                        &[StepStatus::Pending],
                        status,
                        None,
                        None,
                        &Actor::System,
                        None,
                    )
                    .await?;
                    return Ok(());
                }
                Next::Execute(step, attempt, current) => {
                    if current == StepStatus::Pending
                        && !self
                            .transition(
                                &run,
                                &step.id,
                                attempt,
                                &[StepStatus::Pending],
                                StepStatus::Running,
                                None,
                                None,
                                &Actor::System,
                                None,
                            )
                            .await?
                    {
                        return Ok(()); // krok przejął ktoś inny
                    }
                    let (status, result) = match self.execute(&run, step, attempt).await {
                        Ok(out) => (StepStatus::Succeeded, out),
                        Err(error) => (StepStatus::Failed, error),
                    };
                    self.transition(
                        &run,
                        &step.id,
                        attempt,
                        &[StepStatus::Running],
                        status,
                        Some(&result),
                        None,
                        &Actor::System,
                        None,
                    )
                    .await?;
                }
            }
        }
    }

    /// Wykonuje akcję kroku. Wynik/błąd to tekst do `step_runs.result` i dziennika.
    async fn execute(&self, run: &Run, step: &StepDef, attempt: i32) -> Result<String, String> {
        let name = step.action.as_deref().ok_or("krok nie ma akcji")?;
        let snapshot = self.snapshot.load();
        if matches!(name, "escalate" | "notify") {
            let notice = notify::Notice::new(
                name,
                run.alert_id,
                run.episode,
                run.run_id,
                &run.title,
                step,
                &run.context,
                &self.public_ui_url,
            );
            let (requests, unknown) =
                notify::build(&notice, &step.params, &snapshot, chrono::Utc::now())?;
            let keys: Vec<String> = requests.iter().map(notify::delivery_key).collect();
            let already: std::collections::HashSet<String> =
                sqlx::query_scalar::<_, String>("SELECT idempotency_key FROM deliveries WHERE idempotency_key = ANY($1) AND status = 'delivered'")
                    .bind(&keys)
                    .fetch_all(&self.pool)
                    .await
                    .map_err(|e| format!("odczyt dostarczeń: {e}"))?
                    .into_iter()
                    .collect();
            let summary = notify::summary(&requests, &already, &unknown);
            for request in requests
                .iter()
                .filter(|r| !already.contains(&notify::delivery_key(r)))
            {
                kafka::publish(
                    &self.producer,
                    topics::DELIVERIES_REQUESTED,
                    &run.alert_id.to_string(),
                    request,
                )
                .await
                .map_err(|e| format!("publikacja zlecenia dostarczenia: {e}"))?;
            }
            return Ok(summary);
        }
        let action = snapshot
            .registry
            .get(name)
            .ok_or_else(|| format!("akcji `{name}` nie ma już w rejestrze"))?;
        let key = contracts::keys::step(run.run_id, &step.id, attempt as u32);
        let out = self
            .executor
            .execute(action, &step.params, &run.context, &key)
            .await
            .map_err(|e| e.to_string())?;
        Ok(format!(
            "{} {}",
            out.status,
            out.body.chars().take(200).collect::<String>()
        )
        .trim_end()
        .to_string())
    }

    /// Warunkowe przejście stanu kroku + wpis w dzienniku (jedna transakcja) + `ui.events`.
    /// Zwraca `false`, gdy krok nie był w żadnym ze stanów `from` (ktoś był szybszy).
    #[allow(clippy::too_many_arguments)]
    pub async fn transition(
        &self,
        run: &Run,
        step_id: &str,
        attempt: i32,
        from: &[StepStatus],
        to: StepStatus,
        result: Option<&str>,
        note: Option<&str>,
        actor: &Actor,
        audit_action: Option<&str>,
    ) -> anyhow::Result<bool> {
        let from: Vec<String> = from.iter().copied().map(status_str).collect();
        let to_str = status_str(to);
        let mut tx = self.pool.begin().await?;
        let changed = sqlx::query(
            "UPDATE step_runs SET status = $4, result = COALESCE($5, result), note = COALESCE($6, note), actor = $7, updated_at = now()
             WHERE run_id = $1 AND step_id = $2 AND attempt = $3 AND status = ANY($8)",
        )
        .bind(run.run_id)
        .bind(step_id)
        .bind(attempt)
        .bind(&to_str)
        .bind(result)
        .bind(note)
        .bind(serde_json::to_value(actor)?)
        .bind(&from)
        .execute(&mut *tx)
        .await?
        .rows_affected()
            == 1;
        if !changed {
            tx.rollback().await?;
            return Ok(false);
        }
        let action = match (audit_action, to) {
            (Some(a), _) => a.to_string(),
            (None, StepStatus::Running) => "step_started".to_string(),
            (None, other) => format!("step_{}", status_str(other)),
        };
        let detail = result.or(note);
        let entry = audit(
            &mut tx,
            run.alert_id,
            run.episode,
            Some(run.run_id),
            Some(step_id),
            actor,
            &action,
            detail,
        )
        .await?;
        tx.commit().await?;

        tracing::info!(run_id = %run.run_id, step_id, attempt, status = to_str, "step");
        let status = json!({ "run_id": run.run_id, "step_id": step_id, "attempt": attempt, "status": to_str, "result": result, "note": note });
        self.ui(run.alert_id, UiEventKind::StepStatus, status).await;
        self.ui(run.alert_id, UiEventKind::AuditEntry, entry).await;
        Ok(true)
    }

    /// `ui.events` to tylko podgląd na żywo (źródłem prawdy jest baza), więc błąd = log.
    async fn ui(&self, alert_id: Uuid, kind: UiEventKind, payload: Value) {
        let event = UiEvent {
            alert_id: Some(alert_id),
            kind,
            payload,
            at: chrono::Utc::now(),
        };
        if let Err(e) = kafka::publish(
            &self.producer,
            topics::UI_EVENTS,
            &alert_id.to_string(),
            &event,
        )
        .await
        {
            tracing::warn!(error = %e, "publishing ui event failed");
        }
    }
}

/// Wpis w `audit_log`; zwraca go w kształcie `AuditEntry` z OpenAPI (dla `ui.events`).
#[allow(clippy::too_many_arguments)]
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    alert_id: Uuid,
    episode: i32,
    run_id: Option<Uuid>,
    step_id: Option<&str>,
    actor: &Actor,
    action: &str,
    detail: Option<&str>,
) -> anyhow::Result<Value> {
    let actor = serde_json::to_value(actor)?;
    let (id, at): (i64, chrono::DateTime<chrono::Utc>) = sqlx::query_as(
        "INSERT INTO audit_log (alert_id, episode, run_id, step_id, actor, action, detail) VALUES ($1, $2, $3, $4, $5, $6, $7) RETURNING id, at",
    )
    .bind(alert_id)
    .bind(episode)
    .bind(run_id)
    .bind(step_id)
    .bind(&actor)
    .bind(action)
    .bind(detail)
    .fetch_one(&mut **tx)
    .await?;
    Ok(
        json!({ "id": id, "alert_id": alert_id, "episode": episode, "run_id": run_id, "step_id": step_id, "actor": actor, "action": action, "detail": detail, "at": at }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse::parse_runbook;
    use crate::registry::Registry;

    fn steps() -> Vec<StepDef> {
        let def = parse_runbook(
            "r.yaml",
            "steps:\n  - {action: http_get, url: x}\n  - {action: http_get, url: y, approval: required}\n  - Call the on-call person\n",
        )
        .unwrap();
        Registry::default().classify(&def).steps
    }

    fn states(list: &[(&str, StepStatus, i32)]) -> HashMap<String, (StepStatus, i32)> {
        list.iter()
            .map(|(id, s, a)| (id.to_string(), (*s, *a)))
            .collect()
    }

    #[test]
    fn sequence_waits_for_approval_then_manual_then_completes() {
        use StepStatus::*;
        let steps = steps();
        assert!(matches!(next(&steps, &states(&[])), Next::Execute(s, 0, Pending) if s.id == "s1"));
        assert!(
            matches!(next(&steps, &states(&[("s1", Succeeded, 0)])), Next::Wait(s, 0, WaitingApproval) if s.id == "s2")
        );
        assert!(
            matches!(next(&steps, &states(&[("s1", Succeeded, 0), ("s2", Running, 0)])), Next::Execute(s, 0, Running) if s.id == "s2")
        );
        assert!(
            matches!(next(&steps, &states(&[("s1", Succeeded, 0), ("s2", Succeeded, 0)])), Next::Wait(s, 0, WaitingManual) if s.id == "s3")
        );
        assert_eq!(
            next(
                &steps,
                &states(&[
                    ("s1", Succeeded, 0),
                    ("s2", Skipped, 0),
                    ("s3", Succeeded, 0)
                ])
            ),
            Next::Completed
        );
    }

    #[test]
    fn failure_and_rejection_stop_the_sequence() {
        use StepStatus::*;
        let steps = steps();
        let failed = next(&steps, &states(&[("s1", Failed, 0)]));
        assert_eq!(
            (run_status(&failed), failed),
            ("failed", Next::Stopped(Failed))
        );
        let rejected = next(
            &steps,
            &states(&[("s1", Succeeded, 0), ("s2", Rejected, 0)]),
        );
        assert_eq!(run_status(&rejected), "rejected");
        assert!(
            matches!(next(&steps, &states(&[("s1", Pending, 1)])), Next::Execute(s, 1, Pending) if s.id == "s1")
        );
    }

    #[test]
    fn restart_resumes_from_step_two() {
        use StepStatus::*;
        let steps = steps();
        let n = next(&steps, &states(&[("s1", Succeeded, 0), ("s2", Pending, 0)]));
        assert!(matches!(n, Next::Wait(s, _, _) if s.id == "s2"));
    }

    /// Pełny przebieg na prawdziwej bazie:
    /// `docker compose up -d postgres redpanda topics-init && cargo test -p runbooks -- --ignored`
    #[tokio::test]
    #[ignore = "wymaga Postgresa i Redpandy z docker compose"]
    async fn run_against_database() {
        use axum::{
            http::HeaderMap,
            routing::{get, post},
            Router,
        };
        use contracts::{ActionDef, ActionKind, HttpActionSpec, Risk};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let calls = Arc::new(AtomicUsize::new(0));
        let keys = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let (c, k) = (calls.clone(), keys.clone());
        let app = Router::new()
            .route("/health", get(|| async { "ok" }))
            .route(
                "/retry",
                post(move |h: HeaderMap| async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    k.lock()
                        .unwrap()
                        .push(h["idempotency-key"].to_str().unwrap().to_string());
                    "retried"
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let config = common::Config::from_env();
        let pool = common::db::connect_and_migrate(&config.database_url)
            .await
            .unwrap();
        let retry = ActionDef {
            name: "retry_test".into(),
            kind: ActionKind::Http,
            risk: Risk::Low,
            description: None,
            params_schema: None,
            http: Some(HttpActionSpec {
                method: "POST".into(),
                url: format!("{base}/retry"),
                headers: Default::default(),
                body: None,
                timeout_ms: 2000,
            }),
            version: 1,
        };
        let registry = Registry::new(vec![retry]);
        let def = parse_runbook(
            "it.yaml",
            &format!("steps:\n  - {{check: health, action: http_get, url: '{base}/health'}}\n  - {{action: retry_test}}\n  - {{action: escalate, team: x, approval: required}}\n  - Call the on-call person\n"),
        )
        .unwrap();
        let def = registry.classify(&def);
        let engine = Engine {
            pool: pool.clone(),
            producer: kafka::producer(&config.kafka_brokers).unwrap(),
            executor: Executor::new(vec!["127.0.0.1".into()]),
            snapshot: Arc::new(ArcSwap::from_pointee(Snapshot {
                registry,
                ..Default::default()
            })),
            public_ui_url: "http://localhost:5173".into(),
            active_runs: Default::default(),
        };
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../contracts/fixtures/topics/alerts.changed.json"
        );
        let mut alert: AlertChanged =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        alert.alert_id = Uuid::now_v7();

        let ctx = matching::alert_context(&alert);
        let run_id = engine.create_run(&alert, &def, 1, &ctx).await.unwrap();
        assert_eq!(
            engine.create_run(&alert, &def, 1, &ctx).await.unwrap(),
            run_id
        );

        let status = |step: &'static str| {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>("SELECT status FROM step_runs WHERE run_id = $1 AND step_id = $2 ORDER BY attempt DESC LIMIT 1")
                    .bind(run_id)
                    .bind(step)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        let run_status = || {
            let pool = pool.clone();
            async move {
                sqlx::query_scalar::<_, String>("SELECT status FROM runbook_runs WHERE run_id = $1")
                    .bind(run_id)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };

        engine.advance(run_id).await.unwrap();
        assert_eq!(
            (status("s1").await, status("s2").await, status("s3").await),
            (
                "succeeded".into(),
                "succeeded".into(),
                "waiting_approval".into()
            )
        );
        assert_eq!(run_status().await, "waiting");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        engine.advance(run_id).await.unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        sqlx::query("UPDATE step_runs SET status = 'running' WHERE run_id = $1 AND step_id = 's2'")
            .bind(run_id)
            .execute(&pool)
            .await
            .unwrap();
        engine.advance(run_id).await.unwrap();
        let keys = keys.lock().unwrap().clone();
        assert_eq!(keys, vec![contracts::keys::step(run_id, "s2", 0); 2]);

        let approver = Actor::User {
            person_id: "jan".into(),
        };
        let cmd = |step: &str, kind: RunbookCommandKind| RunbookCommand {
            alert_id: alert.alert_id,
            run_id,
            step_id: step.into(),
            kind,
            actor: approver.clone(),
            at: chrono::Utc::now(),
        };
        engine
            .apply_command(&cmd("s2", RunbookCommandKind::Reject))
            .await
            .unwrap();
        assert_eq!(status("s2").await, "succeeded");
        engine
            .apply_command(&cmd("s3", RunbookCommandKind::Approve))
            .await
            .unwrap();
        engine.advance(run_id).await.unwrap();
        assert_eq!(status("s3").await, "failed");
        assert_eq!(run_status().await, "failed");
        engine
            .apply_command(&cmd("s3", RunbookCommandKind::Retry))
            .await
            .unwrap();
        engine.advance(run_id).await.unwrap();
        let attempts: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM step_runs WHERE run_id = $1 AND step_id = 's3'",
        )
        .bind(run_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(
            (attempts, status("s3").await),
            (2, "waiting_approval".into())
        );
        engine
            .apply_command(&cmd("s3", RunbookCommandKind::Skip))
            .await
            .unwrap();
        engine.advance(run_id).await.unwrap();
        assert_eq!(
            (status("s3").await, status("s4").await, run_status().await),
            ("skipped".into(), "waiting_manual".into(), "waiting".into())
        );
        engine
            .apply_command(&cmd(
                "s4",
                RunbookCommandKind::Done {
                    note: Some("called".into()),
                },
            ))
            .await
            .unwrap();
        engine.advance(run_id).await.unwrap();
        assert_eq!(
            (status("s4").await, run_status().await),
            ("succeeded".into(), "completed".into())
        );

        let actions: Vec<String> =
            sqlx::query_scalar("SELECT action FROM audit_log WHERE run_id = $1 ORDER BY id")
                .bind(run_id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(
            actions,
            [
                "run_started",
                "step_started",
                "step_succeeded",
                "step_started",
                "step_succeeded",
                "step_waiting_approval",
                "step_succeeded",
                "step_approved",
                "step_failed",
                "step_retry",
                "step_waiting_approval",
                "step_skipped",
                "step_waiting_manual",
                "step_done",
            ]
        );
    }

    #[test]
    fn status_strings_match_contract() {
        assert_eq!(status_str(StepStatus::WaitingApproval), "waiting_approval");
        assert_eq!(
            parse_status("waiting_manual").unwrap(),
            StepStatus::WaitingManual
        );
    }

    #[tokio::test]
    async fn run_guard_prevents_concurrent_advance_for_same_run() {
        let active: Arc<tokio::sync::Mutex<HashSet<Uuid>>> = Default::default();
        let run_a = Uuid::now_v7();
        let run_b = Uuid::now_v7();

        let g1 = try_acquire_run(&active, run_a).await;
        assert!(g1.is_some());
        assert!(try_acquire_run(&active, run_a).await.is_none());
        assert!(try_acquire_run(&active, run_b).await.is_some());

        drop(g1);
        assert!(try_acquire_run(&active, run_a).await.is_some());
    }
}
