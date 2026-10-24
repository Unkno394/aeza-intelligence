#!/usr/bin/env python3
"""Independent arithmetic + keyed buyer census audit against live Rust HTTP and CLI.
No production model, network data ingestion or ML implementation.
"""
import argparse,copy,json,math,subprocess,urllib.request
from decimal import Decimal,ROUND_HALF_EVEN
from collections import Counter,defaultdict
from pathlib import Path
ap=argparse.ArgumentParser();ap.add_argument('--base',default='http://127.0.0.1:8091/api/market/v2');ap.add_argument('--cases',default='');ap.add_argument('--binary',required=True);ap.add_argument('--output',required=True);args=ap.parse_args();out=Path(args.output);out.mkdir(parents=True,exist_ok=True)
D=Decimal;Q=D('0.000000001');MASK=(1<<64)-1

def request(path,payload=None):
 req=urllib.request.Request(args.base+path,data=None if payload is None else json.dumps(payload).encode(),headers={'Content-Type':'application/json'})
 with urllib.request.urlopen(req,timeout=60) as r:return json.load(r)
def save(name,v):(out/name).write_text(json.dumps(v,ensure_ascii=False,indent=2))
def money(n):return D(str(n)).quantize(Q,rounding=ROUND_HALF_EVEN)
def draw(seed,buyer,month,channel):
 z=((seed+buyer*0x9e3779b97f4a7c15)&MASK)^((month*0xbf58476d1ce4e5b9)&MASK)^((channel*0x94d049bb133111eb)&MASK)
 z=(z+0x9e3779b97f4a7c15)&MASK;z=((z^(z>>30))*0xbf58476d1ce4e5b9)&MASK;z=((z^(z>>27))*0x94d049bb133111eb)&MASK
 return ((z^(z>>31))>>11)/(1<<53)
def preference(seed,buyer,provider):
 h=14695981039346656037
 for c in provider.encode():h=((h^c)*1099511628211)&MASK
 return draw(seed,buyer,0,h)
def region_matches(regions,required):
 def canonical(s):
  s=''.join(c for c in s.lower() if c.isalpha());return {'россия':'russia','москва':'moscow','санктпетербург':'saintpetersburg'}.get(s,s)
 eu={'amsterdam','frankfurt','gravelines','helsinki','madrid','milan','paris','stockholm','strasbourg','warsaw','copenhagen'}
 a=canonical(required)
 return any(canonical(r)==a or (canonical(r)=='eu' and a in eu) or(a=='eu' and canonical(r) in eu) or(a=='russia' and canonical(r) in ['moscow','saintpetersburg']) for r in regions)
def policy(s,p,price):
 d=p.get('details') or {}
 return price is not None and d.get('availability')!='unavailable' and (s['market']=='all' or d.get('market')==s['market']) and d.get('service_type','vps') in s['accepted_service_types'] and (not s.get('required_virtualization') or d.get('virtualization')==s['required_virtualization']) and (not s.get('required_cpu_class') or d.get('cpu_class')==s['required_cpu_class']) and (not s.get('require_complete_monthly_payment') or d.get('complete_monthly_payment',False))
def resources(p):return [money(p[k]) for k in ['vcpu','ram_gb','disk_gb']]

