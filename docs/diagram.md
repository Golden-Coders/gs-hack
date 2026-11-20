# Diagram architektury

Diagram odtworzony ze zdjęcia tablicy (`917423941_1492683679378398_2501446404069057702_n.jpg`).

```mermaid
flowchart LR
    %% Źródła powiadomień
    N((Notification)) --> HTTP([HTTP])
    N --> API([API's])
    N --> SMTP([SMTP])
    N -.-> MORE([...])

    HTTP --> U[Notification Unifier]
    API --> U
    SMTP --> U
    MORE -.-> U

    U -- "Message JSON" --> RS[Rule set]
    RS -- "DATA RULES" --> RB{RUN BOOK}
    RB --> AS[Action Steps]
    RB --> S[Sender]

    %% Obsługa błędów
    S -- Failure --> FH[Fail handler]
    FH -- RETRY --> S

    %% Kanały wysyłki
    S -- RULE --> SMS([SMS])
    S -- RULE --> EMAIL([Email])
    S -- RULE --> PHONE([Phone])

    %% Persystencja i UI
    S -- "*" --> DB[(DB)]
    DB --> UI[UI]
    UI --> DB
```

## Otwarte pytania (z tablicy)

1. Jak przychodzą Notifications? (chyba tak)
2. Czy w wiadomości jest informacja, czy coś jest pilne?

## Status (z tablicy)

- [x] Did 1
- [x] Did 2
- [ ] Doing 3
