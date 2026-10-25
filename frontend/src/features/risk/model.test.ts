import {test} from 'node:test';
import assert from 'node:assert/strict';
import {createDataset,defaults} from '../../mocks/data';
import type {Account,Dataset,Service,Settings,Transaction} from '../../types';
import {calculateRisk} from './model';

const account: Account = {
  id: '1', name: 'Test', email: 'test@example.test', balanceCents: 0,
  totalReplenishedCents: 0,
};
const asOf = '2026-10-25T00:00:00.000Z';
const service = (overrides: Partial<Service> = {}): Service => ({
  id: 's1', accountId: account.id, name: 'vps', region: 'DE',
  priceCents: 1000, renewalPriceCents: 1000, paymentTermSeconds: 30 * 86400,
  renewsAt: new Date(Date.parse(asOf) + 86400000).toISOString(),
  autoRenew: true, status: 'active', ...overrides,
});
const dataset = (overrides: Partial<Dataset> = {}): Dataset => ({
  accounts: [account], services: [service()], transactions: [], asOf, snapshots: [], ...overrides,
});
const weights = (overrides: Partial<Settings['weights']> = {}): Settings['weights'] => ({
  ...defaults.weights, ...overrides,
});

test('mock fixture has exact counts and valid customer relationships',()=>{
  const d=createDataset();
  assert.equal(d.accounts.length,10); assert.equal(d.services.length,30); assert.equal(d.transactions.length,100);
  for(const s of d.services) assert.ok(d.accounts.some(a=>a.id===s.accountId));
  for(const t of d.transactions){
    assert.ok(d.accounts.some(a=>a.id===t.accountId));
    if(t.serviceId) assert.ok(d.services.some(s=>s.id===t.serviceId&&s.accountId===t.accountId));
    assert.ok(Number.isInteger(t.amountCents));
  }
  assert.equal(d.snapshots.length,0);
});

test('score stays in range, normalized weights are scale invariant, and factors sum to score',()=>{
  const d=createDataset();
  for(const a of d.accounts){
    const result=calculateRisk(a,d,defaults);
    assert.ok(result.score>=0&&result.score<=100);
    assert.equal(result.score,Math.round(result.factors.reduce((sum,f)=>sum+f.points,0)));
    const doubled={...defaults,weights:Object.fromEntries(Object.entries(defaults.weights).map(([key,value])=>[key,value*2])) as Settings['weights']};
    assert.equal(calculateRisk(a,d,doubled).score,result.score);
  }
});

test('a factor weight of zero disables its contribution and changing another weight changes the score',()=>{
  const d=dataset({services:[service({autoRenew:false}),service({id:'s2',autoRenew:true,renewsAt:new Date(Date.parse(asOf)+2*86400000).toISOString()})]});
  const baseline=calculateRisk(account,d,defaults);
  const disabled=calculateRisk(account,d,{...defaults,weights:weights({noAutoProlong:0,paymentCoverage:100,prolongWait:0,failedProlong:0,expiringRevenue:0})});
  assert.equal(disabled.factors.find(f=>f.key==='noAutoProlong')?.points,0);
  assert.notEqual(disabled.score,baseline.score);
});

test('balance covers upcoming payments while top-ups and referral totals are not treated as available funds',()=>{
  const d=dataset({accounts:[{...account,totalReplenishedCents:undefined}],transactions:[{
    id:'topup',accountId:account.id,amountCents:100000,type:'topup',status:'completed',createdAt:asOf,
  }]});
  const poor=calculateRisk({...account,totalReplenishedCents:undefined},d,defaults);
  const rich=calculateRisk({...account,balanceCents:1000,totalReplenishedCents:100000},d,defaults);
  assert.equal(poor.spendableBalanceCents,0);
  assert.equal(poor.topupsCents,100000);
  assert.equal(rich.spendableBalanceCents,1000);
  assert.equal(rich.topupsCents,100000);
  assert.ok(rich.score<poor.score);
});

