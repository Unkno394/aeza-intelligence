export type ComparisonTier = 'exact' | 'close' | 'reference';

export interface MarketOffer {
  providerCode: string;
  cpuCores: number | null;
  ramGib: number | null;
  diskGib: number | null;
  diskType: string | null;
  region: string | null;
  vatIncluded: boolean | null;
  trafficGib: number | null;
  trafficUnlimited: boolean | null;
  priceRub: number | null;
  priceKind: string;
}

export interface CurrentComparison<T extends MarketOffer> {
  offer: T;
  tier: ComparisonTier;
  reasons: string[];
  differencePercent: number;
  distance: number;
}

const normal = (value: string | null) => value?.trim().toLocaleLowerCase('ru-RU') ?? '';
const standardPrices = new Set(['regular', 'listed_monthly', 'official_catalog']);

export function compareCurrentOffers<T extends MarketOffer>(target: T, offers: T[]): CurrentComparison<T>[] {
  const targetPrice = target.priceRub;
  if (!targetPrice || !Number.isFinite(targetPrice)) return [];

  return offers.flatMap(offer => {
    if (offer.providerCode === target.providerCode || !offer.priceRub || !Number.isFinite(offer.priceRub)) return [];

    const pairs = [[target.cpuCores, offer.cpuCores], [target.ramGib, offer.ramGib], [target.diskGib, offer.diskGib]];
    const knownSpecs = pairs.every(([own, other]) => own !== null && other !== null && own > 0 && other > 0);
    const ratios = knownSpecs ? pairs.map(([own, other]) => other! / own!) : [];
    const exactSpecs = knownSpecs && ratios.every(ratio => Math.abs(ratio - 1) < 0.001);
    const nearSpecs = knownSpecs && ratios.every(ratio => ratio >= 0.5 && ratio <= 2);
    const sameRegion = Boolean(normal(target.region) && normal(target.region) === normal(offer.region));
    const regionCompatible = !target.region || !offer.region || sameRegion;
    const diskKnown = Boolean(normal(target.diskType) && normal(offer.diskType));
    const sameDisk = diskKnown && normal(target.diskType) === normal(offer.diskType);
    const sameVat = target.vatIncluded !== null && offer.vatIncluded !== null && target.vatIncluded === offer.vatIncluded;
    const trafficKnown = target.trafficUnlimited === true || target.trafficGib !== null;
    const peerTrafficKnown = offer.trafficUnlimited === true || offer.trafficGib !== null;
    const sameTraffic = trafficKnown && peerTrafficKnown &&
      (target.trafficUnlimited === true && offer.trafficUnlimited === true ||
        target.trafficUnlimited !== true && offer.trafficUnlimited !== true && target.trafficGib === offer.trafficGib);
    const standardPrice = standardPrices.has(target.priceKind) && standardPrices.has(offer.priceKind);

    const tier: ComparisonTier = exactSpecs && sameRegion && sameDisk && sameVat && sameTraffic && standardPrice
      ? 'exact'
      : nearSpecs && regionCompatible && (!diskKnown || sameDisk) && standardPrice ? 'close' : 'reference';
    const reasons: string[] = [];
    if (!knownSpecs) reasons.push('характеристики неполные');
    else if (!exactSpecs) reasons.push(nearSpecs ? 'ресурсы отличаются в пределах 2×' : 'ресурсы существенно отличаются');
    if (!sameRegion) reasons.push(target.region && offer.region ? 'другой регион' : 'регион не указан');
    if (!diskKnown) reasons.push('тип диска неизвестен');
    else if (!sameDisk) reasons.push('другой тип диска');
    if (!sameVat) reasons.push(target.vatIncluded === null || offer.vatIncluded === null ? 'НДС не уточнён' : 'условия НДС отличаются');
    if (!sameTraffic) reasons.push(!trafficKnown || !peerTrafficKnown ? 'трафик не уточнён' : 'лимит трафика отличается');
    if (!standardPrice) reasons.push('особые условия цены');

    const resourceDistance = knownSpecs ? ratios.reduce((sum, ratio) => sum + Math.abs(Math.log2(ratio)), 0) : 20;
    const distance = resourceDistance + (sameRegion ? 0 : target.region && offer.region ? 4 : 2) +
      (sameDisk ? 0 : diskKnown ? 2 : 0.5) + (sameVat ? 0 : 0.25) + (sameTraffic ? 0 : 0.25);

    return [{ offer, tier, reasons, differencePercent: (offer.priceRub / targetPrice - 1) * 100, distance }];
  }).sort((a, b) => {
    const order = { exact: 0, close: 1, reference: 2 };
    return order[a.tier] - order[b.tier] || a.distance - b.distance || a.offer.priceRub! - b.offer.priceRub!;
  });
}
