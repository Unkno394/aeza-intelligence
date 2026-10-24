#!/usr/bin/env python3
"""Real TCP regression suite, seed 42, archived catalog unchanged. No external packages."""
import argparse, copy, json, subprocess, urllib.request, urllib.error
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--base',default='http://127.0.0.1:8092/api/market/v1');p.add_argument('--output',required=True);p.add_argument('--engine');a=p.parse_args();out=Path(a.output);out.mkdir(parents=True,exist_ok=True)
def call(path,body=None):
 req=urllib.request.Request(a.base+path,data=None if body is None else json.dumps(body).encode(),headers={'Content-Type':'application/json'})
 with urllib.request.urlopen(req,timeout=60) as r:return json.load(r)
def eq(x,y):assert abs(x-y)<1e-7,(x,y)
catalog=call('/catalog');plans={p['id']:p for p in catalog};base=call('/example')
base.update(seed=42,months=6,buyers_per_month=40,arrival_weights=[1,0,0,0,0,0],monthly_departure_probability=0,preference_strength=0,switching_cost_utility=.05,outside_option_utility=-10,market_events=[],strategies=[{'name':'baseline','aeza_price_multipliers':[1]*6}])
base['aeza_plans']=[base['aeza_plans'][1]];base['aeza_plans'][0].update(monthly_usd=24,regions=['Stavanger']);base['segments']=[dict(name='controlled_real_market',weight=1,budget_min=45,budget_max=55,vcpu=2,ram_gb=4,disk_gb=60,region='Stavanger')]
def event(month,mult=1,available=True):return dict(plan_id='index-318',starts_month=month,price_multiplier=mult,available=available,strategy=None)
def verify(r):
 allplans={**plans,**{p['id']:p for p in r['scenario']['aeza_plans']}}
 for run in r['runs']:
  rev=profit=0
  for i,m in enumerate(run['months']):
   assert m['active_leases']+m['outside_buyers']==m['cumulative_buyers'];assert sum(m['plan_active_leases'].values())==m['active_leases']
   eq(sum(p['market_share'] for p in m['providers'].values()),int(m['active_leases']>0))
   for provider,v in m['providers'].items():
    prev=0 if i==0 else run['months'][i-1]['providers'][provider]['active_leases'];assert v['at_risk']==prev
    assert prev==v['renewals']+v['switches_out']+v['churn_to_outside'];assert v['active_leases']==v['renewals']+v['new_leases']+v['switches_in'];assert v['new_leases']==v['first_leases']+v['returning_leases']
    eq(v['revenue_usd'],sum(n*m['effective_prices_usd'][id] for id,n in m['plan_active_leases'].items() if allplans[id]['provider']==provider))
    assert v['switches_out']==sum(f['events'] for f in m['flows'] if f['from']==provider and f['to']!='outside')
    assert v['switches_in']==sum(f['events'] for f in m['flows'] if f['to']==provider and f['from']!='outside')
    if provider=='aeza':
     eq(v['cost_usd'],r['scenario']['aeza_fixed_cost_usd']+r['scenario']['aeza_variable_cost_usd']*v['active_leases']);eq(v['profit_usd'],v['revenue_usd']-v['cost_usd']);rev+=v['revenue_usd'];profit+=v['profit_usd']
   for d in m['decisions']:
    if d['utility'] is not None:eq(d['utility'],2+d['provider_preference']-d['price_penalty']-d['switching_penalty'])
  eq(rev,run['aeza_total_revenue_usd']);eq(profit,run['aeza_total_profit_usd'])
  for f in run['flows']:assert f['unique_buyers']<=f['events'];assert f['events']==sum(x['events'] for m in run['months'] for x in m['flows'] if x['from']==f['from'] and x['to']==f['to'])
