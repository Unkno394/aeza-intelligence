import type { Settings } from '../../types';

export function validSettings(settings: Settings): boolean {
  const weights = Object.values(settings.weights);
  return weights.every(value => Number.isInteger(value) && value >= 0 && value <= 100)
    && weights.reduce((sum, value) => sum + value, 0) - settings.weights.priceRisk > 0
    && Number.isInteger(settings.mediumThreshold) && settings.mediumThreshold >= 1
    && Number.isInteger(settings.highThreshold) && settings.highThreshold > settings.mediumThreshold && settings.highThreshold <= 100
    && Number.isInteger(settings.renewalWindowDays) && settings.renewalWindowDays >= 1 && settings.renewalWindowDays <= 90
    && Number.isInteger(settings.historicalMinDays) && settings.historicalMinDays >= 7 && settings.historicalMinDays <= 365;
}