def check_report(report,catalog,census=True):
 s=report['scenario'];plans=sorted(catalog+s['aeza_plans'],key=lambda p:p['id']);pm={p['id']:p for p in plans};checks=0
 for run in report['runs']:
  strategy=next(x for x in s['strategies'] if x['name']==run['strategy']);totalrev=D(0);totalprofit=D(0);prev={};buyers=[];unique=defaultdict(set);flow_ids=defaultdict(set);flow_events=Counter()
  for m in run['months']:
   mo=m['month']-1;assert m['arrivals']==math.floor(s['buyers_per_month']*s['arrival_weights'][mo]+.5)
   for p in plans:
    price=None if (p.get('details') or {}).get('availability')=='unavailable' else money(p['monthly_rub'])
    if price is not None:
     factor=strategy['aeza_price_multipliers'][mo] if p['provider']=='aeza' else 1
     if p['provider']!='aeza':
      events=[e for e in s['market_events'] if e['plan_id']==p['id'] and e['starts_month']<=mo+1 and (not e.get('strategy') or e['strategy']==run['strategy'])]
      if events:
       e=max(events,key=lambda e:e['starts_month']);factor=e['price_multiplier']
       if not e['available']:price=None
     if price is not None:price=(price*D(str(factor))).quantize(Q,rounding=ROUND_HALF_EVEN)
    assert (D(m['exact_prices_rub'][p['id']]) if p['id'] in m['exact_prices_rub'] else None)==price,(run['strategy'],p['id'],'price mismatch')
   for provider,a in m['providers'].items():
    revenue=sum((D(m['exact_prices_rub'][pid])*n for pid,n in m['plan_active_leases'].items() if pm[pid]['provider']==provider),D(0));ledger=m['billing_ledger_rub'][provider];assert revenue==D(ledger['revenue'])
    before=prev.get(provider,0);assert a['at_risk']==before
    assert a['active_leases']==before+a['new_leases']+a['switches_in']-a['switches_out']-a['churn_to_outside']
    assert a['new_leases']==a['first_leases']+a['returning_leases'];assert a['renewals']+a['churn_to_outside']+a['switches_out']==a['at_risk']
    assert a['market_share']==(a['active_leases']/m['active_leases'] if m['active_leases'] else 0)
    if provider=='aeza':
     cost=money(s['aeza_fixed_cost_rub'])+money(s['aeza_variable_cost_rub'])*a['active_leases'];assert D(ledger['cost'])==cost;assert D(ledger['profit'])==revenue-cost;totalrev+=revenue;totalprofit+=revenue-cost
    prev[provider]=a['active_leases']
   assert m['active_leases']+m['outside_buyers']==m['cumulative_buyers'];assert sum(m['plan_active_leases'].values())==m['active_leases']
   rows=m['buyer_behavior'];a=m['providers']['aeza'];assert sum(r['decisions'] for r in rows)==m['cumulative_buyers'];assert sum(r['aeza_active'] for r in rows)==a['active_leases']
   assert sum(r['exogenous_exits']+r['choice_exits'] for r in rows)==a['churn_to_outside'];assert sum(r['switches_out'] for r in rows)==a['switches_out']
   for r in rows:
    assert sum(r['reasons'].values())==r['decisions'];assert r['aeza_active']==r['aeza_at_risk']+r['first_leases']+r['market_returns']+r['switches_in']-r['switches_out']-r['exogenous_exits']-r['choice_exits']
    assert r['loss_rate']==((r['exogenous_exits']+r['choice_exits']+r['switches_out'])/r['aeza_at_risk'] if r['aeza_at_risk'] else None)
   if not census:continue
   # Independently rebuild every buyer and decision; all state is private to this verifier.
   old_len=len(buyers)
   for ident in range(old_len,old_len+m['arrivals']):
    target=draw(s['seed'],ident,0,0)*sum(g['weight'] for g in s['segments']);acc=0;seg=len(s['segments'])-1
    for i,g in enumerate(s['segments']):
     acc+=g['weight']
     if acc>target:seg=i;break
    g=s['segments'][seg];budget=g['budget_min']+draw(s['seed'],ident,0,1)*(g['budget_max']-g['budget_min']);buyers.append({'id':ident,'segment':seg,'budget':budget,'plan':None,'ever':False})
   prices={pid:float(v) for pid,v in m['exact_prices_rub'].items()};limits=[money(s['aeza_capacity'][k]) for k in ['vcpu','ram_gb','storage_gb']];used=[D(0)]*3
   for b in buyers:
    p=pm.get(b['plan'])
    if p and p['provider']=='aeza':used=[x+y for x,y in zip(used,resources(p))]
   counts=defaultdict(Counter);segments=defaultdict(Counter);competitor_counts=defaultdict(Counter);mf=Counter();beta={'low':.5,'medium':2,'high':5}[run['sensitivity']]
   for b in buyers:
    old=pm.get(b['plan']);fromp=old['provider'] if old else 'outside';g=s['segments'][b['segment']]
    if fromp=='aeza':used=[x-y for x,y in zip(used,resources(old))]
    free=[x-y for x,y in zip(limits,used)];fits=lambda p:all(x<=y for x,y in zip(resources(p),free))
    pen=lambda price:float(money(price)*int(beta*2))/float(money(b['budget'])*2)
    score=lambda p:2+s['preference_strength']*preference(s['seed'],b['id'],p['provider'])-pen(prices[p['id']])-(s['switching_cost_utility'] if old and fromp!=p['provider'] else 0)
    stages=[];eligible=[];before_capacity=[]
    for p in plans:
     price=prices.get(p['id']);stage=0
     if policy(s,p,price):
      stage=1
      if all(p[k]>=g[k] for k in ['vcpu','ram_gb','disk_gb']):
       stage=2
       if not g['region'] or region_matches(p['regions'],g['region']):
        stage=3
        if price<=b['budget']:
         stage=4;before_capacity.append(p)
         if p['provider']!='aeza' or fits(p):stage=5;eligible.append(p)
     if p['provider']=='aeza':stages.append(stage)
    departure=sum(x['monthly_departure_probability'][mo] for x in s['external_models'])/len(s['external_models']) if s['external_models'] else s['monthly_departure_probability']
    forced=bool(old) and draw(s['seed'],b['id'],mo,2)<departure
    chosen=None;best=s['outside_option_utility']
    if not forced:
     for p in eligible:
      u=score(p)
      if u>best:chosen=p;best=u
    to=chosen['provider'] if chosen else 'outside'
    if forced:reason='exogenous_departure_unknown_cause'
    elif to=='aeza':reason='aeza_retained' if fromp=='aeza' else 'aeza_best_available_utility'
    else:
     stage=max(stages,default=0)
     if stage<5:reason=['aeza_unavailable_or_policy','aeza_resources','aeza_region','aeza_budget','aeza_capacity'][stage]
     elif chosen:
      own=max([p for p in eligible if p['provider']=='aeza'],key=score);eq=score(own)+pen(prices[own['id']])-pen(prices[chosen['id']]);reason='competitor_equal_utility_id_tiebreak' if score(own)==score(chosen) else 'competitor_price_decisive_at_equal_price' if prices[chosen['id']]<prices[own['id']] and eq>score(chosen) else 'competitor_utility_preference_or_switching'
     else:reason='aeza_utility_below_outside'
    seg=segments[b['segment']];counts[b['segment']][reason]+=1;unique[reason].add(b['id']);seg['decisions']+=1;seg['aeza_at_risk']+=fromp=='aeza';seg['aeza_active']+=to=='aeza'
    if to=='aeza' and fromp!='aeza':seg['switches_in' if old else 'market_returns' if b['ever'] else 'first_leases']+=1
    if fromp=='aeza' and to!='aeza':seg['switches_out' if chosen else 'exogenous_exits' if forced else 'choice_exits']+=1
    if chosen and to!='aeza':competitor_counts[b['segment']][to]+=1
    if fromp!=to:mf[(fromp,to)]+=1;flow_events[(fromp,to)]+=1;flow_ids[(fromp,to)].add(b['id'])
    if chosen and to=='aeza':used=[x+y for x,y in zip(used,resources(chosen))]
    for ex in [x for x in m['decisions'] if x['buyer_id']==b['id']]:
     assert ex['from']==fromp and ex['to']==to;assert ex['chosen_plan_id']==(chosen['id'] if chosen else None)
     assert math.isclose(ex['budget_rub'],b['budget'],rel_tol=1e-14)
     if chosen:
      assert math.isclose(ex['utility'],score(chosen),abs_tol=1e-12)
      assert math.isclose(ex['price_penalty'],pen(prices[chosen['id']]),abs_tol=1e-12)
      assert ex['switching_penalty']==(s['switching_cost_utility'] if old and fromp!=to else 0)
      assert math.isclose(ex['provider_preference'],s['preference_strength']*preference(s['seed'],b['id'],to),abs_tol=1e-12)
     assert ex['eligible_offers']==len(before_capacity)
     competitors=[p for p in before_capacity if p['provider']!='aeza'];best_comp=max(competitors,key=score) if competitors else None
     assert ex['best_competitor_plan_id']==(best_comp['id'] if best_comp else None)
     if best_comp:assert math.isclose(ex['best_competitor_utility'],score(best_comp),abs_tol=1e-12)
     if forced:assert ex['reason']=='exogenous_departure'
    b['plan']=chosen['id'] if chosen else None;b['ever']|=bool(chosen);checks+=1
   for row in rows:
    idx=row['segment_index'];assert row['competitor_choices']==dict(competitor_counts[idx]);assert row['segment']==s['segments'][idx]['name'];assert row['reasons']==dict(counts[idx]),(run['strategy'],mo,idx,row['reasons'],counts[idx])
    for k,n in segments[idx].items():assert row[k]==n,(k,row[k],n)
   assert {(f['from'],f['to']):f['events'] for f in m['flows']}==dict(mf)
   assert all(math.isclose(float(v),m['aeza_capacity_usage'][k],abs_tol=1e-8) for v,k in zip(used,['vcpu','ram_gb','storage_gb']))
  assert D(run['exact_totals_rub']['revenue'])==totalrev;assert D(run['exact_totals_rub']['profit'])==totalprofit
  if census:
   assert run['behavior_unique_clients']=={k:len(v) for k,v in unique.items()}
   assert {(f['from'],f['to']):(f['events'],f['unique_buyers']) for f in run['flows']}=={k:(n,len(flow_ids[k])) for k,n in flow_events.items()}
 return checks