rows=[]
for name in ['baseline','own_rise','competitor_cut','entry','recovery','high_switch_cost','zero_buyers','no_suitable_offers']:
 s=copy.deepcopy(base)
 if name in ['own_rise','high_switch_cost']:s['strategies'][0]['aeza_price_multipliers']=[1,1,1.3,1.3,1.3,1.3]
 if name=='high_switch_cost':s['switching_cost_utility']=20
 if name=='competitor_cut':s['market_events']=[event(3,.7)]
 if name=='entry':s['aeza_plans'][0]['monthly_usd']=30.5;s['market_events']=[event(1,available=False),event(3)]
 if name=='recovery':s['strategies'][0]['aeza_price_multipliers']=[1,1,1.3,1.3,.7,.7]
 if name=='zero_buyers':s['buyers_per_month']=0
 if name=='no_suitable_offers':s['segments'][0]['disk_gb']=1e8
 (out/f'{name}-request.json').write_text(json.dumps(s,indent=2))
 r=call('/simulate',s);assert r==call('/simulate',s);verify(r)
 if a.engine:
  direct=json.loads(subprocess.check_output([a.engine,'--scenario',str(out/f'{name}-request.json')]))
  assert direct==r,'HTTP/engine mismatch'
 for run in r['runs']:
  expected=37 if name=='entry' and run['sensitivity']=='low' else 40
  if name in ['own_rise','competitor_cut','entry','recovery']:assert run['months'][2]['providers']['aeza']['switches_out']==expected
  else:assert all(m['providers']['aeza']['switches_out']==0 for m in run['months'])
  if name=='recovery':assert run['months'][4]['providers']['aeza']['switches_in']==40
  if name=='competitor_cut':eq(run['months'][2]['effective_prices_usd']['index-318'],25.09*.7)
  if name=='entry':assert 'index-318' not in run['months'][0]['effective_prices_usd'];eq(run['months'][2]['effective_prices_usd']['index-318'],25.09)
 (out/f'{name}-response.json').write_text(json.dumps(r,indent=2))
 rows.append(dict(case=name,passed=True,expected='all accounting identities, replay and engine/HTTP parity; switches follow utility with fixed seed',actual=[dict(sensitivity=r['sensitivity'],switches_out_month3=r['months'][2]['providers']['aeza']['switches_out'],switches_in_month5=r['months'][4]['providers']['aeza']['switches_in'],final_leases=r['months'][-1]['providers']['aeza']['active_leases'],revenue=r['aeza_total_revenue_usd'],profit=r['aeza_total_profit_usd']) for r in r['runs']]))
 req=dict(scenario=s,strategy='baseline',month=3,region='Stavanger',verified_only=False);analysis=call('/competition',req);assert analysis['report']==r
 for row in analysis['comparisons']:
  assert row['competitor']==plans[row['competitor']['id']]
  if row['effective_price_usd'] is not None:eq(row['indicative_difference_percent'],(row['effective_price_usd']/row['aeza_effective_price_usd']-1)*100)
  if not row['competitor']['regions']:assert not row['comparable']
 (out/f'{name}-analysis.json').write_text(json.dumps(analysis,indent=2))
# Realistic defaults: preserve demographics/price levels; no forcing transitions.
s=call('/example');s['strategies']=[dict(name='baseline',aeza_price_multipliers=[1]*6),dict(name='rise_30',aeza_price_multipliers=[1,1,1.3,1.3,1.3,1.3]),dict(name='recovery',aeza_price_multipliers=[1,1,1.3,1.3,.7,.7])]
for segment in s['segments']:segment['region']='Frankfurt'
for mode in ['control','competitor_cut','entry']:
 s['market_events']=[] if mode=='control' else [event(3,.7)] if mode=='competitor_cut' else [event(1,available=False),event(3)]
 r=call('/simulate',s);verify(r);assert r==call('/simulate',s)
 if a.engine:
  request_path=out/f'default-{mode}-request.json';request_path.write_text(json.dumps(s,indent=2));assert json.loads(subprocess.check_output([a.engine,'--scenario',str(request_path)]))==r
 (out/f'default-{mode}-request.json').write_text(json.dumps(s,indent=2));(out/f'default-{mode}-response.json').write_text(json.dumps(r,indent=2))
 rows.append(dict(case=f'default-{mode}',passed=True,expected='observe actual transitions, no required minimum',actual=[dict(strategy=r['strategy'],sensitivity=r['sensitivity'],flows=r['flows'],final_leases=r['months'][-1]['providers']['aeza']['active_leases'],revenue=r['aeza_total_revenue_usd'],profit=r['aeza_total_profit_usd']) for r in r['runs']]))
invalid=copy.deepcopy(base);invalid['market_events']=[event(3)];invalid['market_events'][0]['plan_id']='fabricated'
try:call('/simulate',invalid);raise AssertionError('invalid accepted')
except urllib.error.HTTPError as e:assert e.code==422
(out/'http-results.json').write_text(json.dumps(rows,indent=2));print(json.dumps(rows,indent=2))
