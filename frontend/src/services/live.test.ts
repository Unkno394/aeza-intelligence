import assert from 'node:assert/strict';
import test from 'node:test';
import { defaults } from '../mocks/data';
import type { Dataset, SettingsMetadata } from '../types';
import { LiveClient, type ServerMessage } from './live';

class FakeWebSocket {
  static OPEN = 1;
  static instances: FakeWebSocket[] = [];

  readyState = 0;
  onopen: (() => void) | null = null;
  onmessage: ((event: { data: string }) => void) | null = null;
  onclose: (() => void) | null = null;
  sent: string[] = [];

  constructor(readonly url: string) {
    FakeWebSocket.instances.push(this);
  }

  open() {
    this.readyState = FakeWebSocket.OPEN;
    this.onopen?.();
  }

  receive(message: unknown) {
    this.onmessage?.({ data: JSON.stringify(message) });
  }

  send(message: string) {
    this.sent.push(message);
  }

  close() {
    this.readyState = 3;
    this.onclose?.();
  }

  disconnectFromServer() {
    this.readyState = 3;
    this.onclose?.();
  }
}

test('live client handles updates, settings replies, reconnects, and stops cleanly', async () => {
  const originalWebSocket = Object.getOwnPropertyDescriptor(globalThis, 'WebSocket');
  const originalWindow = Object.getOwnPropertyDescriptor(globalThis, 'window');
  const originalSetTimeout = globalThis.setTimeout;
  const originalClearTimeout = globalThis.clearTimeout;
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  const setTimer = ((callback: () => void) => {
    const id = ++nextTimer;
    timers.set(id, callback);
    return id;
  }) as typeof setTimeout;
  const clearTimer = ((id?: ReturnType<typeof setTimeout>) => {
    if (typeof id === 'number') timers.delete(id);
  }) as typeof clearTimeout;

  FakeWebSocket.instances = [];
  Object.defineProperty(globalThis, 'WebSocket', { configurable: true, value: FakeWebSocket });
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: { setTimeout: setTimer },
  });
  globalThis.setTimeout = setTimer;
  globalThis.clearTimeout = clearTimer;

  try {
    const statuses: string[] = [];
    const messages: ServerMessage[] = [];
    const client = new LiveClient('ws://example.test/api/client/ws', message => messages.push(message), status => statuses.push(status));
    client.start();
    const firstSocket = FakeWebSocket.instances[0];
    assert.equal(firstSocket.url, 'ws://example.test/api/client/ws');
    firstSocket.open();
    assert.deepEqual(statuses, ['connecting', 'open']);

    const dataset = {
      accounts: [], services: [], transactions: [], asOf: '2026-10-25T00:00:00.000Z', snapshots: [],
    } as Dataset;
    const metadata: SettingsMetadata = { version: 1, updatedAt: null, updatedBy: 'operator' };
    firstSocket.receive({ type: 'dataset', dataset });
    firstSocket.receive({ type: 'scores', changed: [], removed: ['old-account'] });
    assert.deepEqual(messages.map(message => message.type), ['dataset', 'scores']);
    assert.deepEqual(messages[0], { type: 'dataset', dataset });
    assert.deepEqual(messages[1], { type: 'scores', changed: [], removed: ['old-account'] });

    const settingsUpdate = client.updateSettings(defaults);
    const sent = JSON.parse(firstSocket.sent[0]);
    assert.deepEqual(sent, { type: 'updateSettings', requestId: '1', settings: defaults });
    firstSocket.receive({ type: 'ack', requestId: '1', settingsMetadata: metadata });
    assert.deepEqual(await settingsUpdate, metadata);

    const rejectedUpdate = client.updateSettings(defaults);
    firstSocket.receive({ type: 'error', requestId: '2', message: 'forbidden' });
    await assert.rejects(rejectedUpdate, /forbidden/);
    assert.equal(messages.length, 2, 'ack/error replies are not forwarded as data updates');

    firstSocket.disconnectFromServer();
    assert.equal(statuses.at(-1), 'closed');
    assert.equal(timers.size, 1, 'a dropped connection schedules one reconnect');
    const [timerId, reconnect] = timers.entries().next().value!;
    timers.delete(timerId);
    reconnect();
    assert.equal(FakeWebSocket.instances.length, 2);
    const secondSocket = FakeWebSocket.instances[1];
    secondSocket.open();
    secondSocket.receive({ type: 'snapshot', dataset, settings: defaults, settingsMetadata: metadata, risks: [] });
    assert.equal(messages.at(-1)?.type, 'snapshot');

    client.stop();
    secondSocket.disconnectFromServer();
    assert.equal(timers.size, 0, 'stopping clears pending reconnects');
    assert.equal(FakeWebSocket.instances.length, 2, 'a stopped client does not reconnect');
  } finally {
    globalThis.setTimeout = originalSetTimeout;
    globalThis.clearTimeout = originalClearTimeout;
    if (originalWebSocket) Object.defineProperty(globalThis, 'WebSocket', originalWebSocket);
    else Reflect.deleteProperty(globalThis, 'WebSocket');
    if (originalWindow) Object.defineProperty(globalThis, 'window', originalWindow);
    else Reflect.deleteProperty(globalThis, 'window');
  }
});
