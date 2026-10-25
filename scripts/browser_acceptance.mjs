#!/usr/bin/env node
// Browser acceptance checks for a disposable running Compose stack.
// Requires Chromium and Node 22+ (native WebSocket/fetch); no npm dependencies.
import assert from 'node:assert/strict';
import {mkdtemp, mkdir, readdir, readFile, rm} from 'node:fs/promises';
import net from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';

const base = new URL(process.env.PUBLIC_URL ?? 'http://127.0.0.1:8080');
const username = process.env.E2E_USERNAME;
const password = process.env.E2E_PASSWORD;
const chromium = process.env.BROWSER_BIN ?? 'chromium';
assert.ok(username && password, 'Set E2E_USERNAME and E2E_PASSWORD to an active test admin');

const freePort = () => new Promise((resolve, reject) => {
  const server = net.createServer();
  server.once('error', reject);
  server.listen(0, '127.0.0.1', () => {
    const port = server.address().port;
    server.close(error => error ? reject(error) : resolve(port));
  });
});
const profile = await mkdtemp(path.join(os.tmpdir(), 'aeza-browser-'));
const debuggingPort = await freePort();
const child = spawn(chromium, [
  '--headless=new', '--no-sandbox', '--disable-dev-shm-usage', '--disable-gpu',
  '--no-first-run', '--no-default-browser-check', '--remote-allow-origins=*',
  `--remote-debugging-port=${debuggingPort}`, `--user-data-dir=${profile}`, 'about:blank',
], {stdio: ['ignore', 'ignore', 'pipe']});
let chromeLog = '';
child.stderr.on('data', chunk => { chromeLog = (chromeLog + chunk.toString()).slice(-12000); });

