# Konfiguracja (seed + hot reload)

Postgres jest źródłem prawdy w runtime. Pliki tutaj to seed i alternatywny sposób edycji:
właściwa binarka obserwuje swój podkatalog i robi upsert do bazy (design D5).

| Katalog / plik | Konsument | Zawartość |
|---|---|---|
| `sources/*.yaml` | `ingest` | źródła Kafki: `topic`, `schema`, `format` |
| `mappings/*.yaml` | `lifecycle` | mappingi schematów (`MappingDef`) |
| `rules/*.yaml` | `router` | reguły (`RuleDef`) |
| `people.yaml` | `router` | osoby i zespoły (`PersonDef`, `TeamDef`) |
| `runbooks/*.yaml`, `*.md`, `*.txt` | `runbooks` | runbooki AUTOMATIC (YAML) i MANUAL (tekst) |
| `actions/*.yaml` | `runbooks` | akcje deklaratywne HTTP (`ActionDef`) |
| `demo/` | `make demo` | dane i notatki do scenariusza demo |

Format każdego pliku odpowiada typom z `crates/contracts` (przykłady w `crates/contracts/fixtures/config/`).
