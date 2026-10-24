import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import {MarketApi,marketCurrencyWire,type Scenario} from './api';
const s:Scenario=JSON.parse(readFileSync(new URL('../../../../market-simulator/data/example.json',import.meta.url),'utf8'));s.currency='RUB';
test('behavior posts unchanged scenario, focused strategy and sensitivity over RUB transport',async()=>{
 const original=structuredClone(s);const old=globalThis.fetch;
 globalThis.fetch=async(url,init)=>{assert.equal(url,'/api/market/v2/behavior');assert.equal(init?.method,'POST');const body=JSON.parse(String(init?.body));assert.deepEqual(body,{scenario:marketCurrencyWire(s,true),strategy:s.strategies[0].name,sensitivity:'high'});return new Response(JSON.stringify({currency:'RUB',candidates:[{profit_rub:-100,delta_profit_rub:25}],report:{runs:[{behavior_unique_clients:{aeza_budget:10}}]}}),{status:200});};
 try {const result=await new MarketApi().behavior(s,s.strategies[0].name,'high');assert.equal(result.candidates[0].profit_usd,-100);assert.equal(result.candidates[0].delta_profit_usd,25);assert.equal(result.report.runs[0].behavior_unique_clients?.aeza_budget,10);assert.deepEqual(s,original);}finally{globalThis.fetch=old;}
});
test('behavior API failures are displayed as errors, never fabricated results',async()=>{const old=globalThis.fetch;globalThis.fetch=async()=>new Response(JSON.stringify({error:'simulation capacity reached'}),{status:429});try{await assert.rejects(new MarketApi().behavior(s,'base','medium'),/simulation capacity reached/);}finally{globalThis.fetch=old;}});
import {number2,input2,metadataDisplay} from './format';
test('all market presentation values use at most hundredths without mutating source facts',()=>{
 assert.equal(number2(535.140397),'535,14');assert.equal(number2('80.5381'),'80,54');assert.equal(number2(-12.346),'-12,35');assert.equal(input2(241.6143),241.61);assert.equal(number2(null),'—');
 const original={amount:'5.933456',rate:80.5381,source:'https://example.com/1.23456',date:'2026-10-24'};const before=structuredClone(original);const display=JSON.parse(metadataDisplay(original));assert.equal(display.amount,'5.93');assert.equal(display.rate,80.54);assert.equal(display.source,original.source);assert.deepEqual(original,before);
});
