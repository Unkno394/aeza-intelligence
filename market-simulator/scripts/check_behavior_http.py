#!/usr/bin/env python3
"""Reproducible live HTTP buyer analytics/recommendation audit (stdlib only)."""
import argparse,copy,json,urllib.request,urllib.error
from decimal import Decimal
from pathlib import Path
p=argparse.ArgumentParser();p.add_argument('--base',default='http://127.0.0.1:8091/api/market/v2');p.add_argument('--output',required=True);a=p.parse_args();out=Path(a.output);out.mkdir(parents=True,exist_ok=True)
def req(path,data=None):
    r=urllib.request.Request(a.base+path,data=None if data is None else json.dumps(data).encode(),headers={'Content-Type':'application/json'})
    with urllib.request.urlopen(r,timeout=60) as r:return json.load(r)
s=req('/example');s['seed']=42;s['buyers_per_month']=30
cases=[]
for level in ['low','medium','high']:cases.append((level,copy.deepcopy(s),level,'Полный баланс всех решений; повторный seed идентичен; лучшая прибыль выбрана из проверенных цен.'))
for name,expected in [('zero','Нет клиентов и причин; контроль сохраняется.'),('random_exit','Уходы имеют неизвестную экзогенную причину.'),('resources','Ограничение ресурсов выявлено без назначения ценовой причины.'),('region','Регион не покрыт Aéza.'),('capacity','Нулевые мощности запрещают продажи Aéza.'),('budget','Все подходящие тарифы дороже бюджета.'),('cloud_choice','Покупатели с большим бюджетом выбирают облака при нулевой мощности Aéza.')]:
    t=copy.deepcopy(s)
    if name=='zero':t['buyers_per_month']=0
    if name=='random_exit':t['monthly_departure_probability']=1
    if name=='capacity' or name=='cloud_choice':t['aeza_capacity']['vcpu']=0
    for g in t['segments']:
        if name=='resources':g['vcpu']=1000
        if name=='region':g['region']='Atlantis'
        if name=='budget':g['budget_min']=1;g['budget_max']=2
        if name=='cloud_choice':g['budget_min']=10000;g['budget_max']=10000
    if name=='cloud_choice':t['accepted_service_types']=['vps','cloud_vm']
    cases.append((name,t,'medium',expected))
rows=[]
for name,t,level,expected in cases:
    payload={'scenario':t,'strategy':t['strategies'][0]['name'],'sensitivity':level}
    d=req('/behavior',payload);assert d==req('/behavior',payload),'non deterministic replay'
    (out/f'{name}-request.json').write_text(json.dumps(payload,ensure_ascii=False,indent=2))
    (out/f'{name}-response.json').write_text(json.dumps(d,ensure_ascii=False,indent=2))
    prepared=d['report']['scenario'];direct=req('/simulate',prepared);assert d['report']==direct,'analytics differs from direct simulation'
    # Focused control must preserve the input strategy, not reset its prices or other inputs.
    original=copy.deepcopy(t);original['strategies']=[next(x for x in t['strategies'] if x['name']==payload['strategy'])];original['sensitivities']=[level]
    original_report=req('/simulate',original);assert original_report['runs'][0]['months']==d['report']['runs'][0]['months']
    totals={};competitors={}
    for run in d['report']['runs']:
        for m in run['months']:
            b=m['buyer_behavior'];z=m['providers']['aeza']
            assert sum(x['decisions'] for x in b)==m['cumulative_buyers']
            assert sum(x['aeza_active'] for x in b)==z['active_leases']
            assert sum(x['aeza_at_risk'] for x in b)==z['at_risk']
            assert sum(x['switches_out'] for x in b)==z['switches_out']
            assert sum(x['exogenous_exits']+x['choice_exits'] for x in b)==z['churn_to_outside']
            for x in b:
                assert sum(x['reasons'].values())==x['decisions']
                assert x['aeza_active']==x['aeza_at_risk']+x['first_leases']+x['market_returns']+x['switches_in']-x['switches_out']-x['exogenous_exits']-x['choice_exits']
                risk=x['aeza_at_risk'];loss=x['switches_out']+x['exogenous_exits']+x['choice_exits']
                assert x['loss_rate']==(loss/risk if risk else None)
            ledger=m['billing_ledger_rub']['aeza'];assert Decimal(ledger['revenue'])-Decimal(ledger['cost'])==Decimal(ledger['profit'])
        assert all(n<=run['months'][-1]['cumulative_buyers'] for n in run['behavior_unique_clients'].values())
    c=d['candidates'];best=max(Decimal(x['exact_totals_rub']['profit']) for x in c);chosen=next(x for x in c if x['strategy']==d['recommended_strategy']);assert Decimal(chosen['exact_totals_rub']['profit'])==best
    for m in d['report']['runs'][0]['months']:
        for x in m['buyer_behavior']:
            for k,n in x['reasons'].items():totals[k]=totals.get(k,0)+n
            for k,n in x['competitor_choices'].items():competitors[k]=competitors.get(k,0)+n
    if name=='zero':assert totals=={} and d['recommended_strategy']=='control'
    if name=='random_exit':assert totals.get('exogenous_departure_unknown_cause',0)>0
    if name in ['resources','region','capacity','budget']:assert totals.get('aeza_'+name,0)>0
    if name=='cloud_choice':assert competitors
    rows.append({'case':name,'expected':expected,'passed':True,'reasons_events':totals,'competitor_choice_events':competitors,'candidates':c,'recommendation':d['recommended_strategy']})
    print(name,'PASS',d['recommended_strategy'])
for payload in [{}, {'scenario':s,'strategy':'missing','sensitivity':'medium'}, {'scenario':{**s,'valuation_fx':{'date':'fake'}},'strategy':s['strategies'][0]['name'],'sensitivity':'medium'}]:
    try:req('/behavior',payload);raise AssertionError('invalid payload accepted')
    except urllib.error.HTTPError as e:assert e.code in [400,422]
(out/'results.json').write_text(json.dumps(rows,ensure_ascii=False,indent=2));print(len(rows),'live HTTP cases passed + 3 invalid requests')
