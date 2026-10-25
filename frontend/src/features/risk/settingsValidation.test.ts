import assert from 'node:assert/strict';
import test from 'node:test';
import { defaults } from '../../mocks/data';
import { validSettings } from './settingsValidation';

test('recommended settings are valid and price risk may stay disabled', () => {
  assert.equal(validSettings(defaults), true);
  assert.equal(validSettings({ ...defaults, weights: { ...defaults.weights, priceRisk: 0 } }), true);
});

test('all model weights must remain nonnegative bounded integers with a positive base weight', () => {
  for (const value of [-1, 101, 1.5, Number.NaN, Number.POSITIVE_INFINITY]) {
    for (const key of Object.keys(defaults.weights) as (keyof typeof defaults.weights)[]) {
      const candidate = { ...defaults, weights: { ...defaults.weights, [key]: value } };
      assert.equal(validSettings(candidate), false, `${key}=${value}`);
    }
  }
  const noBaseWeight = {
    ...defaults,
    weights: { paymentCoverage: 0, noAutoProlong: 0, prolongWait: 0, failedProlong: 0, expiringRevenue: 0, priceRisk: 100 },
  };
  assert.equal(validSettings(noBaseWeight), false);
});

test('thresholds and forecast windows reject inverted, fractional, and out-of-range values', () => {
  for (const candidate of [
    { ...defaults, mediumThreshold: 0 },
    { ...defaults, mediumThreshold: 35.5 },
    { ...defaults, highThreshold: defaults.mediumThreshold },
    { ...defaults, highThreshold: 101 },
    { ...defaults, renewalWindowDays: 0 },
    { ...defaults, renewalWindowDays: 91 },
    { ...defaults, historicalMinDays: 6 },
    { ...defaults, historicalMinDays: 366 },
  ]) assert.equal(validSettings(candidate), false, JSON.stringify(candidate));
});
