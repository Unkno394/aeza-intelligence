#!/usr/bin/env python3
"""Generate repeatable PostgreSQL fixtures; Python 3.10+, no dependencies."""

import argparse
import calendar
import json
import random
from datetime import datetime, timedelta, timezone
from pathlib import Path

TERMS = ('hour', 'week', 'month', 'quarter_year', 'half_year', 'year')
STATES = ('active', 'blocked', 'deleted', 'suspended', 'prolong_wait')


def iso(value):
    return value.astimezone(timezone.utc).isoformat(timespec='milliseconds').replace('+00:00', 'Z')


def add_term(value, term):
    if term == 'hour':
        return value + timedelta(hours=1)
    if term == 'week':
        return value + timedelta(weeks=1)
    months = {'month': 1, 'quarter_year': 3, 'half_year': 6, 'year': 12}[term]
    month_index = value.year * 12 + value.month - 1 + months
    year, month = divmod(month_index, 12)
    month += 1
    day = min(value.day, calendar.monthrange(year, month)[1])
    return value.replace(year=year, month=month, day=day)


def generate(accounts_count, services_count, transactions_count, seed, reference, id_offset=0):
    rng = random.Random(seed)
    accounts = []
    for i in range(1, accounts_count + 1):
        balance = rng.randint(0, 100_000)
        ident = id_offset + i
        email_prefix = 'model-fixture' if id_offset else 'user'
        accounts.append({
            'id': ident, 'email': f'{email_prefix}-{i:04d}@example.test', 'balance': balance,
            'withdrawBalance': rng.randint(0, balance),
            'totalReplenished': balance + rng.randint(0, 500_000),
            'bonusBalance': rng.randint(0, 10_000),
            'bonusState': ('locked', 'unlocked')[(i - 1) % 2],
        })
    services = []
    for i in range(1, services_count + 1):
        term = TERMS[(i - 1) % len(TERMS)]
        status = STATES[(i - 1) % len(STATES)]
        if status == 'active':
            created = reference - timedelta(minutes=30)
        elif status in ('deleted', 'prolong_wait'):
            created = reference - timedelta(days=400)
        else:
            created = reference - timedelta(days=rng.randint(1, 90))
        services.append({
            'id': id_offset + i, 'name': f'service-{i:04d}',
            'ownerId': id_offset + (i - 1) % accounts_count + 1,
            'paymentTerm': term, 'autoProlong': bool(rng.getrandbits(1)),
            'createdAt': iso(created), 'expiresAt': iso(add_term(created, term)),
            'status': status, 'price': rng.randint(1, 1000),
        })
    transactions = []
    for i in range(1, transactions_count + 1):
        service = services[(i - 1) % services_count]
        service_created = datetime.fromisoformat(service['createdAt'].replace('Z', '+00:00'))
        created = service_created + timedelta(minutes=rng.randint(0, 5))
        performed = i % 3 != 0
        amount = 0 if i == 1 else 1000 if i == 2 else rng.randint(0, 1000)
        transactions.append({
            'id': id_offset + i, 'amount': amount,
            'status': 'performed' if performed else 'not_performed',
            'performedAt': iso(created + timedelta(seconds=rng.randint(1, 60))) if performed else None,
            'createdAt': iso(created),
            'type': 'order' if i <= services_count else 'prolong',
            'serviceId': service['id'],
        })
    return {'accounts': accounts, 'services': services, 'transactions': transactions}


def model_history(accounts, days, reference):
    """Synthetic daily feature snapshots for testing both Rust models offline."""
    history = []
    for day in range(days):
        observed = reference - timedelta(days=days - day - 1)
        clients = []
        for account in accounts:
            ident = account['id']
            base = ident % 5
            variation = (day % 7) * 0.002
            unusual = ident == accounts[0]['id'] and day == days - 1
            clients.append({
                'accountId': ident,
                'email': account['email'],
                'servicesCount': 3 + base + (8 if unusual else 0),
                'monthlySpend': float(1200 + 210 * base + (1500 if unusual else 0)),
                'shortTermShare': round(0.10 + base * 0.05, 3),
                'noAutoProlongShare': round(0.1 + base * 0.03 + (0.75 if unusual else variation), 3),
                'prolongWaitShare': round(0.05 + base * 0.02 + (0.75 if unusual else variation), 3),
                'failedProlongShare': round(0.02 + base * 0.01 + (0.75 if unusual else variation), 3),
                'expiringRevenueShare': round(0.12 + base * 0.04 + (0.55 if unusual else variation), 3),
                'upcomingPayments': 1 + base + (5 if unusual else 0),
                'spendableBalance': 10000 + ident * 500,
                'paymentCoverage': round(0.04 if unusual else 2.2 - base * 0.18 + variation, 3),
            })
        history.append({'observedAt': iso(observed), 'clients': clients})
    return history