let socket;
try {
  let targets;
  for (let attempt = 0; attempt < 100; attempt++) {
    if (child.exitCode !== null) throw new Error(`Chromium exited: ${chromeLog}`);
    try {
      targets = await (await fetch(`http://127.0.0.1:${debuggingPort}/json/list`)).json();
      if (targets.some(target => target.type === 'page')) break;
    } catch {}
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  const page = targets?.find(target => target.type === 'page');
  assert.ok(page, `Chromium remote debugger did not start: ${chromeLog}`);
  socket = new WebSocket(page.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    socket.addEventListener('open', resolve, {once: true});
    socket.addEventListener('error', reject, {once: true});
  });

  let nextId = 0;
  const pending = new Map();
  const javascriptErrors = [];
  const apiFailures = [];
  socket.addEventListener('message', event => {
    const message = JSON.parse(event.data);
    if (message.id) {
      const waiter = pending.get(message.id);
      if (waiter) {
        pending.delete(message.id);
        message.error ? waiter.reject(new Error(message.error.message)) : waiter.resolve(message.result);
      }
      return;
    }
    if (message.method === 'Runtime.exceptionThrown') {
      javascriptErrors.push(message.params.exceptionDetails.text);
    } else if (message.method === 'Runtime.consoleAPICalled' && message.params.type === 'error') {
      javascriptErrors.push(message.params.args.map(arg => arg.value ?? arg.description ?? '').join(' '));
    } else if (message.method === 'Network.responseReceived') {
      const {url} = message.params.response;
      if (url.startsWith(`${base.origin}/api/`) && message.params.response.status >= 400) {
        apiFailures.push(`${message.params.response.status} ${url}`);
      }
    } else if (message.method === 'Network.loadingFailed') {
      const request = requestUrls.get(message.params.requestId);
      if (request?.startsWith(`${base.origin}/api/`)) apiFailures.push(`${message.params.errorText} ${request}`);
    } else if (message.method === 'Network.requestWillBeSent') {
      requestUrls.set(message.params.requestId, message.params.request.url);
    }
  });
  const requestUrls = new Map();
  function cdp(method, params = {}) {
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      pending.set(id, {resolve, reject});
      socket.send(JSON.stringify({id, method, params}));
    });
  }
  const evaluate = async expression => {
    const result = await cdp('Runtime.evaluate', {
      expression, awaitPromise: true, returnByValue: true, userGesture: true,
    });
    if (result.exceptionDetails) throw new Error(result.exceptionDetails.text);
    return result.result.value;
  };
  const waitFor = async (expression, description, timeoutMs = 30000) => {
    const end = Date.now() + timeoutMs;
    let last;
    while (Date.now() < end) {
      last = await evaluate(expression);
      if (last) return last;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    throw new Error(`Timed out waiting for ${description}; last value: ${JSON.stringify(last)}`);
  };

  await cdp('Page.enable');
  await cdp('Runtime.enable');
  await cdp('Network.enable');
  await cdp('Page.addScriptToEvaluateOnNewDocument', {source:`(() => {
    const NativeWebSocket = window.WebSocket;
    Object.defineProperty(window, '__aezaTestWebSockets', {value:[], configurable:true});
    Object.defineProperty(window, '__aezaTestSnapshotCount', {value:0, writable:true, configurable:true});
    window.WebSocket = new Proxy(NativeWebSocket, {construct(target, args) {
      const socket = Reflect.construct(target, args);
      if (String(args[0]).includes('/api/client/ws')) {
        window.__aezaTestWebSockets.push(socket);
        socket.addEventListener('message', event => {
          try {
            if (JSON.parse(event.data).type === 'snapshot') window.__aezaTestSnapshotCount += 1;
          } catch {}
        });
      }
      return socket;
    }});
  })()`});
  await cdp('Page.navigate', {url: base.href});
  await waitFor(`document.querySelector('button.login-primary')?.innerText.includes('SSO')`, 'SSO login control');
  await evaluate(`Array.from(document.querySelectorAll('button.login-primary')).find(button => button.innerText.includes('SSO')).click()`);
  await waitFor(`location.href.includes('/realms/aeza/protocol/openid-connect/auth')`, 'Keycloak authorization page');
  await waitFor(`document.querySelector('#kc-form-login input#username')`, 'Keycloak login form');
  await evaluate(`(() => {
    const set = (selector, value) => {
      const input = document.querySelector(selector);
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
      setter.call(input, value);
      input.dispatchEvent(new Event('input', {bubbles:true}));
      input.dispatchEvent(new Event('change', {bubbles:true}));
    };
    set('#username', ${JSON.stringify(username)});
    set('#password', ${JSON.stringify(password)});
    document.querySelector('#kc-form-login').requestSubmit();
  })()`);
  await waitFor(`location.origin === ${JSON.stringify(base.origin)} && !location.search.includes('authError')`, 'OIDC callback to the application', 45000);
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Dashboard')`, 'dashboard after SSO login', 45000);
  apiFailures.length = 0; // the initial anonymous /api/auth/me is expected to return 401.
  console.log('PASS: login screen → Keycloak → OIDC callback → authenticated dashboard');

  const unnamedControls = await evaluate(`Array.from(document.querySelectorAll('button, a[href], input, select, textarea'))
    .filter(element => { const rect = element.getBoundingClientRect(); const style = getComputedStyle(element); return rect.width > 0 && rect.height > 0 && style.visibility !== 'hidden'; })
    .filter(element => {
      const labelledBy = element.getAttribute('aria-labelledby');
      const referencedLabel = labelledBy ? labelledBy.split(/\\s+/).map(id => document.getElementById(id)?.innerText ?? '').join(' ').trim() : '';
      const name = element.getAttribute('aria-label') || referencedLabel || element.getAttribute('title') || element.labels?.[0]?.innerText || element.innerText || element.getAttribute('placeholder') || '';
      return !name.trim();
    }).map(element => ({tag:element.tagName, type:element.getAttribute('type'), className:element.className}));`);
  assert.deepEqual(unnamedControls, [], `visible primary controls should expose an accessible name: ${JSON.stringify(unnamedControls)}`);
  console.log('PASS: visible dashboard controls expose accessible names');

  const routes = [
    ['/', 'Dashboard'],
    ['/snapshot', 'Snapshot Model'],
    ['/price-impact', 'Ценовой риск'],
    ['/historical', 'Historical Model'],
    ['/market', 'Брендовый интерес'],
    ['/market?tab=prices', 'Бенчмарк цен'],
    ['/market?tab=monitor', 'Мониторинг рынка'],
    ['/market-simulator', 'Сценарии рынка'],
    ['/settings', 'Настройки'],
  ];
  for (const [route, heading] of routes) {
    await cdp('Page.navigate', {url: new URL(route, base).href});
    const rendered = await waitFor(`document.querySelector('main h1')?.innerText`, `${route} heading`, 45000);
    assert.ok(rendered.includes(heading), `${route} rendered heading ${JSON.stringify(rendered)}`);
    console.log(`PASS: route ${route} renders “${heading}”`);
  }

  const downloadDir = path.join(profile, 'downloads');
  await mkdir(downloadDir);
  await cdp('Browser.setDownloadBehavior', {behavior:'allow', downloadPath:downloadDir});
  await cdp('Page.navigate', {url:new URL('/', base).href});
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Dashboard')`, 'dashboard customer table');
  await evaluate(`document.querySelector('.client-link')?.click()`);
  await waitFor(`location.pathname === '/snapshot' && new URLSearchParams(location.search).has('client')`, 'selected customer snapshot');
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Snapshot Model')`, 'customer snapshot heading');
  console.log('PASS: selecting a customer opens that customer’s snapshot');

  await cdp('Page.navigate', {url:new URL('/', base).href});
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Dashboard')`, 'dashboard before CSV export');
  const activeRiskFilter = await evaluate(`(() => {
    const choices = Array.from(document.querySelectorAll('.legend button')).map(button => ({
      button, count: Number(button.querySelector('strong')?.innerText ?? 0),
      label: button.innerText.trim().split('\\n')[0],
    }));
    const selected = choices.find(choice => choice.count > 0);
    if (!selected) throw new Error('No non-empty risk segment is available for filtered export');
    selected.button.click();
    return {count:selected.count, label:selected.label};
  })()`);
  await waitFor(`document.querySelector('.pagination > span')?.innerText.includes(${JSON.stringify(`из ${activeRiskFilter.count} клиентов`)})`, 'risk filter before export');
  await evaluate(`document.querySelector('.export-menu summary')?.click()`);
  await evaluate(`Array.from(document.querySelectorAll('.export-menu button')).find(button => button.innerText.includes('CSV'))?.click()`);
  let csvFile;
  for (let attempt=0; attempt<100; attempt++) {
    const files = await readdir(downloadDir);
    csvFile = files.find(file => file.endsWith('.csv'));
    if (csvFile) break;
    await new Promise(resolve => setTimeout(resolve, 100));
  }
  assert.ok(csvFile, 'CSV export should download a file from the dashboard');
  const csvBytes = await readFile(path.join(downloadDir, csvFile));
  assert.deepEqual([...csvBytes.subarray(0,3)], [0xef,0xbb,0xbf]);
  assert.ok(csvBytes.length > 100, 'downloaded CSV should contain report rows');
  const csvLines = new TextDecoder().decode(csvBytes).replace(/^\uFEFF/, '').split(/\r?\n/).filter(Boolean);
  assert.equal(csvLines.length - 1, activeRiskFilter.count, 'CSV export should contain every filtered customer across all pages');
  console.log(`PASS: ${activeRiskFilter.label} filter exports all ${activeRiskFilter.count} matching customers as CSV`);

  async function downloadFromMenu(label, extension) {
    await evaluate(`document.querySelector('.export-menu summary')?.click()`);
    await evaluate(`Array.from(document.querySelectorAll('.export-menu button')).find(button => button.innerText.includes(${JSON.stringify(label)}))?.click()`);
    let file;
    for (let attempt=0; attempt<100; attempt++) {
      const files = await readdir(downloadDir);
      file = files.find(candidate => candidate.endsWith(`.${extension}`));
      if (file) break;
      await new Promise(resolve => setTimeout(resolve, 100));
    }
    assert.ok(file, `${label} export should download a file from the dashboard`);
    return readFile(path.join(downloadDir, file));
  }

  const xlsxBytes = await downloadFromMenu('Excel', 'xlsx');
  assert.deepEqual([...xlsxBytes.subarray(0,2)], [0x50,0x4b], 'Excel download should be an XLSX ZIP archive');
  const pdfBytes = await downloadFromMenu('PDF', 'pdf');
  assert.equal(new TextDecoder().decode(pdfBytes.subarray(0,5)), '%PDF-', 'PDF download should have a PDF signature');
  const sqlBytes = await downloadFromMenu('SQL', 'sql');
  assert.match(new TextDecoder().decode(sqlBytes), /CREATE TEMP TABLE/i, 'SQL download should contain the report table');
  console.log('PASS: dashboard export menu downloads valid Excel, PDF, and SQL reports');

  await cdp('Page.navigate', {url: new URL('/', base).href});
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Dashboard')`, 'dashboard reload');
  await evaluate(`(() => {
    const input = document.querySelector('input[placeholder="Поиск по email или ID клиента…"]');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(input, '__e2e_no_matching_customer__');
    input.dispatchEvent(new Event('input', {bubbles:true}));
  })()`);
  await waitFor(`document.querySelector('.table-empty')?.innerText.includes('Клиенты не найдены')`, 'empty search state');
  console.log('PASS: customer search shows its empty state');

  await cdp('Page.navigate', {url: new URL('/settings', base).href});
  await waitFor(`document.querySelector('input[aria-label="Услуги без автопродления: вес"]')`, 'editable settings form');
  const originalWeight = await evaluate(`Number(document.querySelector('input[aria-label="Услуги без автопродления: вес"]').value)`);
  await evaluate(`(() => {
    const input = document.querySelector('input[aria-label="Услуги без автопродления: вес"]');
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set;
    setter.call(input, ${originalWeight + 1});
    input.dispatchEvent(new Event('input', {bubbles:true}));
  })()`);
  await waitFor(`Array.from(document.querySelectorAll('button')).some(button => button.innerText.includes('Сохранить изменения') && !button.disabled)`, 'enabled settings save button');
  await evaluate(`Array.from(document.querySelectorAll('button')).find(button => button.innerText.includes('Сохранить изменения')).click()`);
  await waitFor(`document.querySelector('.settings-feedback.status')?.innerText.includes('Настройки сохранены')`, 'settings save confirmation', 45000);
  console.log('PASS: settings form saves through the authenticated live API');

  await evaluate(`(() => {
    window.confirm = () => true;
    Array.from(document.querySelectorAll('button')).find(button => button.innerText.includes('Восстановить по умолчанию'))?.click();
  })()`);
  await waitFor(`Number(document.querySelector('input[aria-label="Услуги без автопродления: вес"]')?.value) === 15`, 'recommended default model weights');
  await evaluate(`Array.from(document.querySelectorAll('button')).find(button => button.innerText.includes('Сохранить изменения'))?.click()`);
  await waitFor(`document.querySelector('.settings-feedback.status')?.innerText.includes('Настройки сохранены')`, 'default settings save confirmation', 45000);
  console.log('PASS: model settings restore to defaults and persist through the API');

  const socketsBeforeReconnect = await evaluate(`window.__aezaTestWebSockets?.length ?? 0`);
  const snapshotsBeforeReconnect = await evaluate(`window.__aezaTestSnapshotCount ?? 0`);
  assert.ok(socketsBeforeReconnect > 0, 'the authenticated page should own a live WebSocket');
  assert.ok(snapshotsBeforeReconnect > 0, 'the authenticated page should receive an initial WebSocket snapshot');
  await evaluate(`(() => {
    const socket = [...window.__aezaTestWebSockets].reverse().find(item => item.readyState === WebSocket.OPEN);
    if (!socket) throw new Error('No open analyzer WebSocket to disconnect');
    socket.close(1000, 'acceptance reconnect');
  })()`);
  await waitFor(
    `(window.__aezaTestWebSockets?.length ?? 0) > ${socketsBeforeReconnect} && window.__aezaTestWebSockets.at(-1)?.readyState === WebSocket.OPEN && window.__aezaTestSnapshotCount > ${snapshotsBeforeReconnect}`,
    'WebSocket to reconnect and receive a fresh server snapshot',
    20000,
  );
  console.log('PASS: an open browser session reconnects after a dropped WebSocket and receives a fresh snapshot');

  await cdp('Emulation.setDeviceMetricsOverride', {width: 390, height: 844, deviceScaleFactor: 1, mobile: true});
  await cdp('Page.navigate', {url: new URL('/', base).href});
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Dashboard')`, 'mobile dashboard');
  const dimensions = await evaluate(`({width:innerWidth, documentWidth:document.documentElement.scrollWidth, focusable:document.querySelectorAll('a[href],button:not([disabled]),input:not([disabled])').length})`);
  assert.ok(dimensions.documentWidth <= dimensions.width + 2, `horizontal page overflow at mobile size: ${JSON.stringify(dimensions)}`);
  assert.ok(dimensions.focusable > 0, 'primary page controls should be keyboard focusable');
  await cdp('Input.dispatchKeyEvent', {type:'keyDown', key:'Tab', code:'Tab'});
  await cdp('Input.dispatchKeyEvent', {type:'keyUp', key:'Tab', code:'Tab'});
  const focus = await evaluate(`({tag:document.activeElement.tagName, role:document.activeElement.getAttribute('role'), label:document.activeElement.getAttribute('aria-label'), text:document.activeElement.innerText})`);
  assert.notEqual(focus.tag, 'BODY', `Tab did not move focus to a control: ${JSON.stringify(focus)}`);
  console.log(`PASS: 390px layout has no page overflow; keyboard focus moved to ${focus.tag}`);

  await cdp('Network.setBlockedURLs', {urlPatterns:[`${base.origin}/api/auth/me`]});
  await cdp('Page.reload');
  await waitFor(`document.querySelector('.login-page h1')?.innerText.includes('Не удалось проверить вход')`, 'API error state after blocked session request');
  await cdp('Network.setBlockedURLs', {urlPatterns:[]});
  await evaluate(`document.querySelector('.login-retry')?.click()`);
  await waitFor(`document.querySelector('main h1')?.innerText.includes('Dashboard')`, 'recovery after API is available again', 45000);
  apiFailures.length = 0; // the deliberately blocked session check is the tested failure.
  console.log('PASS: API failure state recovers without a manual browser restart');

  const liveCustomerEmail = '<img src=x onerror="window.__aezaXssFired=true">@integration.test';
  await evaluate(`window.__aezaXssFired = false`);
  const replacement = {
    accounts: [{
      id: 990000001, email: liveCustomerEmail, balance: 10000,
      withdrawBalance: 10000, totalReplenished: 10000, bonusBalance: 0, bonusState: 'locked',
    }],
    services: [],
    transactions: [],
  };
  const replacementResponse = await evaluate(`(async () => {
    const response = await fetch('/api/v1/source/snapshot', {
      method: 'PUT', headers: {'content-type':'application/json'}, body: ${JSON.stringify(JSON.stringify(replacement))},
    });
    return {status:response.status, body:await response.text()};
  })()`);
  assert.equal(replacementResponse.status, 200, `test dataset replacement failed: ${replacementResponse.body}`);
  await waitFor(
    `Array.from(document.querySelectorAll('td.email')).some(cell => cell.innerText === ${JSON.stringify(liveCustomerEmail)})`,
    'dashboard row to update from the live dataset without navigating or reloading',
  );
  await evaluate(`new Promise(resolve => setTimeout(resolve, 250))`);
  const renderedXssCell = await evaluate(`(() => {
    const cell = Array.from(document.querySelectorAll('td.email')).find(item => item.innerText === ${JSON.stringify(liveCustomerEmail)});
    return {found:Boolean(cell), hasImage:Boolean(cell?.querySelector('img'))};
  })()`);
  assert.equal(renderedXssCell.found, true, 'the user-provided field should remain visible as text');
  assert.equal(renderedXssCell.hasImage, false, 'the user-provided field must not become executable markup');
  assert.equal(await evaluate(`window.__aezaXssFired === true`), false, 'displaying the hostile field must not execute its handler');
  await waitFor(`document.querySelector('.metrics .metric strong')?.innerText === '1'`, 'customer summary card to update from the live dataset');
  await waitFor(`document.querySelector('.donut-center strong')?.innerText === '1'`, 'risk distribution chart to update from the live scores');
  assert.equal(await evaluate(`location.pathname === '/'`), true, 'live dataset refresh should keep the current page');
  console.log('PASS: an API source replacement updates the open dashboard, summary card and chart through WebSocket without reloading');

  assert.deepEqual(apiFailures, [], `API requests failed during browser flow:\n${apiFailures.join('\n')}`);
  assert.deepEqual(javascriptErrors, [], `browser reported JavaScript errors:\n${javascriptErrors.join('\n')}`);
  console.log('PASS: no failed API requests or JavaScript console/runtime errors');
} finally {
  socket?.close();
  child.kill('SIGTERM');
  await new Promise(resolve => {
    if (child.exitCode !== null) resolve();
    else { child.once('exit', resolve); setTimeout(() => { child.kill('SIGKILL'); resolve(); }, 3000).unref(); }
  });
  await rm(profile, {recursive:true, force:true});
}
