import assert from 'node:assert/strict';
import { test } from 'node:test';
import { demoEvidenceReady, demoPriceRisk } from './demoPriceRisk';
import { demoPriceImpactEvents } from './priceImpact';
import type { RiskResult, Settings } from '../types';

const settings: Settings = {
  weights: { paymentCoverage: 35, noAutoProlong: 15, prolongWait: 15, failedProlong: 10, expiringRevenue: 25, priceRisk: 25 },
  mediumThreshold: 35, highThreshold: 65, renewalWindowDays: 14, historicalMinDays: 30,
};
const base: RiskResult = {
  score: 41, level: 'medium', assessmentStatus: 'sufficient_data', availableFactors: 2,
  factors: [
    { key: 'paymentCoverage', label: 'Покрытие', value: 0.5, available: true, severity: 0.5, points: 35, explanation: '' },
    { key: 'noAutoProlong', label: 'Автопродление', value: 0.2, available: true, severity: 0.2, points: 6, explanation: '' },
    { key: 'priceRisk', label: 'Ценовой риск', value: null, available: false, severity: 0, points: 0, explanation: '' },
  ],
  upcomingCents: 1000, monthlyCents: 1000, topupsCents: 0, spendableBalanceCents: 500, paymentCoverage: 0.5,
};

test('scenario price risk requires a cohort and changes only the isolated score', () => {
  const events = demoPriceImpactEvents();
  assert.equal(demoEvidenceReady(events), true);
  const adverse = events.find(event => event.currentPriceCents === 675 && event.renewalsAfter === 0)!;
  assert.equal(demoPriceRisk(base, settings, adverse, false), base);
  const scenario = demoPriceRisk(base, settings, adverse, true);
  assert.ok(scenario.score > base.score);
  const smallerIncrease = { ...adverse, currentPriceCents: 525 };
  assert.ok(scenario.score > demoPriceRisk(base, settings, smallerIncrease, true).score);
  assert.ok(demoPriceRisk(base, { ...settings, weights: { ...settings.weights, priceRisk: 40 } }, adverse, true).score > scenario.score);
  assert.equal(scenario.factors.find(factor => factor.key === 'priceRisk')?.available, true);
  assert.equal(base.factors.find(factor => factor.key === 'priceRisk')?.available, false);
  assert.equal(demoPriceRisk(base, { ...settings, weights: { ...settings.weights, priceRisk: 0 } }, adverse, true).score, base.score);
});

test('missing follow-up and missing event do not contribute', () => {
  const event = { ...demoPriceImpactEvents()[0], completeFollowup: false };
  assert.equal(demoPriceRisk(base, settings, event, true), base);
  assert.equal(demoPriceRisk(base, settings, undefined, true), base);
});
