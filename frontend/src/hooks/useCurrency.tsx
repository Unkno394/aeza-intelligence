import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { MarketApi, type Fx } from '../features/market/api';
import { formatCurrency, type CurrencyCode, type RateTable } from '../features/market/currency';

const CURRENCY_KEY = 'aeza-display-currency';
const FX_CACHE_KEY = 'aeza-last-verified-fx';
const REFRESH_MS = 15 * 60 * 1000;

type FxCache = { fx: Fx; checkedAt: string; warning: string | null };
type CurrencyState = {
  currency: CurrencyCode;
  setCurrency: (currency: CurrencyCode) => void;
  fx: Fx | null;
  checkedAt: string | null;
  warning: string | null;
  loading: boolean;
  refresh: () => Promise<void>;
  convertRub: (amount: number, rates?: RateTable | null) => string;
};

const Context = createContext<CurrencyState | null>(null);

function readCurrency(): CurrencyCode {
  try {
    const value = localStorage.getItem(CURRENCY_KEY);
    return value === 'USD' || value === 'EUR' ? value : 'RUB';
  } catch { return 'RUB'; }
}

function readFxCache(): FxCache | null {
  try {
    const parsed = JSON.parse(localStorage.getItem(FX_CACHE_KEY) ?? 'null') as FxCache | null;
    if (!parsed?.fx?.date || !parsed.fx.rub_per_unit?.RUB || !parsed.fx.rub_per_unit?.USD || !parsed.fx.rub_per_unit?.EUR) return null;
    return parsed;
  } catch { return null; }
}

export function CurrencyProvider({ children }: { children: ReactNode }) {
  const [currency, setCurrencyState] = useState<CurrencyCode>(readCurrency);
  const [cache, setCache] = useState<FxCache | null>(readFxCache);
  const cacheRef = useRef<FxCache | null>(cache);
  const [loading, setLoading] = useState(!cache);
  const [warning, setWarning] = useState<string | null>(() => readFxCache()?.warning ?? null);

  const setCurrency = useCallback((next: CurrencyCode) => {
    setCurrencyState(next);
    try { localStorage.setItem(CURRENCY_KEY, next); } catch { /* Browser storage may be disabled. */ }
  }, []);

  const refresh = useCallback(async () => {
    setLoading(current => current && !cacheRef.current);
    try {
      const sources = await new MarketApi().sources();
      const statusWarning = sources.snapshot_status.startsWith('last_verified_snapshot:')
        ? `Источник обновился с ошибкой. Показан последний проверенный снимок (${sources.snapshot_status.slice('last_verified_snapshot:'.length).trim()}).`
        : null;
      const next: FxCache = { fx: sources.fx, checkedAt: new Date().toISOString(), warning: statusWarning };
      cacheRef.current = next;
      setCache(next);
      setWarning(statusWarning);
      try { localStorage.setItem(FX_CACHE_KEY, JSON.stringify(next)); } catch { /* Browser storage may be disabled. */ }
    } catch (error) {
      const message = error instanceof Error ? error.message : 'Не удалось получить снимок курсов.';
      setWarning(cacheRef.current
        ? `${message} Показан последний сохранённый курс от ${cacheRef.current.fx.date}.`
        : `${message} Сохранённого курса в этом браузере нет.`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
    const timer = window.setInterval(() => void refresh(), REFRESH_MS);
    return () => window.clearInterval(timer);
  }, [refresh]);

  const convertRub = useCallback((amount: number, rates?: RateTable | null) => formatCurrency(amount, currency, rates === undefined ? cache?.fx : rates), [cache?.fx, currency]);
  const value = useMemo<CurrencyState>(() => ({
    currency, setCurrency, fx: cache?.fx ?? null, checkedAt: cache?.checkedAt ?? null,
    warning, loading, refresh, convertRub,
  }), [currency, setCurrency, cache, warning, loading, refresh, convertRub]);

  return <Context.Provider value={value}>{children}</Context.Provider>;
}

export function useCurrency() {
  const value = useContext(Context);
  if (!value) throw new Error('useCurrency must be used inside CurrencyProvider');
  return value;
}
