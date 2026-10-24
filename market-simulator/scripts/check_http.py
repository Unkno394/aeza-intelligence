import json, urllib.request, urllib.error, copy, math
from pathlib import Path
import argparse
a=argparse.ArgumentParser();a.add_argument('--base-url',default='http://127.0.0.1:8091/api/market/v1');a.add_argument('--output',type=Path,default=Path('http-audit-results'));args=a.parse_args();BASE=args.base_url.rstrip('/');out=args.output;out.mkdir(parents=True,exist_ok=True)
checks=[]
def get(path):return json.load(urllib.request.urlopen(BASE+path))
def post(s,status=200):
 data=json.dumps(s,allow_nan=False).encode();req=urllib.request.Request(BASE+'/simulate',data=data,headers={'Content-Type':'application/json'})
 try:
  with urllib.request.urlopen(req) as r: code=r.status;raw=r.read()
 except urllib.error.HTTPError as e:code=e.code;raw=e.read()
 assert code==status,(code,raw[:300]);return json.loads(raw)
def check(name):checks.append(name)
def close(a,b):assert math.isclose(a,b,abs_tol=1e-7,rel_tol=1e-10),(a,b)
def accounting(r):
 s=r['scenario']
 for run in r['runs']:
  prior={};rev=profit=0
  for m in run['months']:
   ps=m['providers'];active=sum(p['active_leases'] for p in ps.values());assert active==m['active_leases'];assert active+m['outside_buyers']==m['cumulative_buyers']
   close(sum(p['market_share'] for p in ps.values()),1 if active else 0)
   assert sum(p['switches_in'] for p in ps.values())==sum(p['switches_out'] for p in ps.values())
   for name,p in ps.items():
    prev=prior.get(name,0)
    assert p['active_leases']==prev+p['new_leases']+p['switches_in']-p['switches_out']-p['churn_to_outside']
    assert p['active_leases']==p['new_leases']+p['renewals']+p['switches_in']
    assert prev==p['renewals']+p['switches_out']+p['churn_to_outside']
    close(p['market_share'],p['active_leases']/active if active else 0)
    assert p['revenue_usd']>=0
    if name=='aeza':
     cost=s['aeza_fixed_cost_usd']+s['aeza_variable_cost_usd']*p['active_leases'];close(cost,p['cost_usd']);close(p['profit_usd'],p['revenue_usd']-cost);rev+=p['revenue_usd'];profit+=p['profit_usd']
    else:assert p['cost_usd'] is None and p['profit_usd'] is None
    prior[name]=p['active_leases']
  close(rev,run['aeza_total_revenue_usd']);close(profit,run['aeza_total_profit_usd'])
