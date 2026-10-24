export interface StatisticsPoint {seed:number;profit_rub:number;exact_profit_rub:string;revenue_rub:number;final_active:number;loss_events:number}
export interface StrategyStatistics {strategy:string;profit:{min_rub:number;max_rub:number;median_rub:number;p025_rub:number;p25_rub:number;p75_rub:number;p975_rub:number;sample_sd_rub:number};loss_runs:number;loss_probability_percent:number;loss_probability_ci95_percent:[number,number];sole_wins:number;tied_wins:number;leading_percent:number;leading_ci95_percent:[number,number];points:StatisticsPoint[]}
export interface SeedStatistics {engine_version:string;currency:'RUB';seeds:number[];seed_count:number;sensitivity:Sensitivity;scenario:unknown;catalog_sha256:string;replay_sha256:string;tied_seed_count:number;strategies:StrategyStatistics[];limitations:string[]}
export interface SegmentBehavior {segment_index:number;segment:string;decisions:number;aeza_at_risk:number;aeza_active:number;first_leases:number;market_returns:number;switches_in:number;switches_out:number;exogenous_exits:number;choice_exits:number;loss_rate:number|null;reasons:Record<string,number>;competitor_choices:Record<string,number>}
export interface BehaviorAnalysis {currency:string;recommendation:string;recommended_strategy:string;diagnostic_actions:{reason:string;decision_events:number;text:string;status:string}[];limitations:string[];report:Report;candidates:{strategy:string;final_clients:number;lease_months:number;acquisition_events:number;loss_events:number;at_risk_months:number;revenue_usd:number;profit_usd:number;delta_profit_usd:number;delta_final_clients:number;segment_effects:{segment_index:number;segment:string;lease_months:number;delta_lease_months:number;acquisitions:number;delta_acquisitions:number;losses:number;delta_losses:number}[];final_prices_usd:Record<string,string>}[]}