def wire(v,to_rub):
 if isinstance(v,list):return [wire(x,to_rub) for x in v]
 if isinstance(v,dict):return {k.replace('_usd','_rub') if to_rub else k.replace('_rub','_usd'):x if k in ['details','fx','valuation_fx'] else wire(x,to_rub) for k,x in v.items()}
 return v
base=request('/example');catalog=request('/catalog');base['seed']=42;base['buyers_per_month']=100;base['sensitivities']=['low','medium','high'];base['strategies']=[{'name':name,'aeza_price_multipliers':[factor]*base['months']} for name,factor in [('baseline',1),('rise_36',1.36),('cut_28',.72)]]
rows=[];case_list=[]
for name in ['akash','flat','no_departure','departure_30','limited_capacity','high_cost','cloud_competition','competitor_shock','competitive_switches','recovery']:
 s=copy.deepcopy(base)
 if name=='flat':s['arrival_weights']=[1]*s['months']
 if name=='no_departure':s['monthly_departure_probability']=0
 if name=='departure_30':s['monthly_departure_probability']=.3
 if name=='limited_capacity':s['aeza_capacity'].update(vcpu=24,ram_gb=48,storage_gb=720)
 if name=='high_cost':s['aeza_variable_cost_rub']*=2;s['aeza_fixed_cost_rub']*=2
 if name in ['cloud_competition','competitor_shock','competitive_switches','recovery']:
  s['accepted_service_types']=['vps','cloud_vm']
  for g in s['segments']:g['budget_min']=10000;g['budget_max']=10000
 if name=='competitor_shock':
  selected=min([p for p in catalog if (p.get('details') or {}).get('service_type')=='cloud_vm'],key=lambda p:p['monthly_rub']);s['market_events']=[{'plan_id':selected['id'],'starts_month':3,'price_multiplier':.7,'available':True,'strategy':None}]
 if name in ['competitive_switches','recovery']:
  s['switching_cost_utility']=0
  selected=min([p for p in catalog if p['provider']=='VK Cloud'],key=lambda p:p['monthly_rub'])
  s['market_events']=[{'plan_id':selected['id'],'starts_month':3,'price_multiplier':.7,'available':True,'strategy':None}]
 if name=='recovery':s['strategies'][2]['aeza_price_multipliers']=[1,1,1,1,.72,.72]
 if not args.cases or name in args.cases.split(','):case_list.append((name,s))
