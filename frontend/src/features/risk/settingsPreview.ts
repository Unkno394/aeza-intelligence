import type { RiskLevel, RiskResult, Settings } from '../../types';

export type RiskBand = Extract<RiskLevel, 'low' | 'medium' | 'high'>;
export type RiskDistribution = Record<RiskBand, number> & {
  limited: number;
  insufficient: number;
  movedUp: number;
  movedDown: number;
  total: number;
};

const rank: Record<RiskBand, number> = { low: 0, medium: 1, high: 2 };

/** Reweights the factors already returned by Rust using the draft configuration. */
export function previewRiskResult(risk: RiskResult, settings: Settings): RiskResult {
  const available = risk.factors.filter(factor => factor.available);
  const totalWeight = available.reduce((sum, factor) => sum + settings.weights[factor.key], 0);
  const score = totalWeight > 0
    ? Math.round(available.reduce((sum, factor) => sum + factor.severity * settings.weights[factor.key] / totalWeight * 100, 0))
    : 0;
  const level: RiskLevel = risk.assessmentStatus === 'insufficient_data'
    ? 'insufficient'
    : risk.assessmentStatus === 'limited_data'
      ? 'limited'
      : score >= settings.highThreshold ? 'high' : score >= settings.mediumThreshold ? 'medium' : 'low';
  return { ...risk, score: Math.max(0, Math.min(100, score)), level };
}

export function previewRiskDistribution(risks: Record<string, RiskResult>, settings: Settings): RiskDistribution {
  const result: RiskDistribution = { low: 0, medium: 0, high: 0, limited: 0, insufficient: 0, movedUp: 0, movedDown: 0, total: 0 };
  for (const risk of Object.values(risks)) {
    const preview = previewRiskResult(risk, settings);
    result[preview.level] += 1;
    result.total += 1;
    if (risk.level in rank && preview.level in rank) {
      if (rank[preview.level as RiskBand] > rank[risk.level as RiskBand]) result.movedUp += 1;
      if (rank[preview.level as RiskBand] < rank[risk.level as RiskBand]) result.movedDown += 1;
    }
  }
  return result;
}
