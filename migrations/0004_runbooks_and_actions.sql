-- Runbooki, akcje deklaratywne HTTP, wykonania kroków i dziennik działań (D9, D10, D11).

CREATE TABLE IF NOT EXISTS actions (
    name        text PRIMARY KEY,
    definition  jsonb NOT NULL,          -- contracts::ActionDef
    source_file text,                    -- NULL = dodana przez API
    source_text text,                    -- treść pliku; zmiana => nowa wersja
    version     bigint NOT NULL DEFAULT 1,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS runbooks (
    id          text PRIMARY KEY,
    definition  jsonb NOT NULL,          -- contracts::RunbookDef tak, jak go napisano
    resolved    jsonb,                   -- RunbookDef po klasyfikacji kroków wg rejestru
    def         jsonb GENERATED ALWAYS AS (COALESCE(resolved, definition)) STORED,
    source_file text,
    source_text text,
    version     bigint NOT NULL DEFAULT 1,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS unhandled_actions (
    action      text PRIMARY KEY,
    occurrences int NOT NULL,
    runbook_ids text[] NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS runbook_load_errors (
    source_file text PRIMARY KEY,
    error       text NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE IF NOT EXISTS runbook_runs (
    run_id          uuid PRIMARY KEY,
    alert_id        uuid NOT NULL,
    episode         int NOT NULL,
    runbook_id      text NOT NULL,
    runbook_version bigint NOT NULL,
    kind            text NOT NULL,           -- automatic | manual
    title           text NOT NULL,
    body            text,                    -- MANUAL: treść z podstawionymi wartościami
    steps           jsonb NOT NULL,          -- Vec<StepDef> z podstawionymi wartościami (migawka)
    context         jsonb NOT NULL,          -- kontekst alertu dla szablonów akcji
    alert_title     text NOT NULL,
    specificity     int NOT NULL,            -- większa = bardziej szczegółowe dopasowanie
    status          text NOT NULL,           -- running | waiting | failed | rejected | completed | manual
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (alert_id, episode, runbook_id)
);
CREATE INDEX IF NOT EXISTS runbook_runs_alert ON runbook_runs (alert_id, episode);
CREATE INDEX IF NOT EXISTS runbook_runs_active ON runbook_runs (status) WHERE status IN ('running', 'waiting');

CREATE TABLE IF NOT EXISTS step_runs (
    run_id     uuid NOT NULL,
    step_id    text NOT NULL,
    attempt    int NOT NULL,
    status     text NOT NULL,                -- contracts::StepStatus (snake_case)
    risk       text,                         -- ryzyko akcji w chwili startu (kolejka zatwierdzeń)
    result     text,                         -- wynik albo treść błędu
    note       text,
    actor      jsonb,                        -- kto zatwierdził / oznaczył / pominął
    created_at timestamptz NOT NULL DEFAULT now(),
    updated_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (run_id, step_id, attempt)
);
CREATE INDEX IF NOT EXISTS step_runs_waiting ON step_runs (status) WHERE status IN ('waiting_approval', 'running');

CREATE TABLE IF NOT EXISTS audit_log (
    id       bigserial PRIMARY KEY,
    alert_id uuid NOT NULL,
    episode  int NOT NULL,
    run_id   uuid,
    step_id  text,
    actor    jsonb NOT NULL,                 -- contracts::Actor
    action   text NOT NULL,                  -- run_started, step_started, step_succeeded, step_failed, ...
    detail   text,
    at       timestamptz NOT NULL DEFAULT now()
);
CREATE INDEX IF NOT EXISTS audit_log_alert ON audit_log (alert_id, id);
