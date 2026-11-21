-- P2: tabele reguł, osób, zespołów, preferencji, dostarczeń i symulowanych kanałów (D3, D5, D8, D9).
-- Brak kluczy obcych do tabel innych strumieni (P1/P3).

CREATE TABLE rules (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    team       TEXT,
    def        JSONB NOT NULL,          -- contracts::RuleDef
    enabled    BOOLEAN NOT NULL DEFAULT TRUE,
    version    BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE people (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    timezone   TEXT NOT NULL DEFAULT 'Europe/Warsaw',
    contacts   JSONB NOT NULL DEFAULT '{}'::jsonb,
    fallback   JSONB NOT NULL DEFAULT '[]'::jsonb,
    def        JSONB NOT NULL,          -- contracts::PersonDef
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE teams (
    id         TEXT PRIMARY KEY,
    name       TEXT NOT NULL,
    def        JSONB NOT NULL,          -- contracts::TeamDef
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE team_members (
    team_id   TEXT NOT NULL,
    person_id TEXT NOT NULL,
    PRIMARY KEY (team_id, person_id)
);
CREATE INDEX team_members_person_idx ON team_members (person_id);

CREATE TABLE preferences (
    id         BIGSERIAL PRIMARY KEY,
    owner_type TEXT NOT NULL,           -- 'person' | 'team'
    owner_id   TEXT NOT NULL,
    severity   TEXT NOT NULL,           -- contracts::Severity (snake_case)
    channels   JSONB NOT NULL,          -- Vec<contracts::Channel>
    hours      TEXT
);
CREATE INDEX preferences_owner_idx ON preferences (owner_type, owner_id);

CREATE TABLE deliveries (
    id              UUID PRIMARY KEY,
    idempotency_key TEXT NOT NULL UNIQUE, -- contracts::keys::delivery(alert_id, episode, person_id, channel)
    alert_id        UUID NOT NULL,
    episode         INT NOT NULL,
    person_id       TEXT NOT NULL,
    channel         TEXT NOT NULL,        -- contracts::Channel (ui, email, sms, phone)
    status          TEXT NOT NULL,        -- contracts::DeliveryStatus (pending, retrying, delivered, fallback, failed, dlq)
    attempts        INT NOT NULL DEFAULT 0,
    last_error      TEXT,
    request         JSONB NOT NULL,       -- contracts::DeliveryRequest (do POST /deliveries/{id}/resend)
    received_at     TIMESTAMPTZ NOT NULL,
    delivered_at    TIMESTAMPTZ,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (alert_id, episode, person_id, channel)
);
CREATE INDEX deliveries_alert_episode_idx ON deliveries (alert_id, episode);
CREATE INDEX deliveries_status_updated_idx ON deliveries (status, updated_at DESC);

CREATE TABLE mock_outbox (
    id        BIGSERIAL PRIMARY KEY,
    channel   TEXT NOT NULL,
    recipient TEXT NOT NULL,            -- pole `to` w MockMessage (contracts/openapi.yaml)
    person_id TEXT NOT NULL,
    title     TEXT NOT NULL,
    body      TEXT NOT NULL,
    alert_id  UUID,
    sent_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mock_outbox_sent_at_idx ON mock_outbox (sent_at DESC);

CREATE TABLE mock_channels (
    channel    TEXT PRIMARY KEY,
    available  BOOLEAN NOT NULL DEFAULT TRUE,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO mock_channels (channel, available) VALUES
    ('sms', true),
    ('phone', true),
    ('email', true),
    ('ui', true)
ON CONFLICT DO NOTHING;
