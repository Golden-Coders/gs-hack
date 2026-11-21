-- P3: rejestr akcji i runbooki (seed z config/p3 + zmiany z API), design D5, D11.
-- Tabele wykonania (runbook_runs, step_runs, audit_log) dojdą w 3002, razem z maszyną stanów.

-- Akcje deklaratywne HTTP. Akcje wbudowane (http_get, escalate, notify) żyją w kodzie.
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
    resolved    jsonb,                   -- RunbookDef po klasyfikacji kroków wg rejestru (to czyta api)
    source_file text,
    source_text text,
    version     bigint NOT NULL DEFAULT 1,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Akcje użyte w runbookach, których nie ma w rejestrze (widok „Propozycje”).
CREATE TABLE IF NOT EXISTS unhandled_actions (
    action      text PRIMARY KEY,
    occurrences int NOT NULL,
    runbook_ids text[] NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);

-- Pliki z config/p3, których nie udało się wczytać (poprzednia wersja z bazy działa dalej).
CREATE TABLE IF NOT EXISTS p3_load_errors (
    source_file text PRIMARY KEY,
    error       text NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now()
);
