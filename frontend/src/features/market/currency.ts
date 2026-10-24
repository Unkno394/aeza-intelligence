import type { Fx } from './api';

export type CurrencyCode = 'RUB' | 'USD' | 'EUR';
export type RateTable = Pick<Fx, 'rub_per_unit'>;

export const currencyOptions: { value: CurrencyCode; label: string }[] = [
  { value: 'RUB', label: 'RUB · Российский рубль' },
  { value: 'USD', label: 'USD · Доллар США' },
  { value: 'EUR', label: 'EUR · Евро' },
];

export function convertCurrency(amount: number, from: CurrencyCode, to: CurrencyCode, rates: RateTable | null | undefined): number | null {
  if (!Number.isFinite(amount)) return null;
  if (from === to) return amount;
  if (!rates) return null;
  const fromRate = Number(rates.rub_per_unit[from]);
  const toRate = Number(rates.rub_per_unit[to]);
  if (!(fromRate > 0) || !(toRate > 0)) return null;
  return amount * fromRate / toRate;
}

export function formatCurrency(amount: number | null | undefined, currency: CurrencyCode, rates?: RateTable | null): string {
  if (amount === null || amount === undefined || !Number.isFinite(amount)) return '—';
  const converted = convertCurrency(amount, 'RUB', currency, rates);
  if (converted === null) return 'Курс недоступен';
  return new Intl.NumberFormat('ru-RU', {
    style: 'currency', currency, maximumFractionDigits: 2,
  }).format(converted);
}

export function formatNativeCurrency(amount: number | null | undefined, from: CurrencyCode, to: CurrencyCode, rates?: RateTable | null): string {
  if (amount === null || amount === undefined || !Number.isFinite(amount)) return '—';
  const converted = convertCurrency(amount, from, to, rates);
  if (converted === null) return 'Курс недоступен';
  return new Intl.NumberFormat('ru-RU', {
    style: 'currency', currency: to, maximumFractionDigits: 2,
  }).format(converted);
}

export function ratesForDate(rubPerUsd: number | null | undefined, rubPerEur: number | null | undefined): RateTable | null {
  const table: Record<string, string> = { RUB: '1' };
  if (Number(rubPerUsd) > 0) table.USD = String(rubPerUsd);
  if (Number(rubPerEur) > 0) table.EUR = String(rubPerEur);
  return Object.keys(table).length > 1 ? { rub_per_unit: table } : null;
}
