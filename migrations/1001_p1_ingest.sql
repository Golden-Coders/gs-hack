-- P1: błędy ingestu (np. niepoprawny JSON z topiku Kafki). Czytane przez GET /ingest/errors.
CREATE TABLE ingest_errors (
    id     BIGSERIAL PRIMARY KEY,
    source TEXT NOT NULL,
    error  TEXT NOT NULL,
    sample TEXT,
    at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX ingest_errors_at_idx ON ingest_errors (at DESC);
