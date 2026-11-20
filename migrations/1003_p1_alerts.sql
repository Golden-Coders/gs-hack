-- P1: alerty i epizody. Jedynym pisarzem jest lifecycle (dedup); api tylko czyta.
-- Jeden alert na fingerprint na zawsze: reopen = ten sam wiersz, episode + 1.
CREATE TABLE alerts (
    id          UUID PRIMARY KEY,
    fingerprint TEXT NOT NULL UNIQUE,
    state       TEXT NOT NULL,          -- contracts::AlertState (snake_case)
    severity    TEXT NOT NULL,          -- contracts::Severity (snake_case)
    title       TEXT NOT NULL,
    schema      TEXT NOT NULL,
    episode     INT NOT NULL,
    count       BIGINT NOT NULL,        -- licznik bieżącego epizodu
    first_seen  TIMESTAMPTZ NOT NULL,   -- początek bieżącego epizodu
    last_seen   TIMESTAMPTZ NOT NULL,
    last_event  JSONB NOT NULL,         -- contracts::CanonicalEvent
    acked_by    JSONB,                  -- contracts::Actor
    resolved_by JSONB,                  -- contracts::Actor
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX alerts_updated_at_idx ON alerts (updated_at DESC);

CREATE TABLE alert_episodes (
    alert_id    UUID NOT NULL,
    episode     INT NOT NULL,
    opened_at   TIMESTAMPTZ NOT NULL,
    acked_at    TIMESTAMPTZ,
    acked_by    JSONB,
    resolved_at TIMESTAMPTZ,
    resolved_by JSONB,
    count       BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (alert_id, episode)
);
