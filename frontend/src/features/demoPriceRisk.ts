import type { RiskResult, Settings } from '../types';
import { priceEvidence, priceIncreasePercent, priceOutcome, type PriceImpactEvent } from './priceImpact';

export function demoEvidenceReady(events: PriceImpactEvent[]): boolean {
  return events.length > 0 && priceEvidence(events).stable;
}

/** Recompute an isolated scenario score; the live API score is never modified. */
export function demoPriceRisk(base: RiskResult, settings: Settings, event: PriceImpactEvent | undefined, evidenceReady: boolean): RiskResult {
  if (!event || !evidenceReady || !event.completeBaseline || !event.completeFollowup || !event.renewalDueInWindow) return base;
  const outcome = priceOutcome(event);
  if (outcome === 'pending' || outcome === 'not_due' || outcome === 'unknown') return base;
  const severity = Math.min(1, Math.max(0, priceIncreasePercent(event) / 25)) * (outcome === 'adverse' ? 1 : 0);
  const factors = base.factors.map(factor => factor.key === 'priceRisk'
    ? { ...factor, available: true, value: severity, severity,
        explanation: 'Синтетический сценарий: величина повышения и исход продления. В реальный Risk Score не входит.' }
    : { ...factor });
  const total = factors.reduce((sum, factor) => sum + (factor.available ? settings.weights[factor.key] : 0), 0);
  for (const factor of factors) factor.points = factor.available && total > 0
    ? factor.severity * settings.weights[factor.key] / total * 100 : 0;
  const score = Math.min(100, Math.max(0, Math.round(factors.reduce((sum, factor) => sum + factor.points, 0))));
  const level = base.assessmentStatus === 'insufficient_data' ? 'insufficient'
    : base.assessmentStatus === 'limited_data' ? 'limited'
      : score >= settings.highThreshold ? 'high' : score >= settings.mediumThreshold ? 'medium' : 'low';
  return { ...base, score, level, factors, availableFactors: factors.filter(factor => factor.available).length };
}
