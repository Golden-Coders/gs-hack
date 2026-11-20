-- P1: mappingi schematów (źródło prawdy w runtime) i zdarzenia bez mappingu.
-- Zmiana mappingu: UPSERT z version = version + 1, potem NOTIFY config_changed, 'mappings'.
CREATE TABLE mappings (
    schema     TEXT PRIMARY KEY,
    def        JSONB NOT NULL,          -- contracts::MappingDef
    version    BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Jedno zdarzenie = jeden wiersz; GET /unmapped agreguje po schema.
-- Po zapisaniu mappingu wiersze schematu są ponownie publikowane na events.raw i usuwane.
CREATE TABLE unmapped_events (
    event_id    UUID PRIMARY KEY,
    schema      TEXT NOT NULL,
    envelope    JSONB NOT NULL,         -- contracts::Envelope
    received_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX unmapped_events_schema_idx ON unmapped_events (schema, received_at);
