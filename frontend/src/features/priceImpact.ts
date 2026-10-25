export interface PriceImpactEvent {
  eventId: string;
  serviceId: string;
  accountId: string;
  accountEmail: string | null;
  planKey: string | null;
  planName: string;
  changedAt: string;
  previousPriceCents: number;
  currentPriceCents: number;
  nextRenewalAt: string;
  paymentTerm: string;
  completeBaseline: boolean;
  completePretrend: boolean;
  completeFollowup: boolean;
  renewalDueInWindow: boolean;
  currentStatus: string | null;
  currentAutoProlong: boolean | null;
  renewalsBefore: number;
  renewalsEarlier: number;
  renewalsAfter: number;
  prolongWaitAfter: boolean;
  autoRenewDisabledAfter: boolean;
  inactiveAfter: boolean;
  baselineServicesCount: number | null;
  followupServicesCount: number | null;
  baselineMonthlySpend: number | null;
  followupMonthlySpend: number | null;
  accountSpendBeforeCents: number;
  accountSpendEarlierCents: number;
  accountSpendAfterCents: number;
  controlAccountId: string | null;
  controlServicesBefore: number | null;
  controlServicesAfter: number | null;
  controlSpendBeforeCents: number | null;
  controlSpendAfterCents: number | null;
  controlRenewedAfter: boolean | null;
  controlRenewalsBefore: number | null;
  controlRenewalsEarlier: number | null;
  controlSpendEarlierCents: number | null;
}

export function demoPriceImpactEvents(now = Date.now()): PriceImpactEvent[] {
  const result: PriceImpactEvent[] = [];
  for (let period = 0; period < 2; period++) for (let band = 0; band < 3; band++) for (let index = 0; index < 100; index++) {
    const offset = period * 300 + band * 100 + index + 1;
    const day = now - (period === 0 ? 270 : 75) * 86_400_000;
    const renewed = index < 78 - band * 10;
    const controlRenewed = index < 80;
    const baselineSpend = index % 2 === 0 ? 400 : 600;
    result.push({
      eventId: `demo-${offset}`, serviceId: String(-9_100_000 - offset),
      accountId: String(-9_000_000 - offset), accountEmail: `price-demo-${offset}@example.test`,
      planKey: null, planName: 'PRICE-DEMO-1', changedAt: new Date(day).toISOString(),
      previousPriceCents: 500, currentPriceCents: [525, 575, 675][band],
      nextRenewalAt: new Date(day + 15 * 86_400_000).toISOString(), paymentTerm: 'month',
      completeBaseline: true, completePretrend: true, completeFollowup: true, renewalDueInWindow: true,
      currentStatus: renewed ? 'active' : 'prolong_wait', currentAutoProlong: renewed,
      renewalsEarlier: 1, renewalsBefore: 1, renewalsAfter: Number(renewed), prolongWaitAfter: !renewed,
      autoRenewDisabledAfter: !renewed, inactiveAfter: false,
      baselineServicesCount: 1, followupServicesCount: Number(renewed),
      baselineMonthlySpend: baselineSpend,
      followupMonthlySpend: renewed ? baselineSpend + band * 50 : 0,
      accountSpendEarlierCents: 500, accountSpendBeforeCents: 500, accountSpendAfterCents: renewed ? 500 + band * 50 : 0,
      controlAccountId: String(-9_000_600 - offset),
      controlServicesBefore: 1, controlServicesAfter: Number(controlRenewed),
      controlSpendBeforeCents: 500, controlSpendAfterCents: controlRenewed ? 500 : 0,
      controlRenewedAfter: controlRenewed, controlRenewalsEarlier: 1,
      controlRenewalsBefore: 1, controlSpendEarlierCents: 500,
    });
  }
  return result;
}

export type PriceOutcome = 'renewed' | 'adverse' | 'unknown' | 'pending' | 'not_due';

export function priceIncreasePercent(event: PriceImpactEvent): number {
  return (event.currentPriceCents / event.previousPriceCents - 1) * 100;
}

export function priceOutcome(event: PriceImpactEvent): PriceOutcome {
  if (!event.renewalDueInWindow) return 'not_due';
  if (!event.completeFollowup) return 'pending';
  if (event.renewalsAfter > 0) return 'renewed';
  if (event.inactiveAfter || event.autoRenewDisabledAfter || event.prolongWaitAfter) return 'adverse';
  return 'unknown';
}

