import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {buildMarketScenario} from './scenario';
import type {Scenario,Report} from './api';
const input:Scenario=JSON.parse(readFileSync(new URL('../../../../market-simulator/data/example.json',import.meta.url),'utf8'));
const base=process.env.MARKET_API_URL??'http://127.0.0.1:8091/api/market/v1';
test('real HTTP: UI signed percentages reach Rust unchanged and preserve price/demand direction',async()=>{
  const records:unknown[]=[];
  for(const percent of [-40,-20,-15,-10,0,10,15,20,40]){
    const s=buildMarketScenario(input,input,['baseline','custom'],percent);assert.equal(s.seed,42);
    const response=await fetch(base+'/simulate',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(s)});
    assert.equal(response.status,200);const r=await response.json() as Report;
    assert.deepEqual(r.scenario,s,'Server or frontend changed non-strategy inputs');
    for(const sensitivity of ['low','medium','high']){
      const baseline=r.runs.find(x=>x.strategy==='baseline'&&x.sensitivity===sensitivity)!;
      const custom=r.runs.find(x=>x.strategy==='custom'&&x.sensitivity===sensitivity)!;
      const a=baseline.months[0].providers.aeza.active_leases,b=custom.months[0].providers.aeza.active_leases;
      if(percent>0)assert(b<=a,'Raising own prices made initial demand larger');
      if(percent<0)assert(b>=a,'Lowering own prices made initial demand smaller');
      if(percent===0)assert.deepEqual(custom.months,baseline.months);
      for(const m of custom.months) for(const plan of input.aeza_plans){
        const used=m.aeza_effective_prices_usd?.[plan.id];assert.notEqual(used,undefined,'No engine price trace');
        assert(Math.abs(used!-plan.monthly_usd*(100+percent)/100)<1e-10,'Rust used a wrong tariff price');
      }
    }
    records.push({percent,request:s,response:r});
  }
  if(process.env.PRICE_AUDIT_DIR){mkdirSync(process.env.PRICE_AUDIT_DIR,{recursive:true});writeFileSync(process.env.PRICE_AUDIT_DIR+'/frontend-to-http.json',JSON.stringify(records,null,2));}
});