assert get('/health')['status']=='ok';cat=get('/catalog');check('HTTP health/catalog/example/sources')
s=get('/example');get('/sources');get('/openapi.json');r=post(s);accounting(r);(out/'comparison-response.json').write_text(json.dumps(r,indent=2));check('12 real strategy/sensitivity runs: stock/flow, switching balance, shares, cost, revenue, profit')
assert r==post(s);check('identical HTTP replay')
f=copy.deepcopy(s);f.update(months=3,buyers_per_month=20,arrival_weights=[1]*3,sensitivities=['medium'],monthly_departure_probability=0,preference_strength=0,switching_cost_utility=0,external_models=[])
f['segments']=f['segments'][:1];f['segments'][0].update(budget_min=20,budget_max=20,vcpu=1e6,ram_gb=1e6,disk_gb=1e6)
f['aeza_plans']=f['aeza_plans'][:1];f['aeza_plans'][0].update(vcpu=1e6,ram_gb=1e6,disk_gb=1e6,monthly_usd=5)
f['strategies']=[{'name':'known-price','aeza_price_multipliers':[1]*3}]
a=post(f);accounting(a)
for i,m in enumerate(a['runs'][0]['months']):p=m['providers']['aeza'];assert p['active_leases']==20*(i+1);close(p['revenue_usd'],100*(i+1))
check('known single-plan demand and exact billings 100/200/300 USD')
g=copy.deepcopy(f);g['strategies'][0]['aeza_price_multipliers']=[1,2,2];b=post(g);accounting(b);assert [m['providers']['aeza']['revenue_usd'] for m in b['runs'][0]['months']]==[100,400,600];check('price repricing of both renewals and new leases')
g=copy.deepcopy(f);g['strategies'][0]['aeza_price_multipliers']=[1,5,5];b=post(g);accounting(b);assert b['runs'][0]['months'][1]['active_leases']==0;assert b['runs'][0]['months'][1]['providers']['aeza']['churn_to_outside']==20;check('budget breach: demand zero, prior leases churn, fixed cost remains')
g=copy.deepcopy(f);g['monthly_departure_probability']=1;b=post(g);accounting(b);assert b['runs'][0]['months'][1]['outside_buyers']==20;check('100% departure and next-period reactivation semantics')
g=copy.deepcopy(f);g['monthly_departure_probability']=.4;b=post(g);accounting(b);check('seeded stochastic churn with accounting conservation')
g=copy.deepcopy(f);g['aeza_fixed_cost_usd']=350;g['aeza_variable_cost_usd']=7;b=post(g);accounting(b)
for x,y in zip(a['runs'][0]['months'],b['runs'][0]['months']):assert x['active_leases']==y['active_leases'];close(x['providers']['aeza']['revenue_usd'],y['providers']['aeza']['revenue_usd'])
check('costs change profit without secretly changing buyer demand')
g=copy.deepcopy(f);g['sensitivities']=['low','medium','high'];g['aeza_plans'][0]['monthly_usd']=15;b=post(g);assert [x['months'][0]['active_leases'] for x in b['runs']]==[20,20,0];check('three sensitivity levels, high sensitivity outside option')
g=copy.deepcopy(f);g['segments'][0].update(vcpu=1,ram_gb=1,disk_gb=20);g['aeza_plans'][0]['monthly_usd']=.01;g['strategies'][0]['aeza_price_multipliers']=[1,20,20];g['aeza_plans'][0]['monthly_usd']=1
# Ensure first price undercuts every eligible competitor, second month exceeds budget.
minprice=min(p['monthly_usd'] for p in cat if p['vcpu']>=1 and p['ram_gb']>=1 and p['disk_gb']>=20)
g['aeza_plans'][0]['monthly_usd']=minprice/2;g['strategies'][0]['aeza_price_multipliers']=[1,20,20]
# multiplier cap may not exceed budget, but competitor is cheaper by month 2.
b=post(g);accounting(b);m=b['runs'][0]['months'][1];assert m['providers']['aeza']['switches_out']==20;assert sum(p['switches_in'] for n,p in m['providers'].items() if n!='aeza')==20;check('Aéza price increase produces actual competitor migrations')
g=copy.deepcopy(f);g['strategies']*=2;g['strategies'][1]=copy.deepcopy(g['strategies'][1]);g['strategies'][1]['name']='same-prices';b=post(g);assert b['runs'][0]['months']==b['runs'][1]['months'];check('same prices give identical strategy paths')
for field,value in [('months',0),('monthly_departure_probability',1.1),('buyers_per_month',1001)]:g=copy.deepcopy(f);g[field]=value;post(g,422)
check('invalid scenarios rejected over HTTP')
summary={'base_url':BASE,'checks':checks,'status':'PASS','economic_limits':['One lease per synthetic buyer; no capacity constraint.','Churn is a period outside; reactivation possible next month.','All active leases repriced each month, no grandfathering.','Aéza offers/costs and behavioral inputs are scenario assumptions; profit is simplified operating contribution, not audited accounting profit.','Price response is rule-based, not empirically identified elasticity.']}
(out/'summary.json').write_text(json.dumps(summary,indent=2));print(json.dumps(summary,indent=2))
