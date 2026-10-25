import test from 'node:test';
import assert from 'node:assert/strict';
import { filterTariffs, paginateTariffs, type TariffFilterRow } from './tariffFilters';

type Tariff = TariffFilterRow & { priceRub: number };
const rows: Tariff[] = [
  { providerCode: 'aeza', provider: 'Aéza', planKey: 'basic-2', planName: 'Basic VPS', region: 'DE-FRA', priceRub: 900 },
  { providerCode: 'contabo', provider: 'Contabo', planKey: 'cloud-2', planName: 'Cloud VPS', region: 'DE-FRA', priceRub: 1000 },
  { providerCode: 'hetzner', provider: 'Hetzner', planKey: 'cx22', planName: 'CX22', region: 'US-CLT', priceRub: 800 },
  { providerCode: 'timeweb_cloud', provider: 'Timeweb Cloud', planKey: 'ru-1', planName: 'VDS 1', region: null, priceRub: 700 },
];

const all = { provider: 'all', region: 'all', onlyAeza: false, query: '' };

test('provider and region filters combine without widening results', () => {
  assert.deepEqual(
    filterTariffs(rows, { ...all, provider: 'contabo', region: 'DE-FRA' }).map(row => row.planKey),
    ['cloud-2'],
  );
  assert.deepEqual(filterTariffs(rows, { ...all, provider: 'contabo', region: 'US-CLT' }), []);
});

test('unknown-region filter returns only offers whose region is actually missing', () => {
  assert.deepEqual(
    filterTariffs(rows, { ...all, region: '__unknown__' }).map(row => row.providerCode),
    ['timeweb_cloud'],
  );
});

test('search is case-insensitive across provider, tariff, region, and plan key', () => {
  for (const query of ['HETZNER', 'cx22', 'us-clt']) {
    assert.deepEqual(filterTariffs(rows, { ...all, query }).map(row => row.planKey), ['cx22']);
  }
  assert.deepEqual(filterTariffs(rows, { ...all, query: 'missing' }), []);
});

test('Aéza-only filter composes with search and region filters', () => {
  assert.deepEqual(
    filterTariffs(rows, { ...all, onlyAeza: true, query: 'basic', region: 'DE-FRA' }).map(row => row.planKey),
    ['basic-2'],
  );
  assert.deepEqual(filterTariffs(rows, { ...all, onlyAeza: true, query: 'cloud' }), []);
});

test('pagination clamps stale page numbers and preserves an explicit empty state', () => {
  const second = paginateTariffs(rows, 1, 3);
  assert.equal(second.pageCount, 2);
  assert.equal(second.safePage, 1);
  assert.deepEqual(second.visibleRows.map(row => row.planKey), ['ru-1']);
  const stale = paginateTariffs(rows, 50, 3);
  assert.equal(stale.safePage, 1);
  assert.deepEqual(stale.visibleRows.map(row => row.planKey), ['ru-1']);
  assert.deepEqual(paginateTariffs([], 0, 20), { pageCount: 1, safePage: 0, visibleRows: [] });
});
