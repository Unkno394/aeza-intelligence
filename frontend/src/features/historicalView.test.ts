import assert from 'node:assert/strict';
import test from 'node:test';
import { historyInPeriod, selectedHistoryPoint } from './historicalView';

const points = Array.from({ length: 121 }, (_, index) => ({
  observedOn: new Date(Date.UTC(2026, 0, 1 + index)).toISOString().slice(0, 10),
  score: index,
}));

test('7, 30, and 90 day views use the latest recorded day as their inclusive boundary', () => {
  for (const [days, expectedCount] of [[7, 8], [30, 31], [90, 91]] as const) {
    const visible = historyInPeriod(points, days);
    assert.equal(visible.length, expectedCount);
    assert.equal(visible.at(-1)?.observedOn, points.at(-1)?.observedOn);
    assert.equal(visible[0]?.score, points.length - expectedCount);
  }
});

test('a selected date is retained inside the period and falls back to its latest date outside it', () => {
  const visible = historyInPeriod(points, 7);
  assert.equal(selectedHistoryPoint(points, visible.map(point => point.observedOn), points.at(-3)!.observedOn)?.score, 118);
  assert.equal(selectedHistoryPoint(points, visible.map(point => point.observedOn), points[0]!.observedOn)?.score, 120);
});

test('empty history and a period with no visible observations have no selected date', () => {
  assert.deepEqual(historyInPeriod([], 90), []);
  assert.equal(selectedHistoryPoint(points, [], points[0]!.observedOn), undefined);
});