export function summarizePriceImpact(events: PriceImpactEvent[]) {
  const eligible = events.filter(event => event.completeBaseline && event.completeFollowup && event.renewalDueInWindow);
  const renewed = eligible.filter(event => priceOutcome(event) === 'renewed').length;
  const adverse = eligible.filter(event => priceOutcome(event) === 'adverse').length;
  const unknown = eligible.length - renewed - adverse;
  const known = renewed + adverse;
  const coverage = eligible.length ? known / eligible.length : 0;
  const quality = known < 20 || coverage < 0.8 ? 'insufficient' : known < 100 ? 'limited' : 'sufficient';
  return { eligible: eligible.length, renewed, adverse, unknown, known, coverage, quality,
    observedAdverseShare: known ? adverse / known : null } as const;
}

export const priceBands = [
  { key: 'under10', label: 'До 10%', min: 0, max: 10 },
  { key: '10to25', label: '10–25%', min: 10, max: 25 },
  { key: 'over25', label: 'От 25%', min: 25, max: Infinity },
] as const;

export function summarizePriceBands(events: PriceImpactEvent[]) {
  // An account can have several increases. Keep only its latest eligible event
  // so one customer's payment and infrastructure change is not counted twice.
  const seenAccounts = new Set<string>();
  const eligible = events
    .filter(event => event.completeBaseline && event.completeFollowup && event.renewalDueInWindow)
    .sort((a, b) => Date.parse(b.changedAt) - Date.parse(a.changedAt))
    .filter(event => {
      if (seenAccounts.has(event.accountId)) return false;
      seenAccounts.add(event.accountId);
      return true;
    });
  return priceBands.map(band => {
    const cohort = eligible.filter(event => {
      const increase = priceIncreasePercent(event);
      return increase >= band.min && increase < band.max;
    });
    const summary = summarizePriceImpact(cohort);
    const infrastructure = cohort.filter(event => event.baselineServicesCount !== null && event.followupServicesCount !== null);
    const contracted = infrastructure.filter(event => event.followupServicesCount! < event.baselineServicesCount!).length;
    const payments = cohort.filter(event => event.accountSpendBeforeCents > 0);
    const reducedPayments = payments.filter(event => event.accountSpendAfterCents < event.accountSpendBeforeCents).length;
    const matched = cohort.filter(event => event.controlAccountId !== null);
    const matchedRenewed = matched.filter(event => event.renewalsAfter > 0).length;
    const controlRenewed = matched.filter(event => event.controlRenewedAfter).length;
    const matchedSpend = matched.filter(event => event.accountSpendBeforeCents > 0 && (event.controlSpendBeforeCents ?? 0) > 0);
    const spendChange = (rows: typeof matchedSpend, control: boolean) => rows.length
      ? rows.reduce((sum, event) => sum + (control
        ? (event.controlSpendAfterCents! - event.controlSpendBeforeCents!) / event.controlSpendBeforeCents!
        : (event.accountSpendAfterCents - event.accountSpendBeforeCents) / event.accountSpendBeforeCents), 0) / rows.length
      : null;
    return {
      ...band, ...summary,
      infrastructureObserved: infrastructure.length,
      contracted,
      paymentsObserved: payments.length,
      reducedPayments,
      contractionShare: infrastructure.length >= 20 ? contracted / infrastructure.length : null,
      reducedPaymentsShare: payments.length >= 20 ? reducedPayments / payments.length : null,
      accountSpendBeforeTotal: payments.reduce((sum, event) => sum + event.accountSpendBeforeCents, 0),
      accountSpendEarlierTotal: payments.reduce((sum, event) => sum + event.accountSpendEarlierCents, 0),
      accountSpendAfterTotal: payments.reduce((sum, event) => sum + event.accountSpendAfterCents, 0),
      renewalsBeforeTotal: cohort.reduce((sum, event) => sum + event.renewalsBefore, 0),
      renewalsEarlierTotal: cohort.reduce((sum, event) => sum + event.renewalsEarlier, 0),
      renewalsAfterTotal: cohort.reduce((sum, event) => sum + event.renewalsAfter, 0),
      matched: matched.length,
      distinctControls: new Set(matched.map(event => event.controlAccountId)).size,
      treatedRenewalShare: matched.length >= 20 ? matchedRenewed / matched.length : null,
      controlRenewalShare: matched.length >= 20 ? controlRenewed / matched.length : null,
      matchedSpend: matchedSpend.length,
      treatedSpendChange: matchedSpend.length >= 20 ? spendChange(matchedSpend, false) : null,
      controlSpendChange: matchedSpend.length >= 20 ? spendChange(matchedSpend, true) : null,
    };
  });
}

