import assert from 'node:assert/strict';
import test from 'node:test';
import { needsPriceReview, priceEvidence, priceOutcome, summarizePriceBands, summarizePriceImpact, type PriceImpactEvent } from './priceImpact';

const event = (overrides: Partial<PriceImpactEvent> = {}): PriceImpactEvent => ({
  eventId: '1', serviceId: '1', accountId: '1', accountEmail: 'client@example.test',
  planKey: null, planName: 'TEST-1', changedAt: '2026-08-01T00:00:00Z', previousPriceCents: 1000,
  currentPriceCents: 1200, nextRenewalAt: '2026-08-15T00:00:00Z', paymentTerm: 'month',
  completeBaseline: true, completePretrend: true, completeFollowup: true, renewalDueInWindow: true,
  currentStatus: 'active', currentAutoProlong: true, renewalsEarlier: 1, renewalsBefore: 1, renewalsAfter: 0,
  prolongWaitAfter: false, autoRenewDisabledAfter: false, inactiveAfter: false,
  baselineServicesCount: 3, followupServicesCount: 2, baselineMonthlySpend: 3000,
  followupMonthlySpend: 2500, accountSpendEarlierCents: 3000, accountSpendBeforeCents: 3000, accountSpendAfterCents: 2000,
  controlAccountId: null, controlServicesBefore: null, controlServicesAfter: null,
  controlSpendBeforeCents: null, controlSpendAfterCents: null, controlRenewedAfter: null,
  controlRenewalsEarlier: null, controlRenewalsBefore: null, controlSpendEarlierCents: null,
  ...overrides,
});

test('price cohorts count each account once and omit missing snapshots from contraction rates', () => {
  const cohorts = summarizePriceBands([
    event({ eventId: 'new', serviceId: '2', currentPriceCents: 1300, changedAt: '2026-08-02T00:00:00Z' }),
    event({ eventId: 'old', serviceId: '1', currentPriceCents: 1050 }),
    event({ eventId: 'other', accountId: '2', serviceId: '3', currentPriceCents: 1100,
      baselineServicesCount: null, followupServicesCount: null, accountSpendBeforeCents: 0 }),
  ]);
  assert.deepEqual(cohorts.map(cohort => cohort.eligible), [0, 1, 1]);
  assert.equal(cohorts[2].infrastructureObserved, 1);
  assert.equal(cohorts[1].infrastructureObserved, 0);
  assert.equal(cohorts[1].paymentsObserved, 0);
});

test('repeated increases on one service contribute only the latest eligible event', () => {
  const rows = [
    event({ eventId: 'older', changedAt: '2026-07-01T00:00:00Z', currentPriceCents: 1050 }),
    event({ eventId: 'latest', changedAt: '2026-08-01T00:00:00Z', currentPriceCents: 1300 }),
  ];
  const bands = summarizePriceBands(rows);
  assert.deepEqual(bands.map(band => band.eligible), [0, 0, 1]);
  assert.equal(bands[2].renewalsBeforeTotal, 1);
});

test('price evidence needs large matched cohorts and a repeated direction', () => {
  const daysAgo = (days: number) => new Date(Date.now() - days * 86_400_000).toISOString();
  const rows: PriceImpactEvent[] = [];
  for (let period = 0; period < 2; period++) for (let band = 0; band < 3; band++) for (let i = 0; i < 100; i++) {
    const renewed = i < 80 - band * 15;
    const id = `${period}-${band}-${i}`;
    rows.push(event({ eventId: id, accountId: id, serviceId: id,
      changedAt: daysAgo(period === 0 ? 270 : 75), currentPriceCents: [1050, 1150, 1300][band],
      renewalsAfter: Number(renewed), autoRenewDisabledAfter: !renewed,
      controlAccountId: `control-${id}`, controlRenewedAfter: i < 80,
      controlSpendBeforeCents: 3000, controlSpendAfterCents: 3000,
      controlSpendEarlierCents: 3000, controlRenewalsEarlier: 1, controlRenewalsBefore: 1,
      baselineMonthlySpend: i % 2 ? 3500 : 2500 }));
  }
  assert.equal(priceEvidence(rows).stable, true);
  assert.equal(priceEvidence(rows.slice(1)).sufficient, false);
  assert.equal(priceEvidence(rows.map(row => row.eventId.startsWith('0-2-')
    ? { ...row, renewalsAfter: 1, autoRenewDisabledAfter: false } : row)).stable, false);
});

test('missing renewal without another event remains unknown', () => {
  assert.equal(priceOutcome(event()), 'unknown');
  assert.equal(priceOutcome(event({ autoRenewDisabledAfter: true })), 'adverse');
  assert.equal(priceOutcome(event({ autoRenewDisabledAfter: true, renewalsAfter: 1 })), 'renewed');
});

test('incomplete windows do not enter the descriptive rate', () => {
  const result = summarizePriceImpact([
    event({ eventId: '1', renewalsAfter: 1 }),
    event({ eventId: '2', autoRenewDisabledAfter: true }),
    event({ eventId: '3' }),
    event({ eventId: '4', completeFollowup: false, autoRenewDisabledAfter: true }),
  ]);
  assert.deepEqual([result.eligible, result.renewed, result.adverse, result.unknown, result.known], [3, 1, 1, 1, 2]);
  assert.equal(result.quality, 'insufficient');
});

test('review requires a recent increase and a nearby renewal', () => {
  const now = Date.parse('2026-10-01T00:00:00Z');
  const recent = event({ changedAt: '2026-09-15T00:00:00Z', nextRenewalAt: '2026-10-15T00:00:00Z' });
  assert.equal(needsPriceReview(recent, now), true);
  assert.equal(needsPriceReview({ ...recent, renewalsAfter: 1 }, now), false);
  assert.equal(needsPriceReview({ ...recent, nextRenewalAt: '2026-12-15T00:00:00Z' }, now), false);
});
