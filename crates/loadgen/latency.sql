-- Latencja end-to-end i stan alertów po przebiegu loadgen (D14, sekcja 5.2).
-- Uruchomienie (okno czasowe `since` – liczymy tylko dostarczenia z received_at >= since):
--   docker exec -i gs-hack-postgres-1 psql -U alerting -d alerting -v ON_ERROR_STOP=1 \
--     -v since="'2026-11-21 02:00'" < crates/loadgen/latency.sql
-- Bez `-v since=...` domyślnie ostatnie 15 minut. `since` to wyrażenie SQL, więc stałą
-- datę podaj w apostrofach (jak wyżej); działa też np. -v since="now() - interval '5 minutes'".
-- Bez okna liczą się stare fixtures i ponowne wysyłki (p99 rzędu dni).
--
-- Kolumny `deliveries` potwierdzone z migrations/2001_p2_tables.sql (19.C2):
--   received_at  = received_at zdarzenia źródłowego (DeliveryRequest.received_at, ustawia router),
--   delivered_at = chwila potwierdzenia przez kanał (sender), status = contracts::DeliveryStatus.
-- Pomiar do kanału UI/mocków (ui, sms, phone), nie do Mailpit (żeby nie mierzyć SMTP).

\if :{?since}
\else
\set since 'now() - interval ''15 minutes'''
\endif
\echo 'Okno: received_at >= ' :since

-- 1) p50/p99 opóźnienia delivered_at - received_at w milisekundach (dostarczone, bez email).
SELECT
    count(*)                                                                     AS delivered,
    percentile_cont(0.50) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p50_ms,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p99_ms
FROM deliveries
WHERE status = 'delivered'
  AND channel <> 'email'
  AND delivered_at IS NOT NULL
  AND received_at IS NOT NULL
  AND received_at >= :since;

-- 1b) To samo per kanał (email dla porządku — zawiera czas SMTP).
SELECT
    channel,
    count(*)                                                                     AS delivered,
    percentile_cont(0.50) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p50_ms,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p99_ms
FROM deliveries
WHERE status = 'delivered'
  AND delivered_at IS NOT NULL
  AND received_at IS NOT NULL
  AND received_at >= :since
GROUP BY channel
ORDER BY channel;

-- 2) Po stronie P1: liczba alertów i suma liczników zdarzeń (czy dedup zliczył wszystko).
SELECT
    count(*)                       AS alerts,
    coalesce(sum(alerts.count), 0) AS events_counted
FROM alerts;