export function priceEvidence(events: PriceImpactEvent[]) {
  const matchedPretrend = events.filter(event => event.completePretrend
    && event.renewalsBefore >= event.renewalsEarlier
    && event.accountSpendBeforeCents * 5 >= event.accountSpendEarlierCents * 4
    && event.controlRenewalsBefore === event.renewalsBefore
    && event.controlRenewalsEarlier !== null
    && event.controlRenewalsBefore >= event.controlRenewalsEarlier
    && event.controlSpendEarlierCents !== null
    && (event.controlSpendBeforeCents ?? 0) * 5 >= event.controlSpendEarlierCents * 4);
  const periods = [0, 1].map(index => {
    const cutoff = Date.now() - 183 * 86_400_000;
    const subset = matchedPretrend.filter(event => index === 0 ? Date.parse(event.changedAt) < cutoff : Date.parse(event.changedAt) >= cutoff);
    return summarizePriceBands(subset);
  });
  const sufficient = periods.every(bands => bands.every(band =>
    band.known >= 100 && band.coverage >= 0.8 && band.matched >= 100
      && band.distinctControls >= 100 && band.matchedSpend >= 100));
  const spendValues = matchedPretrend.filter(event => event.completeBaseline && event.completeFollowup
    && event.renewalDueInWindow && event.baselineMonthlySpend !== null)
    .map(event => event.baselineMonthlySpend!).sort((a, b) => a - b);
  const medianSpend = spendValues.length
    ? (spendValues[Math.floor((spendValues.length - 1) / 2)] + spendValues[Math.floor(spendValues.length / 2)]) / 2
    : null;
  const segments = medianSpend === null ? [] : [
    summarizePriceBands(matchedPretrend.filter(event => event.baselineMonthlySpend !== null && event.baselineMonthlySpend <= medianSpend)),
    summarizePriceBands(matchedPretrend.filter(event => event.baselineMonthlySpend !== null && event.baselineMonthlySpend > medianSpend)),
  ];
  const direction = periods.map(bands => {
    const low = bands[0];
    const high = bands[2];
    if (low.observedAdverseShare === null || high.observedAdverseShare === null
      || high.treatedRenewalShare === null || high.controlRenewalShare === null) return null;
    return high.observedAdverseShare - low.observedAdverseShare >= 0.05
      && high.treatedRenewalShare < high.controlRenewalShare;
  });
  const segmentStable = segments.length === 2 && segments.every(bands => {
    const low = bands[0];
    const high = bands[2];
    return low.known >= 30 && high.known >= 30
      && low.distinctControls >= 30 && high.distinctControls >= 30
      && low.observedAdverseShare !== null && high.observedAdverseShare !== null
      && high.observedAdverseShare - low.observedAdverseShare >= 0.05
      && high.treatedRenewalShare !== null && high.controlRenewalShare !== null
      && high.treatedRenewalShare < high.controlRenewalShare;
  });
  return { periods, segments, medianSpend, sufficient, stable: sufficient && direction.every(Boolean) && segmentStable };
}

export function needsPriceReview(event: PriceImpactEvent, now = Date.now()): boolean {
  const changed = Date.parse(event.changedAt);
  const due = Date.parse(event.nextRenewalAt);
  return Number.isFinite(changed) && changed >= now - 90 * 86_400_000
    && Number.isFinite(due) && due <= now + 30 * 86_400_000
    && (due >= now - 7 * 86_400_000 || event.currentStatus === 'prolong_wait')
    && event.renewalsAfter === 0
    && (event.currentStatus === 'active' || event.currentStatus === 'prolong_wait');
}

export async function fetchPriceImpact(signal?: AbortSignal, mode: 'real' | 'synthetic' = 'real'): Promise<PriceImpactEvent[]> {
  const response = await fetch(mode === 'synthetic' ? '/api/client/price-impact-demo' : '/api/client/price-impact', { signal });
  if (!response.ok) throw new Error(`API: ${response.status}`);
  return response.json() as Promise<PriceImpactEvent[]>;
}
