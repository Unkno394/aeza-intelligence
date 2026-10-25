export type HistoricalDatePoint = { observedOn: string };
export type HistoricalPeriod = 7 | 30 | 90;

/** Keep the selected window anchored to the latest recorded observation. */
export function historyInPeriod<T extends HistoricalDatePoint>(
  points: readonly T[],
  periodDays: HistoricalPeriod,
): T[] {
  const latest = points.at(-1);
  if (!latest) return [];
  const cutoff = Date.parse(latest.observedOn) - periodDays * 86_400_000;
  return points.filter(point => Date.parse(point.observedOn) >= cutoff);
}

/** Use a requested date only while it remains visible in the selected period. */
export function selectedHistoryPoint<T extends HistoricalDatePoint>(
  points: readonly T[],
  visibleDates: readonly string[],
  selectedDate: string,
): T | undefined {
  if (!visibleDates.length) return undefined;
  const visible = new Set(visibleDates);
  return points.find(point => point.observedOn === selectedDate && visible.has(point.observedOn))
    ?? points.find(point => point.observedOn === visibleDates.at(-1));
}
