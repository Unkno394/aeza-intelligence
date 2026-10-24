CREATE SCHEMA IF NOT EXISTS market_intelligence;

-- Reserved for a source that supplies region-level interest. The current
-- TrendsMCP collector does not populate this table.
CREATE TABLE IF NOT EXISTS market_intelligence.search_interest_geo (
    brand_code text NOT NULL REFERENCES market_intelligence.search_brands(code),
    source text NOT NULL,
    region_code text NOT NULL,
    region_name text NOT NULL,
    latitude double precision CHECK (latitude BETWEEN -90 AND 90),
    longitude double precision CHECK (longitude BETWEEN -180 AND 180),
    period_start date NOT NULL,
    period_end date NOT NULL CHECK (period_end >= period_start),
    score numeric(8,3) NOT NULL CHECK (score BETWEEN 0 AND 100),
    collected_at timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (brand_code, source, region_code, period_start, period_end)
);

ALTER TABLE market_intelligence.search_interest_geo
    ALTER COLUMN latitude DROP NOT NULL,
    ALTER COLUMN longitude DROP NOT NULL;

CREATE INDEX IF NOT EXISTS search_interest_geo_recent_idx
    ON market_intelligence.search_interest_geo (brand_code, period_end DESC);
