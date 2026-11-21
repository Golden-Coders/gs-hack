# event-ingestion Specification

## Purpose

Przyjmowanie zdarzeń z wielu źródeł (HTTP i Kafka) w kopercie `{format, schema, payload}` i trwałe przekazanie ich do dalszego przetwarzania bez utraty danych.

## Requirements

### Requirement: Ingest koperty przez HTTP
System SHALL przyjmować na `POST /ingest` kopertę JSON z polami `format`, `schema` i `payload` oraz odpowiadać `202 Accepted` z nadanym `event_id` dopiero wtedy, gdy zdarzenie zostało trwale zapisane.

#### Scenario: Poprawna koperta
- **WHEN** klient wysyła `{"format":"json","schema":"workflow.v1","payload":{...}}` na `POST /ingest`
- **THEN** system odpowiada `202` z unikalnym `event_id`, a zdarzenie trafia do dalszego przetwarzania

#### Scenario: Koperta bez wymaganego pola
- **WHEN** klient wysyła kopertę bez pola `schema` lub `payload`
- **THEN** system odpowiada `400` z opisem brakującego pola i niczego nie zapisuje

### Requirement: Ingest surowego payloadu ze schematem w ścieżce
System SHALL przyjmować na `POST /ingest/{schema}` surowy payload i samodzielnie budować kopertę, wyznaczając `format` z nagłówka `Content-Type`.

#### Scenario: Surowy webhook w stylu Alertmanagera
- **WHEN** zewnętrzny system wysyła surowy JSON z `Content-Type: application/json` na `POST /ingest/alertmanager`
- **THEN** system tworzy kopertę z `format = json`, `schema = alertmanager` i odpowiada `202`

#### Scenario: Payload YAML
- **WHEN** klient wysyła treść YAML z `Content-Type: application/yaml` na `POST /ingest/{schema}`
- **THEN** system tworzy kopertę z `format = yaml`

### Requirement: Ingest z topiku Kafki
System SHALL konsumować skonfigurowane topiki Kafki i budować kopertę na podstawie konfiguracji źródła, która przypisuje topikowi `schema` i `format`; wiadomość będąca już kopertą SHALL być przyjęta bez zmian.

#### Scenario: Topik ze skonfigurowanym schematem
- **GIVEN** źródło `trades` skonfigurowane jako `topic: trades, schema: trade.rejected.v2, format: json`
- **WHEN** na topik `trades` trafia surowa wiadomość JSON
- **THEN** system przetwarza ją jako kopertę ze `schema = trade.rejected.v2`

#### Scenario: Nowe źródło dodane konfiguracją
- **WHEN** operator dodaje nowe źródło Kafki w konfiguracji
- **THEN** system zaczyna konsumować nowy topik bez restartu i bez zmian w kodzie

### Requirement: Parsowanie payloadu wg formatu
System SHALL parsować payload zgodnie z polem `format` (co najmniej `json` i `yaml`); nieobsługiwany format lub błąd parsowania SHALL być odrzucony z czytelnym błędem.

#### Scenario: Nieobsługiwany format
- **WHEN** koperta ma `format = xml`, a parser XML nie jest dostępny
- **THEN** przez HTTP system odpowiada `415` z listą obsługiwanych formatów; z Kafki zdarzenie trafia do widocznej w UI listy błędów ingestu

#### Scenario: Niepoprawny JSON
- **WHEN** `format = json`, a payload nie jest poprawnym JSON
- **THEN** system odrzuca zdarzenie z błędem parsowania i nie przekazuje go dalej

### Requirement: Znakowanie czasu i identyfikator zdarzenia
System SHALL nadawać każdemu przyjętemu zdarzeniu unikalny `event_id` i znacznik `received_at`, które są zachowane przez cały przepływ aż do dostarczenia.

#### Scenario: Pomiar latencji end-to-end
- **WHEN** zdarzenie zostaje dostarczone do odbiorcy
- **THEN** zapis dostarczenia zawiera `received_at` oryginalnego zdarzenia, co pozwala policzyć latencję ingest → dostarczenie

### Requirement: Brak utraty przyjętych zdarzeń
System SHALL NOT tracić zdarzenia potwierdzonego odpowiedzią `202` (lub skonsumowanego z Kafki) w przypadku awarii lub restartu dalszych etapów przetwarzania.

#### Scenario: Restart etapu normalizacji
- **GIVEN** zdarzenie zostało przyjęte z odpowiedzią `202`
- **WHEN** proces normalizacji zostaje zrestartowany przed jego przetworzeniem
- **THEN** po restarcie zdarzenie zostaje przetworzone
