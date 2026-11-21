-- Latencja end-to-end i stan alertów po przebiegu loadgen (D14, sekcja 5.2).
-- Uruchomienie:
--   docker compose exec -T postgres psql -U alerting -d alerting < crates/loadgen/latency.sql
--
-- Kolumny `deliveries` potwierdzone z migrations/2001_p2_tables.sql (19.C2):
--   received_at  = received_at zdarzenia źródłowego (DeliveryRequest.received_at, ustawia router),
--   delivered_at = chwila potwierdzenia przez kanał (sender), status = contracts::DeliveryStatus.
-- Pomiar do kanału UI/mocków (ui, sms, phone), nie do Mailpit (żeby nie mierzyć SMTP).

-- 1) p50/p99 opóźnienia delivered_at - received_at w milisekundach (dostarczone, bez email).
SELECT
    count(*)                                                                     AS delivered,
    percentile_cont(0.50) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p50_ms,
    percentile_cont(0.99) WITHIN GROUP (ORDER BY extract(epoch FROM (delivered_at - received_at)) * 1000) AS p99_ms
FROM deliveries
WHERE status = 'delivered'
  AND channel <> 'email'
  AND delivered_at IS NOT NULL
  AND received_at IS NOT NULL;

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
GROUP BY channel
ORDER BY channel;

-- 2) Po stronie P1: liczba alertów i suma liczników zdarzeń (czy dedup zliczył wszystko).
SELECT
    count(*)                       AS alerts,
    coalesce(sum(alerts.count), 0) AS events_counted
FROM alerts;
