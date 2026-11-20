-- Latencja end-to-end i stan alertów po przebiegu loadgen (D14, sekcja 5.2).
-- Uruchomienie:
--   docker compose exec -T postgres psql -U alerting -d alerting < crates/loadgen/latency.sql
--
-- UWAGA: tabela `deliveries` i jej kolumny (`delivered_at`, `received_at`, `status`)
-- należą do P2 (migracje 2xxx, crates/sender). Nazwy trzeba potwierdzić przy integracji (18.1).
-- Pomiar do kanału UI/mocków, nie do Mailpit (żeby nie mierzyć SMTP).

-- 1) p50/p99 opóźnienia delivered_at - received_at w milisekundach (tylko dostarczone).
SELECT
    count(*)                                                                     AS delivered,
    percentile_cont(0.50) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p50_ms,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p99_ms
FROM deliveries
WHERE status = 'delivered'
  AND delivered_at IS NOT NULL
  AND received_at IS NOT NULL;

-- 2) Po stronie P1: liczba alertów i suma liczników zdarzeń (czy dedup zliczył wszystko).
SELECT
    count(*)                       AS alerts,
    coalesce(sum(alerts.count), 0) AS events_counted
FROM alerts;
