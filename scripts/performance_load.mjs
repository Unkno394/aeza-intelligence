#!/usr/bin/env node
import assert from 'node:assert/strict';
import {performance} from 'node:perf_hooks';

const base = new URL(process.env.PUBLIC_URL ?? 'http://127.0.0.1:8080');
const cookie = process.env.AEZA_LOAD_COOKIE;
const durationMs = Number(process.env.LOAD_DURATION_SECONDS ?? 45) * 1000;
const concurrency = Number(process.env.LOAD_CONCURRENCY ?? 8);
const requestTimeoutMs = Number(process.env.LOAD_REQUEST_TIMEOUT_MS ?? 10000);
const p95LimitMs = Number(process.env.LOAD_P95_LIMIT_MS ?? 5000);
const recalculationLimitMs = Number(process.env.LOAD_RECALC_LIMIT_MS ?? 30000);
assert.ok(cookie?.startsWith('aeza_session='), 'AEZA_LOAD_COOKIE must contain a test session');
assert.ok(Number.isFinite(durationMs) && durationMs >= 5000, 'LOAD_DURATION_SECONDS must be at least 5');
assert.ok(Number.isInteger(concurrency) && concurrency >= 1 && concurrency <= 64, 'LOAD_CONCURRENCY must be 1..64');

const samples = {recalculation: [], list: [], detail: [], summary: []};
const failures = [];
const settings = {
  weights: {paymentCoverage: 35, noAutoProlong: 15, prolongWait: 15, failedProlong: 10, expiringRevenue: 25, priceRisk: 0},
  mediumThreshold: 35, highThreshold: 65, renewalWindowDays: 14, historicalMinDays: 30,
};

async function measured(name, path, options = {}, timeoutMs = requestTimeoutMs) {
  const started = performance.now();
  const response = await fetch(new URL(path, base), {
    ...options,
    headers: {...options.headers, cookie},
    signal: AbortSignal.timeout(timeoutMs),
  });
  const elapsed = performance.now() - started;
  samples[name].push(elapsed);
  if (!response.ok) throw new Error(`${name} returned HTTP ${response.status}`);
  return response.json();
}

const recalculationStarted = performance.now();
const recalculated = await measured('recalculation', '/api/client/snapshot', {
  method: 'POST',
  headers: {'content-type': 'application/json'},
  body: JSON.stringify(settings),
}, recalculationLimitMs);
assert.ok(Array.isArray(recalculated) && recalculated.length > 0, 'full Risk Score recalculation returned no customers');
const recalculationMs = performance.now() - recalculationStarted;
if (recalculationMs > recalculationLimitMs) {
  failures.push(`full recalculation exceeded ${recalculationLimitMs}ms`);
}

const stopAt = Date.now() + durationMs;
let stop = false;
async function worker(index) {
  let iteration = 0;
  while (!stop && Date.now() < stopAt) {
    try {
      const pageNumber = (index + iteration) % 10 + 1;
      const page = await measured('list', `/api/v1/customers?sort=id&direction=asc&page=${pageNumber}&limit=100`);
      assert.ok(Array.isArray(page.items) && page.total > 0, 'customer list returned an empty or malformed page');
      const accountId = page.items[0]?.account?.id;
      if (accountId !== undefined) await measured('detail', `/api/v1/customers/${encodeURIComponent(accountId)}`);
      if (iteration % 4 === 0) await measured('summary', '/api/v1/analytics/summary');
      iteration += 1;
    } catch (error) {
      failures.push(String(error));
      stop = true;
    }
  }
}

await Promise.all(Array.from({length: concurrency}, (_, index) => worker(index)));

const percentile = (values, ratio) => {
  if (!values.length) return null;
  const ordered = [...values].sort((left, right) => left - right);
  return Math.round(ordered[Math.min(ordered.length - 1, Math.ceil(ordered.length * ratio) - 1)]);
};
for (const name of ['list', 'detail', 'summary']) {
  const p95 = percentile(samples[name], 0.95);
  if (p95 !== null && p95 > p95LimitMs) failures.push(`${name} p95 ${p95}ms exceeded ${p95LimitMs}ms`);
}
const report = {
  durationSeconds: Number((durationMs / 1000).toFixed(1)),
  concurrency,
  fullRecalculationCustomers: recalculated.length,
  fullRecalculationMs: Math.round(recalculationMs),
  limits: {p95Ms: p95LimitMs, fullRecalculationMs: recalculationLimitMs},
  requests: Object.fromEntries(Object.entries(samples).map(([name, values]) => [name, {
    count: values.length,
    p50Ms: percentile(values, 0.50),
    p95Ms: percentile(values, 0.95),
    maxMs: values.length ? Math.round(Math.max(...values)) : null,
  }])),
  failures,
};
console.log(JSON.stringify(report, null, 2));
if (failures.length) process.exitCode = 1;
