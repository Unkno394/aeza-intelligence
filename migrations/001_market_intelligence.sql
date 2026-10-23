-- Run in the existing Aéza PostgreSQL database.
-- Competitor data lives in its own schema, not in a separate database.
BEGIN;

CREATE SCHEMA IF NOT EXISTS market_intelligence;

CREATE TABLE IF NOT EXISTS market_intelligence.providers (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    code text NOT NULL UNIQUE,
    name text NOT NULL,
    website_url text NOT NULL,
    tariff_page_urls text[] NOT NULL DEFAULT '{}',
    created_at timestamptz NOT NULL DEFAULT now(),
    CHECK (code <> ''),
    CHECK (name <> '')
);

INSERT INTO market_intelligence.providers
    (code, name, website_url, tariff_page_urls)
VALUES
    ('firstvds', 'FirstVDS', 'https://firstvds.ru',
     ARRAY['https://firstvds.ru/docs/tariffs.html']),
    ('timeweb_cloud', 'Timeweb Cloud', 'https://timeweb.cloud',
     ARRAY['https://timeweb.cloud/services/vds-vps']),
    ('selectel', 'Selectel', 'https://selectel.ru',
     ARRAY['https://selectel.ru/services/cloud/vps-vds/']),
    ('hetzner', 'Hetzner', 'https://www.hetzner.com',
     ARRAY['https://www.hetzner.com/cloud/cost-optimized/',
           'https://www.hetzner.com/cloud/regular-performance/',
           'https://www.hetzner.com/cloud/general-purpose/'])
ON CONFLICT (code) DO NOTHING;

-- One row describes one version of a plan's characteristics. If the provider
-- changes CPU, RAM, disk or region, insert a new fingerprint and preserve the
-- old row so past prices keep pointing to the correct configuration.
CREATE TABLE IF NOT EXISTS market_intelligence.provider_tariffs (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    provider_id bigint NOT NULL
        REFERENCES market_intelligence.providers(id),
    plan_key text NOT NULL,
    configuration_fingerprint text NOT NULL,
    plan_name text NOT NULL,
    cpu_cores numeric(8, 2),
    ram_mib integer,
    disk_gib numeric(12, 2),
    disk_type text,
    region text,
    attributes jsonb NOT NULL DEFAULT '{}'::jsonb,
    first_observed_at timestamptz NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now(),
    UNIQUE (provider_id, plan_key, configuration_fingerprint),
    CHECK (plan_key <> ''),
    CHECK (configuration_fingerprint <> ''),
    CHECK (plan_name <> ''),
    CHECK (jsonb_typeof(attributes) = 'object'),
    CHECK (cpu_cores IS NULL OR cpu_cores > 0),
    CHECK (ram_mib IS NULL OR ram_mib > 0),
    CHECK (disk_gib IS NULL OR disk_gib > 0)
);

-- Each row is an observation. Historical rows are never replaced when a new
-- year is imported. observed_at is the Wayback capture time for archive rows
-- and the crawl time for live rows; it is not necessarily the effective date.
CREATE TABLE IF NOT EXISTS market_intelligence.tariff_prices (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    provider_tariff_id bigint NOT NULL
        REFERENCES market_intelligence.provider_tariffs(id),
    observation_key text NOT NULL,
    source_kind text NOT NULL
        CHECK (source_kind IN ('archive', 'live')),
    observed_at timestamptz NOT NULL,
    retrieved_at timestamptz NOT NULL DEFAULT now(),
    effective_at timestamptz,
    price_minor bigint,
    currency char(3),
    billing_period text,
    price_kind text NOT NULL DEFAULT 'regular',
    vat_included boolean,
    source_url text NOT NULL,
    archive_url text,
    cdx_digest text,
    parser_version text NOT NULL,
    parse_status text NOT NULL
        CHECK (parse_status IN ('complete', 'incomplete')),
    raw_excerpt text,
    UNIQUE (provider_tariff_id, observation_key),
    CHECK (observation_key <> ''),
    CHECK (source_url <> ''),
    CHECK (parser_version <> ''),
    CHECK (price_minor IS NULL OR price_minor >= 0),
    CHECK (currency IS NULL OR currency ~ '^[A-Z]{3}$'),
    CHECK (billing_period IS NULL OR billing_period <> ''),
    CHECK (source_kind <> 'archive' OR archive_url IS NOT NULL),
    CHECK (parse_status <> 'complete'
           OR (price_minor IS NOT NULL
               AND currency IS NOT NULL
               AND billing_period IS NOT NULL))
);

CREATE INDEX IF NOT EXISTS tariff_prices_history_idx
    ON market_intelligence.tariff_prices
    (provider_tariff_id, observed_at DESC);

COMMIT;
