// JSON object field order is not semantic; Rust serializes Report through a sorted map.
export function canonicalJSON(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJSON).join(',')}]`;
  if (value !== null && typeof value === 'object') {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().map(k => `${JSON.stringify(k)}:${canonicalJSON(record[k])}`).join(',')}}`;
  }
  return JSON.stringify(value);
}
export function sameScenario(a: unknown, b: unknown): boolean { return canonicalJSON(a) === canonicalJSON(b); }

import type {Plan,Scenario} from './api';

/** The simulation starts with up to three low-to-high vCPU offers for one city.
 * Prefer shared CPU when the snapshot contains that class; otherwise keep all city offers. */
export function initialAezaOfferSet(official: Plan[], city: string): Plan[] {
  const inCity=official.filter(plan=>plan.regions.includes(city)).sort((a,b)=>a.vcpu-b.vcpu);
  const hasShared=inCity.some(plan=>plan.details?.cpu_class==='shared');
  return hasShared?inCity.filter(plan=>plan.details?.cpu_class==='shared'):inCity;
}

export function initialAezaPlans(official: Plan[], city: string): Plan[] {
  return initialAezaOfferSet(official,city).slice(0,3);
}

/** Signed change, not percentage of the original price: +15 -> 1.15, -15 -> 0.85. */
export function priceChangeMultiplier(changePercent: number): number {
  if (!Number.isFinite(changePercent)) throw new Error('Price change must be finite');
  return (100 + changePercent) / 100;
}

/** Preserve the edited market inputs; only assemble the selected price schedules. */
export function buildMarketScenario(draft: Scenario, template: Scenario, names: string[], changePercent: number): Scenario {
  return {...draft, strategies: names.map(name => {
    if (name === 'custom') return {name, aeza_price_multipliers: Array(draft.months).fill(priceChangeMultiplier(changePercent))};
    const source = template.strategies.find(s => s.name === name);
    if (!source && name === 'baseline') return {name, aeza_price_multipliers: Array(draft.months).fill(1)};
    if (!source) throw new Error(`Unknown strategy: ${name}`);
    return {name, aeza_price_multipliers: Array.from({length:draft.months}, (_, i) => source.aeza_price_multipliers[Math.min(i,source.aeza_price_multipliers.length-1)])};
  })};
}
