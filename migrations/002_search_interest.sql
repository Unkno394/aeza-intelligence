-- Apply to the same PostgreSQL database as 001_market_intelligence.sql.
CREATE SCHEMA IF NOT EXISTS market_intelligence;

CREATE TABLE IF NOT EXISTS market_intelligence.search_brands (
    code text PRIMARY KEY,
    name text NOT NULL,
    keyword text NOT NULL,
    source text NOT NULL DEFAULT 'google search',
    enabled boolean NOT NULL DEFAULT true
);

INSERT INTO market_intelligence.search_brands (code, name, keyword) VALUES
    ('aeza', 'Aéza', 'Aeza'),
    ('hetzner', 'Hetzner', 'Hetzner'),
    ('contabo', 'Contabo', 'Contabo'),
    ('timeweb_cloud', 'Timeweb Cloud', 'Timeweb Cloud'),
    ('selectel', 'Selectel', 'Selectel'),
    ('firstvds', 'FirstVDS', 'FirstVDS')
ON CONFLICT (code) DO NOTHING;

CREATE TABLE IF NOT EXISTS market_intelligence.search_interest (
    brand_code text NOT NULL REFERENCES market_intelligence.search_brands(code),
    source text NOT NULL,
    observed_on date NOT NULL,
    score numeric(8, 3) NOT NULL CHECK (score >= 0 AND score <= 100),
    volume bigint CHECK (volume IS NULL OR volume >= 0),
    collected_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (brand_code, source, observed_on)
);

CREATE INDEX IF NOT EXISTS search_interest_recent_idx
    ON market_intelligence.search_interest (brand_code, observed_on DESC);

-- A score is only comparable with observations of the same keyword/source.
CREATE OR REPLACE VIEW market_intelligence.search_interest_changes AS
WITH latest AS (
    SELECT DISTINCT ON (brand_code, source)
        brand_code, source, observed_on, score
    FROM market_intelligence.search_interest
    ORDER BY brand_code, source, observed_on DESC
)
SELECT latest.brand_code, latest.source, latest.observed_on,
       latest.score AS latest_score,
       week.score AS score_7d_ago,
       month.score AS score_30d_ago,
       latest.score - week.score AS change_7d,
       latest.score - month.score AS change_30d
FROM latest
LEFT JOIN LATERAL (
    SELECT score FROM market_intelligence.search_interest AS point
    WHERE point.brand_code = latest.brand_code AND point.source = latest.source
      AND point.observed_on <= latest.observed_on - 7
    ORDER BY point.observed_on DESC LIMIT 1
) AS week ON true
LEFT JOIN LATERAL (
    SELECT score FROM market_intelligence.search_interest AS point
    WHERE point.brand_code = latest.brand_code AND point.source = latest.source
      AND point.observed_on <= latest.observed_on - 30
    ORDER BY point.observed_on DESC LIMIT 1
) AS month ON true;