test('all payment terms project the number of renewals due inside the analysis window',()=>{
  const cases:[string,number,number][]=[
    ['hour',3600,696], ['week',7*86400,5], ['month',30*86400,1],
    ['quarter',91*86400,1], ['half-year',182*86400,1], ['year',365*86400,1],
  ];
  for(const [label,seconds,renewals] of cases){
    const result=calculateRisk(account,dataset({services:[service({paymentTermSeconds:seconds,renewalPriceCents:125})]}),{...defaults,renewalWindowDays:30});
    assert.equal(result.upcomingCents,renewals*125,label);
  }
});

test('blocked and deleted services are excluded while active, suspended, and prolong_wait services remain billable',()=>{
  const d=dataset({services:[
    service({id:'active',priceCents:100,renewalPriceCents:100}),
    service({id:'suspended',priceCents:200,renewalPriceCents:200,status:'suspended'}),
    service({id:'waiting',priceCents:300,renewalPriceCents:300,status:'prolong_wait'}),
    service({id:'blocked',priceCents:400,renewalPriceCents:400,status:'blocked'}),
    service({id:'deleted',priceCents:500,renewalPriceCents:500,status:'cancelled'}),
  ]});
  const result=calculateRisk(account,d,defaults);
  assert.equal(result.monthlyCents,600);
  assert.equal(result.upcomingCents,600);
  assert.equal(result.factors.find(f=>f.key==='prolongWait')?.value,1/3);
});

test('empty customers and customers without upcoming payments are not labelled low risk',()=>{
  const noServices=calculateRisk(account,dataset({services:[]}),defaults);
  assert.equal(noServices.level,'insufficient');
  assert.equal(noServices.assessmentStatus,'insufficient_data');
  assert.equal(noServices.availableFactors,0);

  const noDue=calculateRisk(account,dataset({services:[service({renewsAt:new Date(Date.parse(asOf)+60*86400000).toISOString()})]}),defaults);
  assert.equal(noDue.level,'limited');
  assert.equal(noDue.assessmentStatus,'limited_data');
  assert.equal(noDue.paymentCoverage,null);
});

test('unknown outcomes do not count as failed prolongs and explanations identify contributing factors',()=>{
  const unknown:Transaction={id:'tx',accountId:account.id,serviceId:'s1',billingType:'prolong',amountCents:1000,type:'payment',status:'unknown',createdAt:asOf};
  const result=calculateRisk(account,dataset({transactions:[unknown]}),defaults);
  const failed=result.factors.find(f=>f.key==='failedProlong')!;
  assert.equal(failed.available,false);
  assert.equal(failed.points,0);
  assert.ok(result.factors.filter(f=>f.available).every(f=>f.explanation.trim().length>0));
  assert.equal(result.factors.find(f=>f.key==='priceRisk')?.available,false);
});

test('hourly renewals use the per-period price while recurring exposure uses monthly price',()=>{
  const result=calculateRisk(account,dataset({services:[service({priceCents:72000,renewalPriceCents:100,paymentTermSeconds:3600,renewsAt:new Date(Date.parse(asOf)+3600000).toISOString()})]}),{...defaults,renewalWindowDays:1});
  assert.equal(result.upcomingCents,2300);
  assert.equal(result.monthlyCents,72000);
});

test('zero balances, negative balances, and very large amounts never escape score bounds',()=>{
  for(const balanceCents of [0,-500,Number.MAX_SAFE_INTEGER]){
    const result=calculateRisk({...account,balanceCents},dataset({services:[service({priceCents:1e100,renewalPriceCents:1e100})]}),defaults);
    assert.ok(result.score>=0&&result.score<=100);
    assert.ok(Number.isFinite(result.score));
  }
});

test('risk thresholds are inclusive when the assessment has enough evidence',()=>{
  const wellFunded={...account,balanceCents:100000};
  const complete=calculateRisk(wellFunded,dataset({services:[service({autoRenew:false})]}),defaults);
  assert.equal(complete.assessmentStatus,'sufficient_data');
  assert.equal(calculateRisk(wellFunded,dataset({services:[service({autoRenew:false})]}),{...defaults,mediumThreshold:complete.score,highThreshold:complete.score+1}).level,'medium');
  assert.equal(calculateRisk(wellFunded,dataset({services:[service({autoRenew:false})]}),{...defaults,mediumThreshold:1,highThreshold:complete.score}).level,'high');
});