// Standalone client: copy into a dedicated market feature; existing AnalyzerApi stays independent.
export type Sensitivity = 'low' | 'medium' | 'high';
export interface Fx {date:string;source:string;rub_per_unit:Record<string,string>;rounding:string}
export interface OriginalPrice {amount:string;currency:string;unit:string}
export interface OfferDetails {original_price:OriginalPrice;normalized_monthly_usd:string;normalized_monthly_rub:string;fx:Fx|null;market:string;service_type:string;virtualization:string|null;cpu_class:string|null;vat:string;complete_monthly_payment:boolean;availability:string;billing_hours:number|null;hourly_price:OriginalPrice|null;components:unknown[];source_region:string;notes:string[];egress_gib:number|null}
export interface Capacity {vcpu:number;ram_gb:number;storage_gb:number;origin:string;source:string}
export interface MarketContexts {
 gpu?:{observed_rows:number;quality_ok_rows:number;listing_sample_count:number;providers:Record<string,number>;latest_timestamp:string;source:string;listings:{provider:string;gpu_type:string;gpu_count:number;region:string;is_spot:boolean;original_price:OriginalPrice;price_rub_hour:string;fx_date:string;timestamp:string;source:string}[]};
 akash?:{source:string;daily_points?:number;first_date?:string;last_date?:string;refresh_error?:string;monthly_profile:{months:string[];new_lease_events:number[];normalized_weights?:number[]}};
 api_discovery?:{source:string;selected:string;additional_candidates:string[]};
}
export interface MarketSources {catalog_included:number;prepared_as_of:string;license:string;limitations:string[];fx:Fx;raw_product_count:number;official_aeza_count:number;official_aeza_available:number;snapshot_status:string}
export interface Plan {
  id: string; provider: string; name: string;
  vcpu: number; ram_gb: number; disk_gb: number; monthly_usd: number;
  details?: OfferDetails|null; regions: string[]; origin: string; source: string; observed_at: string;
}
export interface Scenario {
  currency?:'RUB'|'USD';valuation_fx?:Fx|null; seed: number; months: number; buyers_per_month: number; arrival_weights: number[];
  segments: {name: string; weight: number; budget_min: number; budget_max: number; vcpu: number; ram_gb: number; disk_gb: number; region: string | null}[];
  aeza_plans: Plan[];
  strategies: {name: string; aeza_price_multipliers: number[]}[];
  sensitivities: Sensitivity[];
  monthly_departure_probability: number; switching_cost_utility: number;
  outside_option_utility: number; preference_strength: number;
  aeza_variable_cost_usd: number; aeza_fixed_cost_usd: number;
  market_events?: {plan_id: string; starts_month: number; price_multiplier: number; available: boolean; strategy: string|null}[];
  market?:string;accepted_service_types?:string[];required_virtualization?:string|null;required_cpu_class?:string|null;require_complete_monthly_payment?:boolean;aeza_capacity?:Capacity;
  external_models: {slot: 'snapshot'|'historical'; schema_version: 1; model_version: string; source: string; semantics: 'calibrated_monthly_departure_probability'; monthly_departure_probability: number[]}[];
}
export interface Metrics {
  at_risk: number; first_leases: number; returning_leases: number;
  active_leases: number; new_leases: number; renewals: number;
  switches_in: number; switches_out: number; churn_to_outside: number;
  revenue_usd: number; market_share: number; cost_usd: number|null; profit_usd: number|null;
}
export interface Report {
  engine_version: string; replay_sha256: string; catalog_sha256: string;
  scenario: Scenario; warnings: string[];
  runs: {strategy: string; sensitivity: Sensitivity; aeza_total_revenue_usd: number; aeza_total_profit_usd: number;
    behavior_unique_clients?:Record<string,number>; flows: Flow[]; months: {buyer_behavior?:SegmentBehavior[];month: number; aeza_capacity_usage:{vcpu:number;ram_gb:number;storage_gb:number;decisions_blocked:number};billing_ledger_usd:Record<string,Record<string,string>>; effective_prices_usd: Record<string,number>; plan_active_leases:Record<string,number>; flows: Flow[]; decisions: Decision[]; aeza_effective_prices_usd?: Record<string, number>; arrivals: number; cumulative_buyers: number; active_leases: number; outside_buyers: number; providers: Record<string, Metrics>}[]}[];
}
export interface Flow {from:string; to:string; events:number; unique_buyers:number}
export interface Decision {buyer_id:number;from:string;to:string;reason:string;budget_usd:number;chosen_plan_id:string|null;effective_price_usd:number|null;utility:number|null;previous_offer_utility:number|null;outside_utility:number;segment:string;provider_preference:number|null;price_penalty:number|null;switching_penalty:number|null;best_competitor_plan_id:string|null;best_competitor_utility:number|null;eligible_offers:number}
export interface CompetitionAnalysis {
  report: Report; strategy:string; month:number; region:string|null;
  comparisons: {aeza_plan_id:string;aeza_effective_price_usd:number;competitor:Plan;effective_price_usd:number|null;scenario_price_changed:boolean;difference_percent:number|null;comparable:boolean;indicative_difference_percent:number|null;resource_match:string;region_match:string;exclusion:string|null;rank:number|null}[];
  recommendations:{sensitivity:Sensitivity;strategy:string;revenue_usd:number;profit_usd:number;final_share:number;final_prices_usd:Record<string,number>}[];
  explanations:string[];limitations:string[];
}
// Existing view-model property names are retained; public v2 transport uses *_rub exclusively.
export function marketCurrencyWire(value:unknown,toRub:boolean):unknown {
 if(Array.isArray(value))return value.map(v=>marketCurrencyWire(v,toRub));
 if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).map(([k,v])=>[toRub?k.replace(/_usd/g,'_rub'):k.replace(/_rub/g,'_usd'),['details','fx','valuation_fx'].includes(k)?v:marketCurrencyWire(v,toRub)]));
 return value;
}
export class MarketApi {
  constructor(private base = '/api/market/v2') {}
  private async request<T>(path: string, init?: RequestInit): Promise<T> {
    const response = await fetch(this.base + path, init);
    const data = await response.json().catch(() => { throw new Error('API недоступен или вернул некорректный JSON. Проверьте запуск Market API на порту 8091.'); });
    if (!response.ok) throw new Error(data.error ?? `Market API ${response.status}`);
    return ((path==='/contexts'||path==='/statistics')?data:marketCurrencyWire(data,false)) as T;
  }
  competition(scenario: Scenario, strategy:string, month:number, region:string|null, verified_only=true) {
    return this.request<CompetitionAnalysis>('/competition', {method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(marketCurrencyWire({scenario,strategy,month,region,verified_only},true))});
  }
  behavior(scenario:Scenario,strategy:string,sensitivity:Sensitivity) {return this.request<BehaviorAnalysis>('/behavior',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(marketCurrencyWire({scenario,strategy,sensitivity},true))});}
  statistics(scenario:Scenario,sensitivity:Sensitivity){return this.request<SeedStatistics>('/statistics',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(marketCurrencyWire({scenario,sensitivity},true))});}
  sources() { return this.request<MarketSources>('/sources'); }
  contexts() {return this.request<MarketContexts>('/contexts');}
  aezaPlans() { return this.request<Plan[]>('/aeza-plans'); }
  catalog() { return this.request<Plan[]>('/catalog'); }
  example() { return this.request<Scenario>('/example'); }
  simulate(scenario: Scenario, signal?: AbortSignal) {
    // Use safe integer seeds in JS. Abort stops the request; the bounded worker may finish.
    if (!Number.isSafeInteger(scenario.seed) || scenario.seed < 0) throw new Error('seed must be a nonnegative safe integer');
    return this.request<Report>('/simulate', {method: 'POST', headers: {'Content-Type': 'application/json'}, body: JSON.stringify(marketCurrencyWire(scenario,true)), signal});
  }
}
