-- P3: kolumna `def` dla routera (P2). Router czyta `SELECT * FROM runbooks` i bierze definicję
-- z kolumny `def`, żeby dołączać runbooki MANUAL do powiadomień (design D10).
-- Wartość liczy Postgres: wersja po klasyfikacji kroków, a gdy jej jeszcze nie ma – definicja z pliku.
ALTER TABLE runbooks ADD COLUMN IF NOT EXISTS def jsonb GENERATED ALWAYS AS (COALESCE(resolved, definition)) STORED;
