import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { CartesianGrid, Legend, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';
import { ArrowDownRight, ArrowUpRight, Check, ChevronLeft, ChevronRight, RefreshCw, Search, Server, TriangleAlert, X } from 'lucide-react';
import { GlassSelect } from '../../components/ui/GlassSelect';
import { useCurrency } from '../../hooks/useCurrency';
import { compareCurrentOffers } from './marketComparison';
import { filterTariffs, paginateTariffs } from './tariffFilters';
import './tariff-monitor.css';

type Tariff = {
  providerCode: string; provider: string; planKey: string; planName: string;
  cpuCores: number | null; ramGib: number | null; diskGib: number | null;
  diskType: string | null; region: string | null; observedOn: string;
  priceRub: number | null; previousPriceRub: number | null; priceUsd: number | null; previousPriceUsd: number | null;
  priceEur: number | null; previousPriceEur: number | null; rubRateOn: string | null;
  usdRubRate: number | null; usdRateOn: string | null; eurRubRate: number | null; eurRateOn: string | null;
  changePercent: number | null; changePercentUsd: number | null; changePercentEur: number | null;
  historyPoints: number; marketMedianRub: number | null; marketMedianUsd: number | null; marketMedianEur: number | null; providersAtMedian: number;
  currency: string | null; originalPrice: number | null; billingPeriod: string | null;
  vatIncluded: boolean | null; trafficGib: number | null; trafficUnlimited: boolean | null;
  priceKind: string; pageDetails: string | null; sourceUrl: string; parseStatus: string;
};
type PricePoint = { providerCode: string; planKey: string; cpuCores: number | null; ramGib: number | null; diskGib: number | null; diskType: string | null; region: string | null; observedOn: string; priceRub: number; priceUsd: number | null; priceEur: number | null; usdRubRate: number | null; usdRateOn: string | null; eurRubRate: number | null; eurRateOn: string | null };
type ClientCohort = { planKey: string; linkedServices: number; accounts: number; awaitingRenewal: number; inactiveServices: number; performedRenewals: number; pendingRenewalTransactions: number };
type BehaviorRow = { planKey: string; changedAt: string; previousMinor: number; currentMinor: number; currency: string; cohort: 'tariff' | 'peer'; period: 'before' | 'after'; services: number; accounts: number; performedRenewals: number; renewalRevenueMinor: number; prolongWaitEntries: number; prolongWaitServices: number; accountTopupsCents: number; completeFollowup: boolean; completeEventCoverage: boolean | null };
type BenchmarkPoint = { monthStart: string; planKey: string; planName: string; cpuCores: number; ramGib: number; diskGib: number; diskType: string; region: string; priceRub: number; priceUsd: number | null; priceEur: number | null; marketMedianRub: number; marketMedianUsd: number | null; marketMedianEur: number | null; providerMedianMinRub: number; providerMedianMaxRub: number; providerMedianMinUsd: number | null; providerMedianMaxUsd: number | null; providerMedianMinEur: number | null; providerMedianMaxEur: number | null; providersAtMedian: number; gapPercent: number; gapPercentUsd: number | null; gapPercentEur: number | null };

const number = (value: number | null) => value === null ? '—' : new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 2 }).format(value);
const currencyMinor = (minor: number, currency: string) => {
  try { return new Intl.NumberFormat('ru-RU', { style: 'currency', currency, maximumFractionDigits: 2 }).format(minor / 100); }
  catch { return (minor / 100).toFixed(2) + ' ' + currency; }
};
const dateLabel = (value: string) => new Date(value + 'T00:00:00').toLocaleDateString('ru-RU', { day: 'numeric', month: 'short', year: 'numeric' });
const keyOf = (row: Tariff) => [row.providerCode, row.planKey, row.cpuCores, row.ramGib, row.diskGib, row.diskType, row.region].join('|');
const benchmarkKey = (row: BenchmarkPoint) => [row.planKey, row.cpuCores, row.ramGib, row.diskGib, row.diskType, row.region].join('|');
const benchmarkOfferKey = (row: Tariff) => [row.planKey, row.cpuCores, row.ramGib, row.diskGib, row.diskType, row.region].join('|');
const completePriceKinds = new Set(['regular', 'listed_monthly', 'official_catalog']);
const hasExactBenchmark = (row: Tariff) => row.priceRub !== null && row.marketMedianRub !== null && row.providersAtMedian >= 2;
const exactPeers = (target: Tariff, rows: Tariff[]) => {
  if (target.priceRub === null || target.cpuCores === null || target.ramGib === null || target.diskGib === null || !target.diskType || !target.region || target.vatIncluded === null || (target.trafficGib === null && target.trafficUnlimited !== true)) return [];
  return rows.filter(row => row.providerCode !== target.providerCode && row.priceRub !== null
    && completePriceKinds.has(row.priceKind) && row.parseStatus === 'complete'
    && row.cpuCores === target.cpuCores && row.ramGib === target.ramGib && row.diskGib === target.diskGib
    && row.diskType?.toLocaleLowerCase() === target.diskType?.toLocaleLowerCase()
    && row.region?.toLocaleLowerCase() === target.region?.toLocaleLowerCase()
    && row.vatIncluded === target.vatIncluded && row.trafficGib === target.trafficGib
    && row.trafficUnlimited === target.trafficUnlimited);
};
const behaviorReady = (row: BehaviorRow) => row.completeFollowup && row.completeEventCoverage === true && row.services > 0;

