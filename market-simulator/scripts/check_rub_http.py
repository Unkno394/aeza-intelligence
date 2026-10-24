#!/usr/bin/env python3
import argparse,copy,json,urllib.request,urllib.error
from decimal import Decimal as D
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--base',default='http://127.0.0.1:8092/api/market/v2');p.add_argument('--output',required=True,type=Path);a=p.parse_args();a.output.mkdir(parents=True,exist_ok=True)
def call(path,body=None):
 with urllib.request.urlopen(urllib.request.Request(a.base+path,data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json'}),timeout=60) as r:return json.load(r)
s=call('/example');catalog=call('/catalog');own=call('/aeza-plans');contexts=call('/contexts');source=call('/sources');assert len(catalog)==340 and len(own)==110;assert contexts['gpu']['usage']=='reference_only_gpu_not_vps';assert contexts['gpu']['observed_rows']>=contexts['gpu']['listing_sample_count']>0;assert contexts['akash']['daily_points']==2056
assert s['currency']=='RUB' and 'aeza_variable_cost_rub' in s and 'aeza_variable_cost_usd' not in s
plans={p['id']:p for p in catalog};s['aeza_plans']=sorted([p for p in own if p['regions']==['Moscow'] and p['details']['cpu_class']=='shared'],key=lambda p:p['vcpu'])[:3];plans.update({p['id']:p for p in s['aeza_plans']})
s.update(market='russia',buyers_per_month=50,sensitivities=['medium'],accepted_service_types=['vps','cloud_vm']);s['strategies']=[{'name':'baseline','aeza_price_multipliers':[1]*6},{'name':'rise30','aeza_price_multipliers':[1,1,1.3,1.3,1.3,1.3]},{'name':'recovery','aeza_price_multipliers':[1,1,1.3,1.3,.7,.7]}]
for g in s['segments']:g['region']='Russia'
def verify(report):
 assert report['scenario']['currency']=='RUB'
 for run in report['runs']:
  totalrev=totalprofit=D(0)
  for i,m in enumerate(run['months']):
   assert m['active_leases']+m['outside_buyers']==m['cumulative_buyers'];assert sum(m['plan_active_leases'].values())==m['active_leases'];u=m['aeza_capacity_usage'];lim=report['scenario']['aeza_capacity'];assert u['vcpu']<=lim['vcpu'] and u['ram_gb']<=lim['ram_gb'] and u['storage_gb']<=lim['storage_gb']
   for provider,metric in m['providers'].items():
    ledger=m['billing_ledger_rub'][provider];revenue=sum(D(m['exact_prices_rub'][id])*n for id,n in m['plan_active_leases'].items() if plans[id]['provider']==provider);assert D(ledger['revenue'])==revenue
    old=0 if i==0 else run['months'][i-1]['providers'][provider]['active_leases'];assert old==metric['at_risk']==metric['renewals']+metric['switches_out']+metric['churn_to_outside'];assert metric['active_leases']==metric['renewals']+metric['new_leases']+metric['switches_in'];assert metric['new_leases']==metric['first_leases']+metric['returning_leases']
    if provider=='aeza':
     cost=D(str(report['scenario']['aeza_fixed_cost_rub']))+D(str(report['scenario']['aeza_variable_cost_rub']))*metric['active_leases'];assert D(ledger['cost'])==cost;assert D(ledger['profit'])==revenue-cost;totalrev+=revenue;totalprofit+=revenue-cost
   assert not any(plans[id]['details']['market']=='international' for id in m['plan_active_leases'])
  assert D(run['exact_totals_rub']['revenue'])==totalrev;assert D(run['exact_totals_rub']['profit'])==totalprofit
rows=[]
for name in ['vps_only','cloud_allowed','capacity_limited','kvm_required','zero_capacity']:
 q=copy.deepcopy(s)
 if name=='vps_only':q['accepted_service_types']=['vps']
 if name in ['capacity_limited','zero_capacity']:q['aeza_capacity'].update(vcpu=24 if name=='capacity_limited' else 0,ram_gb=48,storage_gb=720)
 if name=='kvm_required':q['required_virtualization']='kvm'
 r=call('/simulate',q);assert r==call('/simulate',q);verify(r)
 (a.output/(name+'-request.json')).write_text(json.dumps(q,indent=2));(a.output/(name+'-response.json')).write_text(json.dumps(r,indent=2))
 for run in r['runs']:
  last=run['months'][-1];m=last['providers']['aeza'];rows.append({'case':name,'strategy':run['strategy'],'seed':q['seed'],'active':m['active_leases'],'share':m['market_share'],'revenue_rub':run['aeza_total_revenue_rub'],'profit_rub':run['aeza_total_profit_rub'],'switches_to_competitors':sum(m['providers']['aeza']['switches_out'] for m in run['months']),'switches_from_competitors':sum(m['providers']['aeza']['switches_in'] for m in run['months']),'competitor_last':{k:v['active_leases'] for k,v in last['providers'].items() if k!='aeza' and v['active_leases']},'passed':True})
 analysis=call('/competition',{'scenario':q,'strategy':'rise30','month':3,'region':'Russia'});assert analysis['report']==r;assert all(not row['comparable'] for row in analysis['comparisons']);(a.output/(name+'-analysis.json')).write_text(json.dumps(analysis,indent=2))
# Reject invented FX and malformed v2 payloads; unknown currencies never become zero.
bad=copy.deepcopy(s);bad['valuation_fx']['rub_per_unit']['USD']='1000'
for payload in [bad,[],{'currency':'RUB'}]:
 try:call('/simulate',payload);raise AssertionError('invalid payload accepted')
 except urllib.error.HTTPError as e:assert e.code==422
(a.output/'rub-http-results.json').write_text(json.dumps(rows,indent=2));(a.output/'source-summary.json').write_text(json.dumps(source,indent=2));print(json.dumps(rows,indent=2))
