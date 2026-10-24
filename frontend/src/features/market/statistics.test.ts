import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {MarketApi,marketCurrencyWire,type Scenario} from './api';
const s:Scenario=JSON.parse(readFileSync(new URL('../../../../market-simulator/data/example.json',import.meta.url),'utf8'));s.currency='RUB';
test('statistics posts actual settings and keeps RUB output fields without mean profit',async()=>{
 const before=structuredClone(s),old=globalThis.fetch;
 globalThis.fetch=async(url,init)=>{assert.equal(url,'/api/market/v2/statistics');assert.deepEqual(JSON.parse(String(init?.body)),{scenario:marketCurrencyWire(s,true),sensitivity:'high'});return new Response(JSON.stringify({currency:'RUB',seed_count:100,strategies:[{profit:{min_rub:-100,max_rub:300,sample_sd_rub:50},loss_probability_percent:29}]}),{status:200});};
 try{const r=await new MarketApi().statistics(s,'high');assert.equal(r.strategies[0].profit.min_rub,-100);assert.equal(r.strategies[0].profit.sample_sd_rub,50);assert.equal(r.strategies[0].loss_probability_percent,29);assert(!('mean_rub' in r.strategies[0].profit));assert.deepEqual(s,before);}finally{globalThis.fetch=old;}
});
test('statistics API errors propagate; no fabricated results on capacity limits',async()=>{const old=globalThis.fetch;globalThis.fetch=async()=>new Response(JSON.stringify({error:'statistics batch already running'}),{status:429});try{await assert.rejects(new MarketApi().statistics(s,'medium'),/batch already running/);}finally{globalThis.fetch=old;}});