export default function TariffMonitor({focusPlanKey,focusProvider,focusCpu,focusRam,focusDisk,focusRegion}:{focusPlanKey?:string|null;focusProvider?:string|null;focusCpu?:string|null;focusRam?:string|null;focusDisk?:string|null;focusRegion?:string|null}) {
  const { currency } = useCurrency();
  const money = (rubValue: number | null, usdValue?: number | null, eurValue?: number | null) => {
    const value = currency === 'RUB' ? rubValue : currency === 'USD' ? usdValue : eurValue;
    if (currency !== 'RUB' && value === null) return 'Нет курса на дату';
    if (value === null || value === undefined) return '—';
    return new Intl.NumberFormat('ru-RU', { style: 'currency', currency, maximumFractionDigits: 2 }).format(value);
  };
  const formatDisplayed = (value:number|null|undefined) => value===null||value===undefined?'—':new Intl.NumberFormat('ru-RU',{style:'currency',currency,maximumFractionDigits:2}).format(value);
  const displayedPrice = (row: {priceRub:number|null;priceUsd?:number|null;priceEur?:number|null}) => currency==='RUB'?row.priceRub:currency==='USD'?row.priceUsd??null:row.priceEur??null;
  const changeForCurrency = (row: Tariff) => currency==='RUB'?row.changePercent:currency==='USD'?row.changePercentUsd:row.changePercentEur;
  const [rows, setRows] = useState<Tariff[]>([]);
  const [history, setHistory] = useState<PricePoint[]>([]);
  const [cohorts, setCohorts] = useState<ClientCohort[]>([]);
  const [behavior, setBehavior] = useState<BehaviorRow[]>([]);
  const [benchmarks, setBenchmarks] = useState<BenchmarkPoint[]>([]);
  const [benchmarkSelection, setBenchmarkSelection] = useState('');
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [benchmarkWarning, setBenchmarkWarning] = useState('');
  const [providerFilter, setProviderFilter] = useState('all');
  const [regionFilter, setRegionFilter] = useState('all');
  const [query, setQuery] = useState('');
  const [onlyAeza, setOnlyAeza] = useState(false);
  const [selectedKey, setSelectedKey] = useState('');
  const [page, setPage] = useState(0);
  const pageSize = 20;

  async function load() {
    setLoading(true); setError('');
    try {
      const urls = ['/api/market/prices/monitor', '/api/market/prices/history', '/api/market/prices/client-cohorts', '/api/market/prices/behavior'];
      const responses = await Promise.all(urls.map(url => fetch(url)));
      const failedIndex = responses.findIndex(response => !response.ok);
      if (failedIndex >= 0) throw new Error(`GET ${urls[failedIndex]}: HTTP ${responses[failedIndex].status}`);
      const [current, series, clientCohorts, priceBehavior] = await Promise.all(responses.map(response => response.json()));
      setRows(current); setHistory(series); setCohorts(clientCohorts); setBehavior(priceBehavior);
      const benchmarkUrl = '/api/market/prices/benchmarks/history';
      try {
        const benchmarkResponse = await fetch(benchmarkUrl);
        if (!benchmarkResponse.ok) throw new Error(`GET ${benchmarkUrl}: HTTP ${benchmarkResponse.status}`);
        const benchmarkHistory: BenchmarkPoint[] = await benchmarkResponse.json();
        setBenchmarks(benchmarkHistory);
        setBenchmarkWarning('');
        if (benchmarkHistory.length) setBenchmarkSelection(current => benchmarkHistory.some(point => benchmarkKey(point) === current) ? current : benchmarkKey(benchmarkHistory.at(-1)!));
        else setBenchmarkSelection('');
      } catch (reason) {
        setBenchmarks([]);
        setBenchmarkSelection('');
        setBenchmarkWarning(reason instanceof Error ? reason.message : `GET ${benchmarkUrl}: запрос не выполнен`);
      }
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Не удалось загрузить тарифы'); }
    finally { setLoading(false); }
  }
  useEffect(() => { void load(); }, []);
  useEffect(() => {
    if (!focusPlanKey || !rows.length) return;
    const match = rows.find(row => row.planKey === focusPlanKey && (!focusProvider || row.providerCode === focusProvider)
      && (!focusCpu || row.cpuCores === Number(focusCpu)) && (!focusRam || Math.abs((row.ramGib ?? 0) - Number(focusRam)) < .05)
      && (!focusDisk || Math.abs((row.diskGib ?? 0) - Number(focusDisk)) < .05)
      && (!focusRegion || row.region === focusRegion));
    if (match) { setSelectedKey(keyOf(match)); setProviderFilter(match.providerCode); setQuery(match.planName); setPage(0); requestAnimationFrame(() => requestAnimationFrame(() => document.querySelector('.tariff-detail')?.scrollIntoView({ block: 'start' }))); }
  }, [focusPlanKey,focusProvider,focusCpu,focusRam,focusDisk,focusRegion,rows]);

  const providers = useMemo(() => [...new Set(rows.map(row => row.providerCode))].sort(), [rows]);
  const regions = useMemo(() => [...new Set(rows.map(row => row.region).filter((value): value is string => Boolean(value)))].sort(), [rows]);
  const filteredRows = useMemo(() => filterTariffs(rows, {
    provider: providerFilter, region: regionFilter, onlyAeza, query,
  }), [rows, providerFilter, regionFilter, onlyAeza, query]);
  const { pageCount, safePage, visibleRows } = paginateTariffs(filteredRows, page, pageSize);
  const selected = rows.find(row => keyOf(row) === selectedKey) ?? null;
  const providerCount = providers.length;
  const aezaRows = rows.filter(row => row.providerCode === 'aeza' && row.priceRub !== null);
  const comparableCount = aezaRows.filter(row => compareCurrentOffers(row, rows).some(peer => peer.tier !== 'reference')).length;
  const competitorCount = rows.filter(row => row.providerCode !== 'aeza').length;
  const selectedHistory = selected ? history.filter(point => point.providerCode === selected.providerCode && point.planKey === selected.planKey
    && point.cpuCores === selected.cpuCores && point.ramGib === selected.ramGib && point.diskGib === selected.diskGib
    && point.diskType === selected.diskType && point.region === selected.region).sort((a, b) => a.observedOn.localeCompare(b.observedOn))
    .map(point=>({...point,displayPrice:currency==='RUB'?point.priceRub:currency==='USD'?point.priceUsd:point.priceEur})).filter(point=>point.displayPrice!==null) : [];
  const selectedPeers = selected ? exactPeers(selected, rows).sort((a, b) => (displayedPrice(a) ?? Infinity) - (displayedPrice(b) ?? Infinity)) : [];
  const selectedAlternatives = selected && !selectedPeers.length ? compareCurrentOffers(selected, rows).slice(0, 6) : [];
  const selectedCohort = selected?.providerCode === 'aeza' ? cohorts.find(item => item.planKey === selected.planKey) : undefined;
  const selectedBehavior = selected?.providerCode === 'aeza' ? behavior.filter(item => item.planKey === selected.planKey) : [];
  const benchmarkOptions = useMemo(() => [...new Map(benchmarks.map(point => [benchmarkKey(point), point])).entries()]
    .map(([value, point]) => ({ value, label: `${point.planName} · ${number(point.cpuCores)} vCPU / ${number(point.ramGib)} GB / ${number(point.diskGib)} GB · ${point.region}` })), [benchmarks]);
  const benchmarkSeries = useMemo(() => benchmarks.filter(point => benchmarkKey(point) === benchmarkSelection)
    .sort((a, b) => a.monthStart.localeCompare(b.monthStart)).map(point => ({ ...point, month: point.monthStart.slice(0, 7),
      priceDisplay:currency==='RUB'?point.priceRub:currency==='USD'?point.priceUsd:point.priceEur,
      medianDisplay:currency==='RUB'?point.marketMedianRub:currency==='USD'?point.marketMedianUsd:point.marketMedianEur,
      minDisplay:currency==='RUB'?point.providerMedianMinRub:currency==='USD'?point.providerMedianMinUsd:point.providerMedianMinEur,
      maxDisplay:currency==='RUB'?point.providerMedianMaxRub:currency==='USD'?point.providerMedianMaxUsd:point.providerMedianMaxEur,
      gapDisplay:currency==='RUB'?point.gapPercent:currency==='USD'?point.gapPercentUsd:point.gapPercentEur,
    })), [benchmarks, benchmarkSelection,currency]);
  const currentBenchmarkOptions = aezaRows.map(row => ({ value: benchmarkOfferKey(row), label: `${row.planName} · ${number(row.cpuCores)} CPU / ${number(row.ramGib)} GB RAM / ${number(row.diskGib)} GB · ${row.region ?? 'регион неизвестен'}` }));
  const currentBenchmarkTarget = aezaRows.find(row => benchmarkOfferKey(row) === benchmarkSelection) ?? aezaRows[0];
  const currentBenchmarkPeers = currentBenchmarkTarget ? compareCurrentOffers(currentBenchmarkTarget, rows).filter(peer => peer.tier !== 'reference') : [];
  const currentProviderPeers = currentBenchmarkPeers.filter((peer, index, all) => all.findIndex(item => item.offer.providerCode === peer.offer.providerCode) === index).slice(0, 6);
  const cheapestPeer = currentBenchmarkPeers.reduce<(typeof currentBenchmarkPeers)[number] | null>((best, peer) => !best || peer.offer.priceRub! < best.offer.priceRub! ? peer : best, null);
  const currentBarOffers = currentBenchmarkTarget ? [currentBenchmarkTarget, ...currentProviderPeers.map(peer => peer.offer)].filter(row => displayedPrice(row) !== null) : [];
  const currentBarMax = Math.max(1, ...currentBarOffers.map(row => displayedPrice(row)!));

  const chooseRow = (row: Tariff) => {
    setSelectedKey(current => current === keyOf(row) ? '' : keyOf(row));
  };
  const resetFilters = () => { setProviderFilter('all'); setRegionFilter('all'); setQuery(''); setOnlyAeza(false); setPage(0); };

  if (loading && rows.length === 0) return <section className="tariff-monitor-view"><div className="tariff-loading"><span className="spin"><RefreshCw size={18}/></span> Загружаем данные по тарифам…</div></section>;
  if (error && rows.length === 0) return <section className="tariff-monitor-view"><div className="tariff-error" role="alert"><TriangleAlert size={18}/><div><strong>Не удалось загрузить цены</strong><span>{error}</span><button type="button" onClick={() => void load()}>Повторить</button></div></div></section>;
  if (!rows.length) return <section className="tariff-monitor-view"><div className="tariff-empty"><Server size={24}/><strong>Снимков подходящих тарифов пока нет</strong><span>Когда парсер сохранит тарифные наблюдения, здесь появятся сравнения, история и клиентские когорты.</span></div></section>;

  return <section className="tariff-monitor-view">
    {error && <div className="tariff-inline-error" role="alert"><TriangleAlert size={15}/> Не удалось обновить данные: {error}. Показаны последние загруженные значения.</div>}

    <div className="tariff-kpis">
      <article><small>Провайдеры</small><strong>{providerCount}</strong><span>с актуальными наблюдениями</span></article>
      <article><small>Тарифы Aéza с близкими предложениями</small><strong>{comparableCount}</strong><span>по CPU, RAM и диску</span></article>
      <article><small>Снимки цен</small><strong>{history.length}</strong><span>с датой наблюдения</span></article>
      <article><small>Тарифы конкурентов</small><strong>{competitorCount}</strong><span>с месячной ценой</span></article>
    </div>

    <section className="tariff-benchmark-history">
      <div className="tariff-market-history-heading"><div><h3>{benchmarkSeries.length ? 'История медианы рынка для Aéza' : 'Срез цен Aéza и конкурентов'}</h3><p>{benchmarkSeries.length ? 'Месячные наблюдения · медиана между независимыми провайдерами' : 'Текущие сохранённые цены близких конфигураций; условия и даты наблюдения указаны отдельно'}</p></div><label>Тариф / конфигурация<GlassSelect value={benchmarkSeries.length ? benchmarkSelection : currentBenchmarkTarget ? benchmarkOfferKey(currentBenchmarkTarget) : ''} onChange={setBenchmarkSelection} searchable searchPlaceholder="Поиск тарифа Aéza" options={benchmarkSeries.length ? benchmarkOptions : currentBenchmarkOptions}/></label></div>
      {benchmarkSeries.length ? <><div className="tariff-benchmark-kpis"><div><small>Цена Aéza</small><strong>{formatDisplayed(benchmarkSeries.at(-1)!.priceDisplay)}</strong></div><div><small>Медиана рынка</small><strong>{formatDisplayed(benchmarkSeries.at(-1)!.medianDisplay)}</strong></div><div><small>Отклонение</small><strong>{benchmarkSeries.at(-1)!.gapDisplay == null ? 'Нет курса на дату' : `${benchmarkSeries.at(-1)!.gapDisplay! >= 0 ? '+' : ''}${benchmarkSeries.at(-1)!.gapDisplay!.toFixed(1)}%`}</strong></div><div><small>Провайдеров</small><strong>{benchmarkSeries.at(-1)!.providersAtMedian}</strong></div></div><div className="tariff-benchmark-chart"><ResponsiveContainer><LineChart data={benchmarkSeries}><CartesianGrid stroke="#a9dfff16" vertical={false}/><XAxis dataKey="month" minTickGap={22}/><YAxis tickFormatter={value => number(Number(value))}/><Tooltip formatter={(value, name) => [formatDisplayed(Number(value)), name === 'priceDisplay' ? 'Aéza' : 'Медиана рынка']}/><Legend formatter={name => name === 'priceDisplay' ? 'Aéza' : 'Медиана рынка'}/><Line dataKey="priceDisplay" name="priceDisplay" stroke="#78d6ff" strokeWidth={2.5} dot={false}/><Line dataKey="medianDisplay" name="medianDisplay" stroke="#f9bc7c" strokeWidth={2.5} dot={false}/></LineChart></ResponsiveContainer></div><p>Разброс медиан провайдеров за последний месяц: {formatDisplayed(benchmarkSeries.at(-1)!.minDisplay)}–{formatDisplayed(benchmarkSeries.at(-1)!.maxDisplay)}. Каждая точка показывает число независимых поставщиков.</p></>
        : currentBenchmarkTarget ? <>
          {benchmarkWarning && <div className="tariff-benchmark-warning"><TriangleAlert size={15}/><span>Исторический API недоступен ({benchmarkWarning}). Срез цен ниже загружен отдельно.</span></div>}
          <div className="tariff-benchmark-kpis"><div><small>Цена Aéza</small><strong>{money(currentBenchmarkTarget.priceRub,currentBenchmarkTarget.priceUsd,currentBenchmarkTarget.priceEur)}</strong></div><div><small>Близких предложений</small><strong>{currentBenchmarkPeers.length}</strong></div><div><small>Провайдеров</small><strong>{new Set(currentBenchmarkPeers.map(peer => peer.offer.providerCode)).size}</strong></div><div><small>Самая низкая цена</small><strong>{cheapestPeer ? money(cheapestPeer.offer.priceRub,cheapestPeer.offer.priceUsd,cheapestPeer.offer.priceEur) : '—'}</strong></div></div>
          {currentBarOffers.length > 1 ? <div className="tariff-current-bars">{currentBarOffers.map(row => <div key={keyOf(row)}><div><strong>{row.provider}</strong><small>{row.planName} · {dateLabel(row.observedOn)} · {row.region ?? 'регион неизвестен'}</small><b>{money(row.priceRub,row.priceUsd,row.priceEur)}</b></div><span><i style={{ width: `${Math.max(3, displayedPrice(row)! / currentBarMax * 100)}%` }}/></span></div>)}</div> : <div className="tariff-detail-empty">Для выбранного тарифа пока нет близких конфигураций другого провайдера с ценой в {currency}.</div>}
          <p>Это сравнение сохранённых цен по CPU, RAM и диску. Неизвестный регион, тип диска, НДС или трафик не считаются подтверждённым совпадением. <Link to="/market?tab=monitor">Открыть различия и источники</Link>.</p>
        </> : <div className="tariff-detail-empty">В сохранённом каталоге пока нет тарифа Aéza с месячной ценой.</div>}
    </section>

    <div className="tariff-controls">
      <label>Провайдер<GlassSelect value={providerFilter} onChange={value => { setProviderFilter(value); setPage(0); }} options={[{ value: 'all', label: 'Все провайдеры' }, ...providers.map(code => ({ value: code, label: rows.find(row => row.providerCode === code)?.provider ?? code }))]}/></label>
      <label className="tariff-search"><Search size={16}/><input value={query} onChange={event => { setQuery(event.target.value); setPage(0); }} placeholder="Поиск по тарифу, конфигурации или региону" aria-label="Поиск тарифов"/></label>
      <label>Регион<GlassSelect value={regionFilter} onChange={value => { setRegionFilter(value); setPage(0); }} options={[{ value: 'all', label: 'Все регионы' }, ...regions.map(region => ({ value: region, label: region })), { value: '__unknown__', label: 'Регион неизвестен' }]}/></label>
      <button type="button" role="switch" aria-checked={onlyAeza} className={'tariff-only-aeza' + (onlyAeza ? ' active' : '')} onClick={() => { setOnlyAeza(value => !value); setPage(0); }}><i>{onlyAeza && <Check size={12}/>}</i>Только Aéza</button>
    </div>

    <section className="tariff-table-panel">
      <div className="tariff-table-heading"><div><strong>Тарифы и рыночное сравнение</strong><span>Цены показаны в {currency}; для старых наблюдений используются курсы на дату фиксации.</span></div><small>{filteredRows.length} тарифов</small></div>
      <div className="table-scroll"><table className="tariff-table"><thead><tr><th>Тариф / провайдер</th><th>Конфигурация</th><th>Регион и условия</th><th>Цена / мес.</th><th>Рыночное сравнение</th><th>Изменение цены</th><th>Клиенты Aéza</th><th>Детали</th></tr></thead><tbody>
        {visibleRows.map(row => {
          const cohort = row.providerCode === 'aeza' ? cohorts.find(item => item.planKey === row.planKey) : undefined;
          const active = selectedKey === keyOf(row);
          const nearest = !hasExactBenchmark(row) ? compareCurrentOffers(row, rows).find(item => item.tier !== 'reference') : undefined;
          return <tr key={keyOf(row)} className={active ? 'selected' : ''} onClick={() => chooseRow(row)} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); chooseRow(row); } }} tabIndex={0} aria-selected={active}>
            <td><strong>{row.planName}</strong><small>{row.provider}</small></td>
            <td>{number(row.cpuCores)} vCPU · {number(row.ramGib)} GB RAM<br/>{number(row.diskGib)} GB {row.diskType ?? 'диск неизвестен'}</td>
            <td>{row.region ?? 'Регион неизвестен'}<small>{row.vatIncluded === null ? 'НДС неизвестен' : row.vatIncluded ? 'НДС включён' : 'Без НДС'} · {row.trafficUnlimited ? 'трафик безлимитный' : row.trafficGib === null ? 'трафик неизвестен' : number(row.trafficGib) + ' GB трафика'}</small></td>
            <td><strong className="tariff-price-value">{money(row.priceRub,row.priceUsd,row.priceEur)}</strong><small>{row.originalPrice === null ? 'Цена недоступна' : number(row.originalPrice) + ' ' + (row.currency ?? '') + ' / ' + (row.billingPeriod ?? 'период не указан')} · курс {currency} на {currency==='USD'?row.usdRateOn??'нет данных':currency==='EUR'?row.eurRateOn??'нет данных':row.rubRateOn??row.observedOn}</small></td>
            <td>{hasExactBenchmark(row) ? <><strong>{money(row.marketMedianRub,row.marketMedianUsd,row.marketMedianEur)}</strong><small>{row.providersAtMedian} провайдера · точное совпадение</small></> : nearest ? <><strong>{money(nearest.offer.priceRub,nearest.offer.priceUsd,nearest.offer.priceEur)}</strong><small>{nearest.offer.provider} · {nearest.tier === 'exact' ? 'точное совпадение' : 'близкий тариф'} · {displayedPrice(nearest.offer) !== null && displayedPrice(row) !== null ? `${((displayedPrice(nearest.offer)! / displayedPrice(row)!) - 1) >= 0 ? '+' : ''}${(((displayedPrice(nearest.offer)! / displayedPrice(row)!) - 1) * 100).toFixed(1)}%` : 'нет курса на дату'}</small></> : <span className="tariff-no-data">Близких предложений нет</span>}</td>
            <td>{changeForCurrency(row) === null ? <span className="tariff-no-data">Недостаточно данных</span> : <><strong className={changeForCurrency(row)! <= 0 ? 'market-positive' : 'market-negative'}>{changeForCurrency(row)! <= 0 ? <ArrowDownRight size={13}/> : <ArrowUpRight size={13}/>} {changeForCurrency(row)! >= 0 ? '+' : ''}{changeForCurrency(row)!.toFixed(1)}%</strong><small>{money(row.previousPriceRub,row.previousPriceUsd,row.previousPriceEur)} → {money(row.priceRub,row.priceUsd,row.priceEur)}</small></>}</td>
            <td>{row.providerCode === 'aeza' ? cohort ? <><strong>{cohort.accounts} клиентов · {cohort.linkedServices} услуг</strong><small>Ожидают продления: {cohort.awaitingRenewal} · неактивны: {cohort.inactiveServices}</small></> : <span className="tariff-no-data">Нет связанных услуг</span> : <span className="tariff-not-applicable">Только для тарифов Aéza</span>}</td>
            <td><button type="button" className="tariff-open-row" onClick={event => { event.stopPropagation(); chooseRow(row); }}>{active ? 'Закрыть' : 'Открыть'}{active ? <X size={14}/> : <ChevronRight size={14}/>}</button></td>
          </tr>;
        })}
        {visibleRows.length === 0 && <tr><td colSpan={8} className="tariff-no-results">По этим фильтрам ничего не найдено. <button type="button" onClick={resetFilters}>Сбросить фильтры</button></td></tr>}
      </tbody></table></div>
      <div className="tariff-pagination"><span>Показано {filteredRows.length ? safePage * pageSize + 1 : 0}–{Math.min((safePage + 1) * pageSize, filteredRows.length)} из {filteredRows.length}</span><div><button type="button" aria-label="Предыдущая страница" disabled={safePage === 0} onClick={() => setPage(safePage - 1)}><ChevronLeft size={16}/></button><span>{safePage + 1} / {pageCount}</span><button type="button" aria-label="Следующая страница" disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}><ChevronRight size={16}/></button></div></div>
    </section>

    {selected && <section className="tariff-detail" aria-label={'Подробности тарифа ' + selected.planName}>
      <div className="tariff-detail-heading"><div><span className="tariff-detail-provider">{selected.provider}</span><h3>{selected.planName}</h3><p>{number(selected.cpuCores)} vCPU · {number(selected.ramGib)} GB RAM · {number(selected.diskGib)} GB {selected.diskType ?? 'диск неизвестен'} · {selected.region ?? 'регион неизвестен'}</p></div><button type="button" aria-label="Закрыть детали тарифа" onClick={() => setSelectedKey('')}><X size={17}/></button></div>
      {selected.providerCode === 'aeza' && <div className="tariff-detail-actions"><Link to={`/price-impact?plan=${encodeURIComponent(selected.planKey)}`}>Поведение клиентов по тарифу <ArrowUpRight size={15}/></Link><Link to={`/market-simulator?from=market&plan=${encodeURIComponent(selected.planKey)}&name=${encodeURIComponent(selected.planName)}&cpu=${selected.cpuCores ?? ''}&ram=${selected.ramGib ?? ''}&disk=${selected.diskGib ?? ''}&region=${encodeURIComponent(selected.region ?? '')}&price=${selected.priceRub ?? ''}`}>Проверить сценарий цены <ArrowUpRight size={15}/></Link></div>}
      <div className="tariff-detail-grid">
        <section className="tariff-detail-card tariff-history-card"><div className="tariff-detail-card-heading"><div><strong>История цены</strong><span>{selectedHistory.length} наблюдений · {currency} за месяц · курс на дату наблюдения</span></div><a href={selected.sourceUrl} target="_blank" rel="noreferrer">Источник тарифа <ArrowUpRight size={13}/></a></div>
          {selectedHistory.length > 1 ? <div className="tariff-detail-chart"><ResponsiveContainer><LineChart data={selectedHistory} margin={{ top: 10, right: 14, bottom: 3, left: 2 }}><CartesianGrid stroke="#a9dfff16" vertical={false}/><XAxis dataKey="observedOn" tickFormatter={dateLabel} minTickGap={22} tick={{ fill: '#89a7bb', fontSize: 14 }} axisLine={false} tickLine={false}/><YAxis domain={['auto', 'auto']} tickFormatter={value => number(Number(value))} tick={{ fill: '#89a7bb', fontSize: 14 }} axisLine={false} tickLine={false} width={48}/><Tooltip labelFormatter={value => dateLabel(String(value))} formatter={value => [formatDisplayed(Number(value)), 'Цена / мес.']} contentStyle={{ background: '#10283d', border: '1px solid #6ab9e533', borderRadius: 9, color: '#e4f4ff' }}/><Line type="monotone" dataKey="displayPrice" stroke="#78d6ff" strokeWidth={2.5} dot={{ r: 3, fill: '#10283d', stroke: '#78d6ff', strokeWidth: 2 }}/></LineChart></ResponsiveContainer></div> : <div className="tariff-detail-empty">{selectedHistory.length === 1 ? 'Есть только один снимок с доступным курсом: для графика нужна ещё одна точка.' : 'История или курс на дату наблюдения недоступны.'}</div>}
        </section>
        <section className="tariff-detail-card"><div className="tariff-detail-card-heading"><div><strong>Сравнение с конкурентами</strong><span>Характеристики, регион и условия; при отсутствии полного совпадения — ближайшие предложения</span></div></div>
          {hasExactBenchmark(selected) && <div className="tariff-market-median"><small>Медиана рынка</small><strong>{money(selected.marketMedianRub,selected.marketMedianUsd,selected.marketMedianEur)}</strong><span>{selected.providersAtMedian} конкурента · {displayedPrice(selected)!==null && (currency==='RUB'?selected.marketMedianRub:currency==='USD'?selected.marketMedianUsd:selected.marketMedianEur) ? `${((displayedPrice(selected)! / (currency==='RUB'?selected.marketMedianRub!:currency==='USD'?selected.marketMedianUsd!:selected.marketMedianEur!) - 1) * 100 >= 0 ? '+' : '')}${((displayedPrice(selected)! / (currency==='RUB'?selected.marketMedianRub!:currency==='USD'?selected.marketMedianUsd!:selected.marketMedianEur!) - 1) * 100).toFixed(1)}% к медиане` : 'Нет курса на дату'}</span></div>}
          {selectedPeers.length > 0 ? <div className="tariff-peer-list">{selectedPeers.map(peer => {const peerPrice=displayedPrice(peer),ownPrice=displayedPrice(selected);return <div key={keyOf(peer)}><span><strong>{peer.provider}</strong><small>{peer.planName} · {dateLabel(peer.observedOn)}</small></span><b>{money(peer.priceRub,peer.priceUsd,peer.priceEur)}</b>{ownPrice!==null&&peerPrice!==null&&<em className={peerPrice<=ownPrice?'is-cheaper':''}>{(peerPrice/ownPrice-1>=0?'+':'')+((peerPrice/ownPrice-1)*100).toFixed(1)}%</em>}</div>;})}</div> : selectedAlternatives.length ? <div className="tariff-peer-list">{selectedAlternatives.map(({ offer, tier, reasons }) => {const peerPrice=displayedPrice(offer),ownPrice=displayedPrice(selected);const gap=peerPrice!==null&&ownPrice!==null?peerPrice/ownPrice-1:null;return <div key={keyOf(offer)}><span><strong>{offer.provider} · {offer.planName}</strong><small>{tier === 'reference' ? 'Ценовой ориентир' : 'Близкое предложение'} · {dateLabel(offer.observedOn)} · {reasons.join(', ') || 'условия совпадают'}</small></span><b>{money(offer.priceRub,offer.priceUsd,offer.priceEur)}</b>{gap!==null&&<em className={gap<=0?'is-cheaper':''}>{gap>=0?'+':''}{(gap*100).toFixed(1)}%</em>}</div>;})}</div> : <p className="tariff-detail-empty">Нет сохранённых цен другого провайдера для сравнения.</p>}
          {selected.marketMedianRub === null && <p className="tariff-detail-note">Медиана скрыта: API не нашёл минимум два предложения с полным совпадением условий.</p>}
        </section>
        <section className="tariff-detail-card tariff-behavior-card"><div className="tariff-detail-card-heading"><div><strong>Поведение клиентов до и после изменения цены</strong><span>Фактические продления и события из PostgreSQL · совпадение по времени не доказывает причинность</span></div></div>
          {selected.providerCode !== 'aeza' ? <p className="tariff-detail-empty">Клиентские метрики доступны для тарифов Aéza, связанных с услугами в PostgreSQL.</p> : selectedBehavior.length === 0 ? <p className="tariff-detail-empty">Недостаточно данных: история цены пока не содержит пригодного изменения этого тарифа.</p> : <div className="tariff-behavior-table-wrap"><table className="tariff-behavior-table"><thead><tr><th>Группа / период</th><th>Услуги · клиенты</th><th>Продления</th><th>prolong_wait</th><th>Выручка с продлений</th><th>Данные</th></tr></thead><tbody>{selectedBehavior.map((item, index) => {
            const ready = behaviorReady(item);
            return <tr key={item.cohort + item.period + index}><td>{item.cohort === 'tariff' ? 'Этот тариф' : 'Похожие услуги'}<small>{item.period === 'before' ? '30 дней до' : '30 дней после'} · {dateLabel(item.changedAt.slice(0, 10))}</small></td><td>{ready ? item.services + ' · ' + item.accounts : 'Недостаточно данных'}</td><td>{ready ? item.performedRenewals : 'Недостаточно данных'}</td><td>{ready ? item.prolongWaitEntries + ' событий · ' + item.prolongWaitServices + ' услуг' : 'Недостаточно данных'}</td><td>{ready ? currencyMinor(item.renewalRevenueMinor, item.currency) : 'Недостаточно данных'}</td><td>{ready ? <span className="tariff-data-complete">Полный период · события учтены</span> : <span className="tariff-data-incomplete">Недостаточно данных</span>}</td></tr>;
          })}</tbody></table></div>}
          {selectedCohort && <p className="tariff-detail-note">Сейчас по этому тарифу: {selectedCohort.accounts} аккаунтов, {selectedCohort.linkedServices} услуг, {selectedCohort.awaitingRenewal} ожидают продления. Пополнения всего аккаунта в метрики тарифа не включены.</p>}
        </section>
      </div>
      {selected.pageDetails && <details className="tariff-source-details"><summary>Условия и сведения со страницы тарифа</summary><p>{selected.pageDetails}</p></details>}
      <p className="tariff-detail-footnote">Медиана и изменения показаны по фактическим данным API. Клиентские метрики описывают группы вокруг изменения цены; совпадение по времени не подтверждает причинный эффект.</p>
    </section>}

    <p className="tariff-method-note">Медиана требует совпадения CPU, RAM, диска, региона, НДС и условий трафика минимум у двух провайдеров. Если её нет, таблица показывает ближайший тариф с ресурсами в пределах 0,5–2× в том же регионе; различия условий раскрыты в карточке. Цены нормализованы за месяц, даты наблюдения показаны отдельно.</p>
  </section>;
}
