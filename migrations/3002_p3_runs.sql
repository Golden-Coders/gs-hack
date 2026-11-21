-- P3: wykonanie runbooków (design D9, D10). Jedyny pisarz: binarka `runbooks`.

-- Uruchomienie runbooka: raz na (alert, epizod, runbook). Także MANUAL – wtedy bez kroków,
-- tylko z treścią z podstawionymi wartościami, żeby api miało jedno źródło „runbooki alertu”.
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
    context         jsonb NOT NULL,          -- kontekst alertu dla szablonów akcji (po restarcie też)
    alert_title     text NOT NULL,
    specificity     int NOT NULL,            -- większa = bardziej szczegółowe dopasowanie
    status          text NOT NULL,           -- running | waiting | failed | rejected | completed | manual
    created_at      timestamptz NOT NULL DEFAULT now(),
    updated_at      timestamptz NOT NULL DEFAULT now(),
    UNIQUE (alert_id, episode, runbook_id)
);
CREATE INDEX IF NOT EXISTS runbook_runs_alert ON runbook_runs (alert_id, episode);
CREATE INDEX IF NOT EXISTS runbook_runs_active ON runbook_runs (status) WHERE status IN ('running', 'waiting');

-- Stan kroku per próba; bieżący stan = najwyższy `attempt`. Klucz = klucz idempotencji kroku.
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