for name,s in case_list:
 r=request('/simulate',s);assert r==request('/simulate',s);checks=check_report(r,catalog);save(name+'-request.json',s);save(name+'-response.json',r)
 # Compare the live HTTP report with standalone Rust CLI, bypassing all HTTP currency adapters.
 cli_s=out/(name+'-cli-scenario.json');cli_c=out/'cli-catalog.json';save(cli_s.name,wire(s,False));save(cli_c.name,wire(catalog,False))
 direct=json.loads(subprocess.check_output([args.binary,'--scenario',str(cli_s),str(cli_c)]));assert wire(direct,True)==r,'HTTP versus CLI mismatch'
 verified=[]
 for selected in ['baseline','rise_36','cut_28']:
  payload={'scenario':s,'strategy':selected,'sensitivity':'medium'};a=request('/behavior',payload);a2=request('/behavior',payload);assert a==a2
  # All recommendation candidates must actually reproduce when separately submitted.
  for candidate in a['report']['runs']:
   rs=copy.deepcopy(a['report']['scenario']);rs['strategies']=[x for x in rs['strategies'] if x['name']==candidate['strategy']];rs['market_events']=[x for x in rs['market_events'] if not x.get('strategy') or x['strategy']==candidate['strategy']]
   actual=request('/simulate',rs);assert actual['runs'][0]==candidate
  focused=copy.deepcopy(s);focused['sensitivities']=['medium'];focused['strategies']=[x for x in s['strategies'] if x['name']==selected];f=request('/simulate',focused);assert f['runs'][0]['months']==a['report']['runs'][0]['months']
  check_report(a['report'],catalog,census=False)
  best=max(D(c['exact_totals_rub']['profit']) for c in a['candidates']);chosen=next(c for c in a['candidates'] if c['strategy']==a['recommended_strategy']);assert D(chosen['exact_totals_rub']['profit'])==best
  for action in a['diagnostic_actions']:assert action['decision_events']==sum(row['reasons'].get(action['reason'],0) for m in a['report']['runs'][0]['months'] for row in m['buyer_behavior'])>0
  verified.append({'selected':selected,'recommended':a['recommended_strategy'],'candidates':a['candidates']});save(name+'-'+selected+'-recommendations.json',a)
 rows.append({'case':name,'passed':True,'independently_checked_buyer_decisions':checks,'recommendations':verified,'results':[{'strategy':run['strategy'],'sensitivity':run['sensitivity'],'final_active':run['months'][-1]['providers']['aeza']['active_leases'],'revenue_rub':run['aeza_total_revenue_rub'],'profit_rub':run['aeza_total_profit_rub'],'first':sum(m['providers']['aeza']['first_leases'] for m in run['months']),'returns':sum(m['providers']['aeza']['returning_leases'] for m in run['months']),'loss_events':sum(m['providers']['aeza']['churn_to_outside']+m['providers']['aeza']['switches_out'] for m in run['months']),'switches_in':sum(m['providers']['aeza']['switches_in'] for m in run['months']),'switches_out':sum(m['providers']['aeza']['switches_out'] for m in run['months'])} for run in r['runs']]})
 print(name,'PASS',checks,'buyer decisions',flush=True)
 save('results.json',rows)
print('PASS:',len(rows)*9,'primary runs;',sum(r['independently_checked_buyer_decisions'] for r in rows),'independent buyer decisions;',len(rows)*3,'recommendation requests x 4 separate replays',flush=True)
