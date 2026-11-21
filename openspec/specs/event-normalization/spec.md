# event-normalization Specification

## Purpose

Przekształcanie zdarzeń w dowolnym schemacie do formatu kanonicznego za pomocą mappingów definiowanych konfiguracją, z obsługą schematów, dla których mappingu jeszcze nie ma.

## Requirements

### Requirement: Mapping per schemat
System SHALL wybierać mapping na podstawie pola `schema` koperty i stosować go do zbudowania zdarzenia kanonicznego zawierającego co najmniej: `event_id`, `schema`, `source`, `severity`, `title`, `fingerprint`, `resolved`, wyciągnięte pola oraz oryginalny payload jako `raw`.

#### Scenario: Zdarzenie workflow
- **GIVEN** mapping `workflow.v1` wyciągający `severity` z `$.severity` i `title` z `$.error.message`
- **WHEN** przychodzi zdarzenie WORKFLOW_FAILED z Appendix A
- **THEN** zdarzenie kanoniczne ma `severity = error`, `title = "Counterparty service did not respond in 5000 ms"`, a `raw` zawiera pełny oryginał

### Requirement: Ekstrakcja pól ścieżkami
Mapping SHALL definiować pola kanoniczne i dodatkowe jako ścieżki do pól payloadu (także zagnieżdżonych); brak pola w payloadzie SHALL skutkować pustą wartością, a nie błędem.

#### Scenario: Zagnieżdżone pole
- **WHEN** mapping wskazuje `$.labels.service` dla zdarzenia Alertmanagera
- **THEN** pole kanoniczne `service` ma wartość `payments-api`

#### Scenario: Brakujące pole
- **WHEN** ścieżka z mappingu nie istnieje w payloadzie
- **THEN** pole jest puste, a zdarzenie jest przetwarzane dalej

### Requirement: Domyślne severity z mappingu
Mapping SHALL określać severity jako ścieżkę do pola, stałą wartość albo warunek na polach zdarzenia; wartości SHALL być znormalizowane do wspólnej skali `critical`, `error`, `warning`, `info`.

#### Scenario: Schemat bez pola severity
- **GIVEN** mapping `trade.rejected.v2` z regułą severity `critical` gdy `payload.notional > 10000000`, w przeciwnym razie `warning`
- **WHEN** przychodzi zdarzenie z `notional = 25000000`
- **THEN** zdarzenie kanoniczne ma `severity = critical`

#### Scenario: Różne nazewnictwo severity
- **WHEN** źródło podaje `ERROR`, a inne `critical`
- **THEN** obie wartości są odwzorowane na wspólną skalę zgodnie z mappingiem

### Requirement: Fingerprint z mappingu
Mapping SHALL definiować listę pól tworzących fingerprint; zdarzenia o tych samych wartościach tych pól SHALL mieć identyczny fingerprint, a domyślnie (gdy mapping go nie określa) fingerprint SHALL być liczony ze `schema` i całego payloadu.

#### Scenario: Różne uruchomienia tego samego błędu
- **GIVEN** fingerprint `workflowName + step + error.code`
- **WHEN** przychodzą dwa zdarzenia z różnym `workflowId`, ale tym samym `workflowName`, `step` i `error.code`
- **THEN** oba zdarzenia mają ten sam fingerprint

### Requirement: Warunek rozwiązania z mappingu
Mapping MAY definiować warunek `resolved`; zdarzenie spełniające ten warunek SHALL być oznaczone jako rozwiązujące problem o swoim fingerprincie.

#### Scenario: Alertmanager status resolved
- **GIVEN** mapping z warunkiem `resolved: status == "resolved"`
- **WHEN** przychodzi zdarzenie Alertmanagera ze `status = resolved`
- **THEN** zdarzenie kanoniczne ma `resolved = true` i ten sam fingerprint co odpowiadające mu zdarzenie `firing`

### Requirement: Zdarzenia bez mappingu (UNMAPPED)
System SHALL zapisać zdarzenie, dla którego `schema` nie ma mappingu, jako UNMAPPED, widoczne w UI z liczbą wystąpień i przykładowym payloadem; takie zdarzenie SHALL NOT zostać utracone.

#### Scenario: Nieznany schemat od jury
- **WHEN** przychodzi zdarzenie ze `schema = judges.custom.v1`, dla którego nie ma mappingu
- **THEN** zdarzenie pojawia się na liście UNMAPPED w UI, a ingest nie zwraca błędu

### Requirement: Ponowne przetworzenie po dodaniu mappingu
Po dodaniu mappingu dla schematu oznaczonego jako UNMAPPED system SHALL ponownie przetworzyć zapisane zdarzenia tego schematu tak, jakby właśnie przyszły.

#### Scenario: Utworzenie mappingu z UI
- **GIVEN** 3 zdarzenia UNMAPPED ze `schema = judges.custom.v1`
- **WHEN** operator zapisuje mapping dla `judges.custom.v1`
- **THEN** wszystkie 3 zdarzenia są normalizowane i przechodzą przez reguły, a lista UNMAPPED dla tego schematu jest pusta

### Requirement: Zmiana mappingu w runtime
System SHALL stosować nowy lub zmieniony mapping do kolejnych zdarzeń bez restartu.

#### Scenario: Edycja mappingu
- **WHEN** operator zmienia ścieżkę `title` w mappingu
- **THEN** kolejne zdarzenia tego schematu mają tytuł wg nowej ścieżki, bez restartu systemu
