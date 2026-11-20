# Konfiguracja (seed + hot reload)

Postgres jest źródłem prawdy w runtime. Pliki tutaj to seed i alternatywny sposób edycji:
binarka-właściciel obserwuje swój katalog i robi upsert do bazy (design D5).

| Katalog / plik | Właściciel | Zawartość |
|---|---|---|
| `p1/sources/*.yaml` | P1 (ingest) | źródła Kafki: `topic`, `schema`, `format` |
| `p1/mappings/*.yaml` | P1 (lifecycle) | mappingi schematów (`MappingDef`) |
| `p2/rules/*.yaml` | P2 (router) | reguły (`RuleDef`) |
| `p2/people.yaml` | P2 (router) | osoby i zespoły (`PersonDef`, `TeamDef`) |
| `p3/runbooks/*.yaml`, `*.md`, `*.txt` | P3 (runbooks) | runbooki AUTOMATIC (YAML) i MANUAL (tekst) |
| `p3/actions/*.yaml` | P3 (runbooks) | akcje deklaratywne HTTP (`ActionDef`) |
| `demo/` | P4 | dane do `make demo` |

Format każdego pliku odpowiada typom z `crates/contracts` (przykłady w `crates/contracts/fixtures/config/`).
