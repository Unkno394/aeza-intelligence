import assert from 'node:assert/strict';
import test from 'node:test';
import type { RiskResult, Settings } from '../../types';
import { previewRiskDistribution, previewRiskResult } from './settingsPreview';

const settings: Settings = {
  weights: { paymentCoverage: 35, noAutoProlong: 15, prolongWait: 15, failedProlong: 10, expiringRevenue: 25, priceRisk: 0 },
  mediumThreshold: 35, highThreshold: 65, renewalWindowDays: 14, historicalMinDays: 30,
};
const risk: RiskResult = {
  score: 30, level: 'low', assessmentStatus: 'sufficient_data', availableFactors: 3,
  factors: [
    { key: 'paymentCoverage', label: 'Payment', severity: 0, points: 0, value: 1, available: true, explanation: '' },
    { key: 'noAutoProlong', label: 'Auto renew', severity: 1, points: 15, value: 1, available: true, explanation: '' },
    { key: 'expiringRevenue', label: 'Expiring', severity: 1, points: 25, value: 1, available: true, explanation: '' },
    { key: 'prolongWait', label: 'Waiting', severity: 0, points: 0, value: null, available: false, explanation: '' },
    { key: 'failedProlong', label: 'Failed', severity: 0, points: 0, value: null, available: false, explanation: '' },
  ],
  upcomingCents: 0, monthlyCents: 0, topupsCents: 0, spendableBalanceCents: 0, paymentCoverage: 1,
};

test('settings preview reweights available factors and uses draft thresholds', () => {
  const draft = { ...settings, weights: { ...settings.weights, paymentCoverage: 0, noAutoProlong: 100, expiringRevenue: 0 }, mediumThreshold: 40, highThreshold: 90 };
  const preview = previewRiskResult(risk, draft);
  assert.equal(preview.score, 100);
  assert.equal(preview.level, 'high');
});

test('settings preview preserves limited-data classifications', () => {
  const limited = { ...risk, assessmentStatus: 'limited_data' as const, level: 'limited' as const };
  const distribution = previewRiskDistribution({ a: limited }, { ...settings, highThreshold: 1 });
  assert.equal(distribution.limited, 1);
  assert.equal(distribution.high, 0);
});
