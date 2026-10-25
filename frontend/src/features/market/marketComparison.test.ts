import test from 'node:test';
import assert from 'node:assert/strict';
import { compareCurrentOffers, type MarketOffer } from './marketComparison';

const aeza: MarketOffer = {
  providerCode: 'aeza',
  cpuCores: 2,
  ramGib: 8,
  diskGib: 80,
  diskType: 'NVMe',
  region: 'DE-FRA',
  vatIncluded: true,
  trafficGib: null,
  trafficUnlimited: true,
  priceRub: 1000,
  priceKind: 'regular',
};

const offer = (overrides: Partial<MarketOffer> = {}): MarketOffer => ({
  ...aeza,
  providerCode: 'contabo',
  priceRub: 1100,
  ...overrides,
});

test('strict exact comparison requires the same known resources and commercial terms', () => {
  const [result] = compareCurrentOffers(aeza, [offer()]);
  assert.equal(result.tier, 'exact');
  assert.deepEqual(result.reasons, []);
  assert.ok(Math.abs(result.differencePercent - 10) < 1e-9);
});

test('provider self-comparisons and non-positive prices are excluded', () => {
  assert.deepEqual(compareCurrentOffers(aeza, [offer({ providerCode: 'aeza' })]), []);
  assert.deepEqual(compareCurrentOffers(aeza, [offer({ priceRub: 0 })]), []);
  assert.deepEqual(compareCurrentOffers({ ...aeza, priceRub: null }, [offer()]), []);
});

test('known region, disk, VAT, traffic, and price-term differences are made explicit', () => {
  const rows = compareCurrentOffers(aeza, [offer({
    region: 'GB-LON',
    diskType: 'SSD',
    vatIncluded: false,
    trafficGib: 1024,
    trafficUnlimited: false,
    priceKind: 'promotion',
  })]);
  assert.equal(rows[0].tier, 'reference');
  for (const reason of ['другой регион', 'другой тип диска', 'условия НДС отличаются', 'лимит трафика отличается', 'особые условия цены']) {
    assert.ok(rows[0].reasons.includes(reason), `missing comparison caveat: ${reason}`);
  }
});

test('unknown configuration details remain references rather than exact matches', () => {
  const [unknown] = compareCurrentOffers(aeza, [offer({
    cpuCores: null,
    ramGib: null,
    diskGib: null,
    diskType: null,
    region: null,
    vatIncluded: null,
    trafficGib: null,
    trafficUnlimited: null,
  })]);
  assert.equal(unknown.tier, 'reference');
  assert.ok(unknown.reasons.includes('характеристики неполные'));
  assert.ok(unknown.reasons.includes('регион не указан'));
  assert.ok(unknown.reasons.includes('НДС не уточнён'));
  assert.ok(unknown.reasons.includes('трафик не уточнён'));
});

test('nearby configurations are ranked by tier, resource distance, then price', () => {
  const exact = offer();
  const close = offer({ cpuCores: 4, ramGib: 8, diskGib: 80, priceRub: 900 });
  const reference = offer({ region: 'US-CLT', priceRub: 500 });
  const rows = compareCurrentOffers(aeza, [reference, close, exact]);
  assert.deepEqual(rows.map(row => row.tier), ['exact', 'close', 'reference']);
  assert.equal(rows[0].offer, exact);
  assert.equal(rows[1].offer, close);
  assert.equal(rows[2].offer, reference);
});
