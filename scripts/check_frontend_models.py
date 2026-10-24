#!/usr/bin/env python3
"""Check that the running React app renders the persisted fixture through HTTP."""

import re
import json
import os
import subprocess
import sys
import tempfile
from html.parser import HTMLParser
from urllib.request import Request, urlopen


class VisibleText(HTMLParser):
    def __init__(self):
        super().__init__()
        self.hidden = 0
        self.parts = []

    def handle_starttag(self, tag, attrs):
        if tag in ('script', 'style'):
            self.hidden += 1

    def handle_endtag(self, tag):
        if tag in ('script', 'style'):
            self.hidden = max(0, self.hidden - 1)

    def handle_data(self, data):
        if not self.hidden:
            self.parts.append(data)


def page_text(url):
    with tempfile.TemporaryDirectory(prefix='aeza-browser-check-') as profile:
        result = subprocess.run(
            ['chromium', '--headless', '--no-sandbox', '--disable-gpu',
             '--disable-dev-shm-usage', '--no-first-run', f'--user-data-dir={profile}',
             '--virtual-time-budget=15000', '--dump-dom', url],
            capture_output=True, text=True, timeout=45, check=True,
        )
    parser = VisibleText()
    parser.feed(result.stdout)
    return ' '.join(' '.join(parser.parts).split())


def main():
    base = sys.argv[1].rstrip('/') if len(sys.argv) > 1 else 'http://127.0.0.1:8080'
    api = os.environ.get('MARKET_API_BASE_URL', 'http://127.0.0.1:3000').rstrip('/')
    settings = {'weights': {'balance': 35, 'renewal': 25, 'autoRenew': 15, 'status': 15, 'transactions': 10},
                'mediumThreshold': 35, 'highThreshold': 65, 'renewalWindowDays': 14}
    request = Request(api + '/api/client/snapshot', data=json.dumps(settings).encode(),
                      headers={'Content-Type': 'application/json'})
    with urlopen(request, timeout=10) as response:
        risks = json.load(response)
    high_count = sum(risk['level'] == 'high' for risk in risks)
    fixture_score = next(risk['score'] for risk in risks if risk['accountId'] == '-8999999')
    dashboard = page_text(base + '/')
    assert 'Всего клиентов' in dashboard and 'Клиенты высокого риска' in dashboard, 'Dashboard did not render API data'
    assert f'Клиенты высокого риска {high_count}' in dashboard, 'Dashboard high-risk count differs from Rust API'
    snapshot = page_text(base + '/snapshot?client=-8999999')
    assert 'model-fixture-0001@example.test' in snapshot and 'Risk Score рассчитывается Rust API' in snapshot, 'Snapshot did not render the fixture and Rust score'
    assert f'{fixture_score} / 100' in snapshot, 'Snapshot Risk Score differs from Rust API'
    historical = page_text(base + '/historical?client=-8999999')
    assert 'Накоплено снимков' in historical and re.search(r'\b(?:[6-9]\d|[1-9]\d{2,})\s*/', historical), 'Historical did not render at least 60 stored days'
    print('Frontend: Dashboard, Snapshot and Historical rendered persisted fixture data')


if __name__ == '__main__':
    main()
