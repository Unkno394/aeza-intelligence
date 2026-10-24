import test from 'node:test';
import assert from 'node:assert/strict';
import {sameScenario} from './scenario';
test('Rust report field ordering does not falsely mark identical settings stale', () => {
  assert(sameScenario({seed:42,plans:[{id:'s',price:5}]},{plans:[{price:5,id:'s'}],seed:42}));
});
test('a changed price, horizon or strategy schedule invalidates the displayed report', () => {
  assert(!sameScenario({price:5},{price:6}));
  assert(!sameScenario({months:6},{months:7}));
  assert(!sameScenario({prices:[1,2]},{prices:[2,1]}));
});

import {buildMarketScenario,priceChangeMultiplier} from './scenario';
import type {Scenario} from './api';
import {readFileSync} from 'node:fs';
const input:Scenario=JSON.parse(readFileSync(new URL('../../../../market-simulator/data/example.json',import.meta.url),'utf8'));

test('signed price changes preserve direction and exact three-tier prices',()=>{
  for (const [percent,multiplier,prices] of [
    [0,1,[5,10,20]],[36,1.36,[6.8,13.6,27.2]],[-28,.72,[3.6,7.2,14.4]],[15,1.15,[5.75,11.5,23]],[-15,.85,[4.25,8.5,17]],
    [10,1.1,[5.5,11,22]],[20,1.2,[6,12,24]],[40,1.4,[7,14,28]],
    [-10,.9,[4.5,9,18]],[-20,.8,[4,8,16]],[-40,.6,[3,6,12]],
  ] as [number,number,number[]][]){
    assert.equal(priceChangeMultiplier(percent),multiplier);
    const s=buildMarketScenario(input,input,['custom'],percent);
    assert.deepEqual(s.strategies[0].aeza_price_multipliers,Array(input.months).fill(multiplier));
    s.aeza_plans.forEach((p,i)=>assert(Math.abs(p.monthly_usd*multiplier-prices[i])<1e-10));
  }
});
test('custom strategy changes only the strategy schedule, without resetting edited tariffs or buyers',()=>{
  const edited=structuredClone(input);edited.aeza_plans[0].monthly_usd=7;edited.aeza_plans[1].monthly_usd=13;edited.aeza_plans[2].monthly_usd=29;
  const before=structuredClone(edited);
  for(const percent of [-40,-15,0,15,40]){
    const result=buildMarketScenario(edited,input,['custom'],percent);
    const {strategies:_,...rest}=result;
    const {strategies:__,...original}=before;
    assert.deepEqual(rest,original);
    assert.deepEqual(edited,before);
    assert.deepEqual(result.aeza_plans.map(p=>p.monthly_usd),[7,13,29]);
  }
});
test('baseline and neutral custom strategy have identical multipliers; presets remain unchanged',()=>{
  const s=buildMarketScenario(input,input,['baseline','custom','increase_20','discount_20'],0);
  assert.deepEqual(s.strategies[0].aeza_price_multipliers,s.strategies[1].aeza_price_multipliers);
  assert.deepEqual(s.strategies[2].aeza_price_multipliers,Array(input.months).fill(1.2));
  assert.deepEqual(s.strategies[3].aeza_price_multipliers,Array(input.months).fill(.8));
});

import {competitionScenario} from './competitionScenario';
test('competition experiments keep seed, edited prices and common inputs; shocks start in month three',()=>{
 const original=structuredClone(input);original.aeza_plans[0].monthly_usd=7;
 const s=competitionScenario(original,'competitor_cut','index-318','Frankfurt');
 assert.equal(s.seed,42);assert.equal(s.aeza_plans[0].monthly_usd,7);assert.deepEqual(s.arrival_weights,original.arrival_weights);assert.equal(s.aeza_variable_cost_usd,original.aeza_variable_cost_usd);
 assert.deepEqual(s.strategies.find(x=>x.name==='rise_30')!.aeza_price_multipliers,[1,1,1.3,1.3,1.3,1.3]);
 assert.deepEqual(s.strategies.find(x=>x.name==='recovery')!.aeza_price_multipliers,[1,1,1.3,1.3,.7,.7]);
 assert.deepEqual(s.market_events,[{plan_id:'index-318',starts_month:3,price_multiplier:.7,available:true,strategy:null}]);
 assert(s.segments.every(x=>x.region==='Frankfurt'));assert.equal(original.segments[0].region,null);assert.equal(original.aeza_plans[0].monthly_usd,7);
});
test('entry uses an observed plan ID and changes only scenario availability; short horizons rejected',()=>{
 const s=competitionScenario(input,'entry','index-318','');
 assert.deepEqual(s.market_events?.map(x=>[x.plan_id,x.starts_month,x.available,x.price_multiplier]),[['index-318',1,false,1],['index-318',3,true,1]]);
 assert.deepEqual(s.segments,input.segments);assert.deepEqual(s.aeza_plans,input.aeza_plans);
 assert.throws(()=>competitionScenario({...input,months:4},'control','index-318','Frankfurt'));
});


import {marketCurrencyWire} from './api';
test('RUB wire adapter preserves original currencies and maps every legacy monetary result',()=>{
 const value={currency:'RUB',aeza_variable_cost_usd:200,aeza_plans:[{monthly_usd:535.14,details:{normalized_monthly_usd:'6.644561977',normalized_monthly_rub:'535.140000000','original_price':{amount:'5.93',currency:'EUR'}}}],providers:{aeza:{revenue_usd:1000}}};
 const wire=marketCurrencyWire(value,true) as any;
 assert.equal(wire.aeza_variable_cost_rub,200);assert.equal(wire.aeza_plans[0].monthly_rub,535.14);assert.equal(wire.aeza_plans[0].details.original_price.currency,'EUR');assert.equal(wire.providers.aeza.revenue_rub,1000);assert.deepEqual(marketCurrencyWire(wire,false),value);
});

// Keep the default npm test entry point aware of API feature tests.
import './behavior.test';
import './statistics.test';