def seed_sql(data):
    parts = [
        '-- Generated from data.json. Run only on empty fixture tables.',
        'BEGIN;',
        'LOCK TABLE public.accounts, public.services, public.transactions IN EXCLUSIVE MODE;',
        "DO $$ BEGIN IF EXISTS (SELECT 1 FROM public.accounts) OR EXISTS (SELECT 1 FROM public.services) OR EXISTS (SELECT 1 FROM public.transactions) THEN RAISE EXCEPTION 'Seed requires empty accounts, services and transactions tables'; END IF; END $$;",
    ]
    for table, rows in data.items():
        if not rows:
            continue
        columns = ', '.join('"' + name + '"' for name in rows[0])
        tag = f'$fixture_{table}$'
        payload = json.dumps(rows, ensure_ascii=True, indent=2)
        parts.append(f'INSERT INTO public.{table} ({columns})\nSELECT {columns} FROM jsonb_populate_recordset(NULL::public.{table}, {tag}{payload}{tag}::jsonb);')
        parts.append(f"SELECT setval(pg_get_serial_sequence('public.{table}', 'id'), GREATEST((SELECT max(id) FROM public.{table}), 1), true);")
    parts.append('COMMIT;')
    return '\n\n'.join(parts) + '\n'


def additive_fixture_sql(data):
    """Insert a namespaced fixture into an existing database without changing sequences."""
    parts = ['-- Additive model fixture. Existing non-fixture IDs cause a rollback.', 'BEGIN;']
    for table, rows in data.items():
        if not rows:
            continue
        ids = ', '.join(str(row['id']) for row in rows)
        if table == 'accounts':
            safe = "email LIKE 'model-fixture-%@example.test'"
        elif table == 'services':
            safe = '"ownerId" < 0'
        else:
            safe = '"serviceId" < 0'
        parts.append(f"DO $$ BEGIN IF EXISTS (SELECT 1 FROM public.{table} WHERE id IN ({ids}) AND NOT ({safe})) THEN RAISE EXCEPTION 'Non-fixture ID collision in {table}'; END IF; END $$;")
        columns = ', '.join('"' + name + '"' for name in rows[0])
        tag = f'$fixture_{table}$'
        payload = json.dumps(rows, ensure_ascii=True)
        parts.append(f'INSERT INTO public.{table} ({columns}) SELECT {columns} FROM jsonb_populate_recordset(NULL::public.{table}, {tag}{payload}{tag}::jsonb) WHERE true ON CONFLICT (id) DO NOTHING;')
    parts.append('COMMIT;')
    return '\n\n'.join(parts) + '\n'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accounts', type=int, default=10)
    parser.add_argument('--services', type=int, default=30)
    parser.add_argument('--transactions', type=int, default=100)
    parser.add_argument('--seed', type=int, default=42)
    parser.add_argument('--reference', default='2026-10-19T15:00:00.000Z', help='ISO 8601 timestamp with timezone')
    parser.add_argument('--history-days', type=int, default=60)
    parser.add_argument('--id-offset', type=int, default=0, help='Use a negative offset for an additive test fixture')
    parser.add_argument('--out', type=Path, default=Path('.'))
    args = parser.parse_args()
    if args.accounts < 1 or args.services < 1 or args.transactions < 0 or args.history_days < 31:
        parser.error('accounts and services must be >= 1, transactions >= 0, history-days >= 31')
    try:
        reference = datetime.fromisoformat(args.reference.replace('Z', '+00:00'))
        if reference.tzinfo is None:
            raise ValueError('timezone is required')
        reference = reference.astimezone(timezone.utc)
        if args.id_offset and args.id_offset + max(args.accounts, args.services, args.transactions) >= 0:
            raise ValueError('id-offset must keep all fixture IDs negative')
        data = generate(args.accounts, args.services, args.transactions, args.seed, reference, args.id_offset)
    except (ValueError, OverflowError) as exc:
        parser.error(str(exc))
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / 'data.json').write_text(json.dumps(data, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')
    (args.out / '002_seed.sql').write_text(seed_sql(data), encoding='utf-8')
    if args.id_offset:
        (args.out / 'fixture_insert.sql').write_text(additive_fixture_sql(data), encoding='utf-8')
    (args.out / 'model_history.json').write_text(
        json.dumps(model_history(data['accounts'], args.history_days, reference), ensure_ascii=False, indent=2) + '\n',
        encoding='utf-8',
    )
    print(f'Created data.json, 002_seed.sql and model_history.json: {args.accounts} accounts, {args.services} services, {args.transactions} transactions, {args.history_days} history days')


if __name__ == '__main__':
    main()
