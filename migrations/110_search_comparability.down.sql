BEGIN;
DROP VIEW IF EXISTS market_intelligence.search_interest_changes;
ALTER TABLE market_intelligence.search_interest DROP COLUMN IF EXISTS comparison_group;
CREATE OR REPLACE VIEW market_intelligence.search_interest_changes AS
WITH latest AS (
    SELECT DISTINCT ON (brand_code, source) brand_code, source, observed_on, score
    FROM market_intelligence.search_interest
    ORDER BY brand_code, source, observed_on DESC
)
SELECT latest.brand_code, latest.source, latest.observed_on,
       latest.score AS latest_score, week.score AS score_7d_ago,
       month.score AS score_30d_ago, latest.score - week.score AS change_7d,
       latest.score - month.score AS change_30d
FROM latest
LEFT JOIN LATERAL (
    SELECT score FROM market_intelligence.search_interest point
    WHERE point.brand_code = latest.brand_code AND point.source = latest.source
      AND point.observed_on <= latest.observed_on - 7
    ORDER BY point.observed_on DESC LIMIT 1
) week ON true
LEFT JOIN LATERAL (
    SELECT score FROM market_intelligence.search_interest point
    WHERE point.brand_code = latest.brand_code AND point.source = latest.source
      AND point.observed_on <= latest.observed_on - 30
    ORDER BY point.observed_on DESC LIMIT 1
) month ON true;
COMMIT;
