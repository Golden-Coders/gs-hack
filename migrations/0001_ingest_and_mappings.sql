-- Ingest i normalizacja: błędy ingestu, mappingi schematów oraz zdarzenia UNMAPPED.

CREATE TABLE ingest_errors (
    id     BIGSERIAL PRIMARY KEY,
    source TEXT NOT NULL,
    error  TEXT NOT NULL,
    sample TEXT,
    at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ingest_errors_at_idx ON ingest_errors (at DESC);

CREATE TABLE mappings (
    schema     TEXT PRIMARY KEY,
    def        JSONB NOT NULL,          -- contracts::MappingDef
    version    BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE unmapped_events (
    event_id    UUID PRIMARY KEY,
    schema      TEXT NOT NULL,
    envelope    JSONB NOT NULL,         -- contracts::Envelope
    received_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX unmapped_events_schema_idx ON unmapped_events (schema, received_at);
