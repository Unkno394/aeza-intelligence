BEGIN;
ALTER TABLE market_intelligence.search_interest
    ADD COLUMN IF NOT EXISTS comparison_group text NOT NULL DEFAULT 'legacy-unverified';

-- Existing per-brand TrendsMCP series were requested independently. Do not imply
-- comparability just because they share an API source label.
UPDATE market_intelligence.search_interest
SET comparison_group = 'trendsmcp:' || brand_code
WHERE source = 'google search' AND comparison_group = 'legacy-unverified';

-- The five-brand Google Trends CSV/Explore import uses one shared 0–100 scale.
UPDATE market_intelligence.search_interest
SET comparison_group = 'google-trends-joint:5y'
WHERE source = 'google trends compare 5y'
  AND comparison_group = 'legacy-unverified';

DROP VIEW IF EXISTS market_intelligence.search_interest_changes;
CREATE VIEW market_intelligence.search_interest_changes AS
WITH latest AS (
    SELECT DISTINCT ON (brand_code, source, comparison_group)
        brand_code, source, comparison_group, observed_on, score
    FROM market_intelligence.search_interest
    ORDER BY brand_code, source, comparison_group, observed_on DESC
)
SELECT latest.brand_code, latest.source, latest.comparison_group,
       latest.observed_on, latest.score AS latest_score,
       week.score AS score_7d_ago, month.score AS score_30d_ago,
       latest.score - week.score AS change_7d,
       latest.score - month.score AS change_30d
FROM latest
LEFT JOIN LATERAL (
    SELECT score FROM market_intelligence.search_interest point
    WHERE point.brand_code = latest.brand_code AND point.source = latest.source
      AND point.comparison_group = latest.comparison_group
      AND point.observed_on <= latest.observed_on - 7
    ORDER BY point.observed_on DESC LIMIT 1
) week ON true
LEFT JOIN LATERAL (
    SELECT score FROM market_intelligence.search_interest point
    WHERE point.brand_code = latest.brand_code AND point.source = latest.source
      AND point.comparison_group = latest.comparison_group
      AND point.observed_on <= latest.observed_on - 30
    ORDER BY point.observed_on DESC LIMIT 1
) month ON true;
COMMIT;
