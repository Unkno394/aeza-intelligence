#!/usr/bin/env python3
"""Real HTTP economic sweep against the deployed default-catalog application. Standard library only."""
import argparse, copy, json, math, urllib.request
from pathlib import Path

def main():
    a=argparse.ArgumentParser();a.add_argument('--base-url',default='http://127.0.0.1:8091/api/market/v1');a.add_argument('--output',type=Path,default=Path('economic-http-results.json'));a.add_argument('--engine-results',type=Path);args=a.parse_args()
    base=args.base_url.rstrip('/');requests=0
    def get(path):
        nonlocal requests
        requests+=1
        with urllib.request.urlopen(base+path,timeout=30) as r:return json.load(r)
    def post(s):
        nonlocal requests
        requests+=1
        req=urllib.request.Request(base+'/simulate',data=json.dumps(s,allow_nan=False).encode(),headers={'Content-Type':'application/json'})
        with urllib.request.urlopen(req,timeout=30) as r:return json.load(r)
    template=get('/example');health=get('/health');get('/catalog');get('/sources')
    reference={r['id']:r for r in json.loads(args.engine_results.read_text())} if args.engine_results else {}
    rows=[]
    def close(x,y):assert math.isclose(x,y,rel_tol=1e-10,abs_tol=1e-7),(x,y)
    def validate(r):
        s=r['scenario']
        for run in r['runs']:
            previous={};revenue=profit=0
            for m in run['months']:
                providers=m['providers'];active=sum(p['active_leases'] for p in providers.values());assert active==m['active_leases'];assert active+m['outside_buyers']==m['cumulative_buyers']
                close(sum(p['market_share'] for p in providers.values()),1 if active else 0)
                assert sum(p['switches_in'] for p in providers.values())==sum(p['switches_out'] for p in providers.values())
                for name,p in providers.items():
                    prior=previous.get(name,0)
                    assert p['active_leases']+p['switches_out']+p['churn_to_outside']==prior+p['new_leases']+p['switches_in']
                    assert prior==p['renewals']+p['switches_out']+p['churn_to_outside']
                    assert p['active_leases']==p['renewals']+p['new_leases']+p['switches_in']
                    close(p['market_share'],p['active_leases']/active if active else 0)
                    if name=='aeza':
                        close(p['cost_usd'],s['aeza_fixed_cost_usd']+s['aeza_variable_cost_usd']*p['active_leases']);close(p['profit_usd'],p['revenue_usd']-p['cost_usd']);revenue+=p['revenue_usd'];profit+=p['profit_usd']
                    previous[name]=p['active_leases']
            close(run['aeza_total_revenue_usd'],revenue);close(run['aeza_total_profit_usd'],profit)
    scenarios=[]
    for level in ('low','medium','high'):
        for name,mul in [('minus40',.6),('minus20',.8),('minus10',.9),('baseline',1.),('plus10',1.1),('plus20',1.2),('plus40',1.4)]:
            s=copy.deepcopy(template);s.update(seed=42,buyers_per_month=30,sensitivities=[level],strategies=[{'name':name,'aeza_price_multipliers':[mul]*s['months']}])
            scenarios.append(('real_'+level+'_'+name,s,'All balances and exact parity with direct engine when reference supplied.'))
    for name in ('zero_buyers','zero_arrivals','no_eligible_plan','zero_churn'):
        s=copy.deepcopy(template);s.update(seed=42,buyers_per_month=20,sensitivities=['medium'],strategies=[{'name':'edge','aeza_price_multipliers':[1.]*s['months']}])
        if name=='zero_buyers':s['buyers_per_month']=0
        if name=='zero_arrivals':s['arrival_weights']=[0.]*s['months']
        if name=='zero_churn':s['monthly_departure_probability']=0
        if name=='no_eligible_plan':
            for segment in s['segments']:segment.update(vcpu=1e9,ram_gb=1e9,disk_gb=1e9)
        scenarios.append((name,s,'Empty/no-eligible market: zero revenue/shares and retained fixed expenses; or zero exogenous departures for zero churn.'))
    for id,s,expected in scenarios:
        row={'id':id,'expected':expected,'seed':42}
        try:
            r=post(s);validate(r);assert r==post(s),'Replay differs'
            if id in reference:
                actual=[{'strategy':run['strategy'],'sensitivity':run['sensitivity'],'revenue':run['aeza_total_revenue_usd'],'profit':run['aeza_total_profit_usd'],'months':run['months']} for run in r['runs']]
                assert actual==reference[id]['actual']['runs'],'Deployed HTTP economics differ from direct engine'
                assert r['replay_sha256']==reference[id]['actual']['replay_sha256'],'Replay identity differs'
            if id in ('zero_buyers','zero_arrivals','no_eligible_plan'):
                for run in r['runs']:
                    for m in run['months']:
                        assert m['active_leases']==0
                        for p in m['providers'].values():assert p['revenue_usd']==0 and p['market_share']==0
                        assert m['providers']['aeza']['profit_usd']==-s['aeza_fixed_cost_usd']
            if id=='zero_churn':
                assert all(m['providers']['aeza']['churn_to_outside']==0 for run in r['runs'] for m in run['months'])
            row.update(passed=True,actual=r)
        except Exception as e:row.update(passed=False,error=str(e))
        rows.append(row)
    result={'base_url':base,'engine':health,'http_requests':requests,'tests':rows,'passed':all(r['passed'] for r in rows)}
    args.output.parent.mkdir(parents=True,exist_ok=True);args.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps({'passed':result['passed'],'cases':len(rows),'http_requests':requests,'output':str(args.output)}));return 0 if result['passed'] else 1
if __name__=='__main__':raise SystemExit(main())
