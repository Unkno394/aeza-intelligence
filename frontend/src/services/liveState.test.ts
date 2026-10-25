import assert from 'node:assert/strict';
import test from 'node:test';
import { defaults } from '../mocks/data';
import type { Dataset } from '../types';
import { type AccountRisk, type ServerMessage } from './live';
import { applyLiveMessage, emptyAnalyzerLiveState } from './liveState';

const dataset = (ids: string[]): Dataset => ({
  accounts: ids.map(id => ({ id } as Dataset['accounts'][number])),
  services: [],
  transactions: [],
  asOf: '2026-10-25T00:00:00.000Z',
  snapshots: [],
});

const risk = (accountId: string, score: number): AccountRisk => ({
  accountId,
  score,
} as AccountRisk);

test('live state replaces datasets and applies changed and removed risks without a page reload', () => {
  const metadata = { version: 1, updatedAt: null, updatedBy: 'operator' };
  const snapshot: ServerMessage = {
    type: 'snapshot',
    dataset: dataset(['1', '2']),
    settings: defaults,
    settingsMetadata: metadata,
    risks: [risk('1', 10), risk('2', 20)],
  };
  let state = applyLiveMessage(emptyAnalyzerLiveState, snapshot);
  assert.deepEqual(state.data?.accounts.map(account => account.id), ['1', '2']);
  assert.deepEqual(Object.keys(state.risks ?? {}).sort(), ['1', '2']);

  state = applyLiveMessage(state, { type: 'dataset', dataset: dataset(['1', '3']) });
  assert.deepEqual(state.data?.accounts.map(account => account.id), ['1', '3']);
  assert.equal(state.risks?.['2']?.score, 20, 'dataset and score events can arrive independently');

  state = applyLiveMessage(state, {
    type: 'scores',
    changed: [risk('1', 80), risk('3', 30)],
    removed: ['2'],
  });
  assert.deepEqual(Object.keys(state.risks ?? {}).sort(), ['1', '3']);
  assert.equal(state.risks?.['1']?.score, 80);
  assert.equal(state.risks?.['3']?.score, 30);

  const settings = { ...defaults, mediumThreshold: defaults.mediumThreshold + 1 };
  state = applyLiveMessage(state, { type: 'settings', settings, settingsMetadata: { ...metadata, version: 2 } });
  assert.equal(state.settings?.mediumThreshold, settings.mediumThreshold);
  assert.equal(state.settingsMetadata?.version, 2);
});

test('settings acknowledgements do not mutate analyzer data state', () => {
  const state = applyLiveMessage(emptyAnalyzerLiveState, {
    type: 'ack',
    requestId: '1',
    settingsMetadata: { version: 1, updatedAt: null, updatedBy: 'operator' },
  });
  assert.strictEqual(state, emptyAnalyzerLiveState);
});
