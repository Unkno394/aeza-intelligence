#!/usr/bin/env python3
"""Offline, deterministic preparation. Never converts Akash block heights to UTC/USD."""
import argparse, csv, gzip, hashlib, json, collections, shutil
from pathlib import Path

def digest(p):
    h=hashlib.sha256()
    with p.open('rb') as f:
        for b in iter(lambda:f.read(1024*1024),b''): h.update(b)
    return h.hexdigest()

def prepare(source, dest):
    dest.mkdir(parents=True,exist_ok=True)
    current=source/'research-samples/vps-price-index-current-2026-10-23.json'
    history=source/'research-samples/vps-price-index-history-2026-10-23.json'
    daily=source/'akash_daily_market_stats.csv'
    leases=source/'akash_historical_leases.csv.gz'
    snapshot=json.loads(current.read_text())
    # Preserve complete source observations; do not silently assert CPU architecture or stock.
    shutil.copyfile(current,dest/'vps-price-index-current.json')
    shutil.copyfile(history,dest/'vps-price-index-history.json')
    shutil.copyfile(daily,dest/'akash_daily_market_stats.csv')
    plans=[]; excluded=collections.Counter()
    for i,p in enumerate(snapshot['plans']):
        fields=[p.get(k) for k in ('price_monthly_usd','vcpu','ram_gb','disk_gb')]
        if p.get('type')!='vps' or not all(isinstance(v,(int,float)) and v>0 for v in fields):
            excluded['not_vps_or_missing_positive_monthly_usd_specs']+=1;continue
        # Restrict MVP market to x86-like general VPS; uncertain architecture is excluded.
        if any(w in p['plan_id'].lower() for w in ('arm','gpu')):
            excluded['arm_or_gpu_name']+=1;continue
        plans.append(dict(id=f'index-{i}',provider=p['provider'],name=p['plan_id'],vcpu=p['vcpu'],ram_gb=p['ram_gb'],disk_gb=p['disk_gb'],monthly_usd=p['price_monthly_usd'],regions=p.get('locations') or [],origin='observed_list_price',source='https://vpslocate.com/data/',observed_at=snapshot['captured_at']))
    counts=collections.Counter(); denoms=collections.Counter(); shapes=collections.Counter()
    with gzip.open(leases,'rt',newline='') as f:
        for r in csv.DictReader(f):
            counts['rows']+=1;denoms[r['price_denom']]+=1
            counts['with_start_utc']+=bool(r['lease_start_utc'])
            counts['with_close_block']+=bool(r['lease_closed_block'])
            if all(r[k] for k in ('cpu_vcores','memory_gib','storage_gib')):
                counts['with_resources']+=1
                if float(r['gpu_units'] or 0)==0:
                    shapes[(r['cpu_vcores'],r['memory_gib'],r['storage_gib'])]+=1
    quality=dict(counts,denom_counts=dict(sorted(denoms.items())),cpu_only_shape_count=len(shapes))
    rows=list(csv.DictReader(daily.open()))
    # Last complete calendar months only; partial October is deliberately excluded.
    monthly=collections.Counter()
    for r in rows:
        if r['date_utc'][:7]<'2026-10' and r['new_lease_events_count']!='':
            monthly[r['date_utc'][:7]]+=int(r['new_lease_events_count'])
    keys=sorted(monthly)[-6:]; mean=sum(monthly[k] for k in keys)/len(keys)
    profile={'months':keys,'new_lease_events':[monthly[k] for k in keys],'normalized_weights':[monthly[k]/mean for k in keys], 'interpretation':'Observed Akash aggregate event shape; transfer and population scale are scenario assumptions, not VPS demand estimates.'}
    def save(n,x): (dest/n).write_text(json.dumps(x,ensure_ascii=False,indent=2)+'\n')
    save('catalog.json',plans);save('akash_quality.json',quality);save('akash_profile.json',profile)
    save('manifest.json',{'version':1,'prepared_as_of':'2026-10-23','source_files':[{'file':p.name,'sha256':digest(p)} for p in (current,history,daily,leases)],'catalog_included':len(plans),'catalog_excluded':dict(excluded),'license':'VPS Price Index — vpslocate.com (CC-BY 4.0), https://vpslocate.com/data/','limitations':['List prices are not transactions or demand. Taxes, stock, architecture, IP and regional comparability are not fully verified.','Akash container leases are not interchangeable with VPS. No elasticity, churn or customer budgets inferred.','Akash exact lease UTC absent; resource joins partial. Block duration is never converted to a calendar lifetime.','Index history is changing-composition aggregate min/median/max, not a per-plan price panel.','Aéza offers and all costs in example are scenario assumptions.']})
    print(json.dumps({'catalog':len(plans),'quality':quality,'profile':profile},ensure_ascii=False))

if __name__=='__main__':
    a=argparse.ArgumentParser();a.add_argument('source',type=Path);a.add_argument('--dest',type=Path,default=Path(__file__).resolve().parents[1]/'data');n=a.parse_args();prepare(n.source,n.dest)
