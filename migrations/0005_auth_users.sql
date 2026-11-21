-- Tabela użytkowników do logowania (PBKDF2-HMAC-SHA256 z losową solą per użytkownik).
CREATE TABLE IF NOT EXISTS auth_users (
    username      TEXT PRIMARY KEY,
    person_id     TEXT NOT NULL,
    display_name  TEXT NOT NULL,
    password_salt TEXT NOT NULL,
    password_hash TEXT NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
