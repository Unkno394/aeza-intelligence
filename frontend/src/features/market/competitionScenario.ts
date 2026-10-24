import type {Scenario} from './api';
export type CompetitionMode = 'control'|'competitor_cut'|'entry';
/** Controlled schedules all share the same buyers, costs, seed and market events. */
export function competitionScenario(draft: Scenario, mode: CompetitionMode, competitorId: string, region: string): Scenario {
  if (draft.months < 5) throw new Error('Для сравнения, включая возврат с пятого месяца, нужно не меньше 5 месяцев.');
  const scenario = structuredClone(draft);
  scenario.strategies = [
    {name:'baseline', aeza_price_multipliers:Array(draft.months).fill(1)},
    {name:'rise_30', aeza_price_multipliers:Array.from({length:draft.months},(_,i)=>i<2?1:1.3)},
    {name:'cut_20', aeza_price_multipliers:Array.from({length:draft.months},(_,i)=>i<2?1:.8)},
    {name:'cut_40', aeza_price_multipliers:Array.from({length:draft.months},(_,i)=>i<2?1:.6)},
    {name:'recovery', aeza_price_multipliers:Array.from({length:draft.months},(_,i)=>i<2?1:i<4?1.3:.7)},
  ];
  if (region) scenario.segments.forEach(s=>{s.region=region;});
  // Keep Market Intelligence observations and unrelated scenario events intact.
  // A synthetic experiment replaces only the selected tariff's event schedule.
  const otherEvents=(draft.market_events??[]).filter(event=>event.plan_id!==competitorId);
  const selectedEvents=mode==='competitor_cut' ? [{plan_id:competitorId,starts_month:3,price_multiplier:.7,available:true,strategy:null}]
    : mode==='entry' ? [{plan_id:competitorId,starts_month:1,price_multiplier:1,available:false,strategy:null},{plan_id:competitorId,starts_month:3,price_multiplier:1,available:true,strategy:null}] : (draft.market_events??[]).filter(event=>event.plan_id===competitorId);
  scenario.market_events=[...otherEvents,...selectedEvents];
  return scenario;
}
