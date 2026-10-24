#!/usr/bin/env bash
set -euo pipefail

python3 - <<'PY'
import json
import urllib.request
import urllib.error
from collections import Counter

base = 'http://127.0.0.1:3000'
paths = {
    'search_interest': '/api/market/search-interest',
    'prices': '/api/market/prices',
    'geography': '/api/market/geography',
    'clients': '/api/client/dataset',
    'client_analysis': '/api/client/analysis',
}
for name, path in paths.items():
    try:
        with urllib.request.urlopen(base + path, timeout=10) as response:
            body = json.load(response)
    except (urllib.error.URLError, json.JSONDecodeError) as error:
        raise SystemExit(f'{name} API check failed: {error}') from None
    if name == 'clients':
        count = len(body['accounts'])
        first_client = body['accounts'][0]['id'] if body['accounts'] else None
    elif name == 'client_analysis':
        count = len(body['clients'])
    else:
        count = len(body)
    print(f'{name}={count}')
    if name == 'search_interest':
        for brand, brand_count in sorted(Counter(row['brandCode'] for row in body).items()):
            print(f'  {brand}={brand_count}')
    if name in {'search_interest', 'prices'} and count == 0:
        raise SystemExit(f'{name} is empty in the API response')
if first_client is not None:
    try:
        with urllib.request.urlopen(base + f'/api/client/historical/{first_client}', timeout=10) as response:
            history = json.load(response)
    except (urllib.error.URLError, json.JSONDecodeError) as error:
        raise SystemExit(f'client_history API check failed: {error}') from None
    print(f'client_history[{first_client}]={len(history)}')
    try:
        with urllib.request.urlopen(base + f'/api/client/cashflow/{first_client}', timeout=10) as response:
            cashflow = json.load(response)
    except (urllib.error.URLError, json.JSONDecodeError) as error:
        raise SystemExit(f'client_cashflow API check failed: {error}') from None
    windows = cashflow['windows']
    assert [item['days'] for item in windows] == [7, 30, 60, 90]
    assert all(item['projectedOutflowCents'] >= 0 and item['scheduledRenewals'] >= 0 for item in windows)
    assert all(a['projectedOutflowCents'] <= b['projectedOutflowCents'] for a, b in zip(windows, windows[1:]))
    assert cashflow['assumption'] and isinstance(cashflow['startingBalanceCents'], int)
    print(f'client_cashflow[{first_client}]: {len(windows)} windows; no-top-up scenario')
PY
