import { useEffect, useMemo, useState } from 'react';
import { Link } from 'react-router-dom';
import { Activity, ArrowDownRight, ArrowRight, ArrowUpRight, CircleHelp, Clock3, Database, ExternalLink, RefreshCw, ShieldCheck, TriangleAlert } from 'lucide-react';
import { fetchPriceImpact, summarizePriceImpact, type PriceImpactEvent } from '../priceImpact';
import { GlassSelect } from '../../components/ui/GlassSelect';
import { compareCurrentOffers } from './marketComparison';
import './market-monitoring.css';

type Quality = {
  providerCode: string; provider: string; products: number; configurations: number;
  coreSpecsKnown: number; regionsKnown: number; diskTypeKnown: number; termsNormalized: number;
  trafficKnown: number; specialConditionsKnown: number; completePrices: number;
  foreignCurrencyPrices: number; missingFxPrices: number;
};
type Tariff = {
  providerCode: string; provider: string; planKey: string; planName: string;
  cpuCores: number | null; ramGib: number | null; diskGib: number | null;
  diskType: string | null; region: string | null; vatIncluded: boolean | null;
  trafficGib: number | null; trafficUnlimited: boolean | null;
  observedOn: string; retrievedAt: string; sourceUrl: string; sourceKind: string; priceRub: number | null;
  originalPrice: number | null; currency: string | null; billingPeriod: string | null;
  priceKind: string; parseStatus: string; rubRateOn: string | null;
  marketMedianRub: number | null; providersAtMedian: number; historyPoints: number;
};
type Change = {
  providerCode: string; provider: string; planKey: string; planName: string;
  cpuCores: number | null; ramGib: number | null; diskGib: number | null; region: string | null;
  observedAt: string; retrievedAt: string; sourceUrl: string; sourceKind: string;
  previousPriceMinor: number; currentPriceMinor: number; currency: string; changePercent: number;
};
type Source = {
  providerCode: string; provider: string; websiteUrl: string; tariffPageUrls: string[];
  lastRetrievedAt: string | null; lastCompleteAt: string | null;
  completeObservations: number; incompleteObservations: number;
};
type Data = { quality: Quality[]; tariffs: Tariff[]; changes: Change[]; sources: Source[]; impact: PriceImpactEvent[] };
const periods = [7, 30, 90] as const;
const number = (value: number) => new Intl.NumberFormat('ru-RU').format(value);
const percent = (value: number) => `${value > 0 ? '+' : ''}${new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 1 }).format(value)}%`;
const day = (value: string) => new Date(value).toLocaleDateString('ru-RU', { day: 'numeric', month: 'short', year: 'numeric' });
const money = (minor: number, currency: string) => new Intl.NumberFormat('ru-RU', { style: 'currency', currency }).format(minor / 100);
const rub = (value: number) => new Intl.NumberFormat('ru-RU', { style: 'currency', currency: 'RUB', maximumFractionDigits: 0 }).format(value);
const decimal = (value: number | null) => value === null ? '—' : new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 2 }).format(value);
const offerKey = (row: Tariff) => [row.providerCode, row.planKey, row.cpuCores, row.ramGib, row.diskGib, row.diskType, row.region].join('|');
const comparisonTierLabel = { exact: 'Точное совпадение', close: 'Близкие ресурсы', reference: 'Ценовой ориентир' };
const tariffUrl = (row: { providerCode: string; planKey: string; cpuCores?: number | null; ramGib?: number | null; diskGib?: number | null; region?: string | null }) => `/market?tab=prices&provider=${encodeURIComponent(row.providerCode)}&plan=${encodeURIComponent(row.planKey)}&cpu=${row.cpuCores ?? ''}&ram=${row.ramGib ?? ''}&disk=${row.diskGib ?? ''}&region=${encodeURIComponent(row.region ?? '')}`;
const simulatorUrl = (row: Tariff) => `/market-simulator?from=market&plan=${encodeURIComponent(row.planKey)}&name=${encodeURIComponent(row.planName)}&cpu=${row.cpuCores ?? ''}&ram=${row.ramGib ?? ''}&disk=${row.diskGib ?? ''}&region=${encodeURIComponent(row.region ?? '')}&price=${row.priceRub ?? ''}`;
async function json<T>(url: string): Promise<T> {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return response.json() as Promise<T>;
}

export default function MarketMonitoring() {
  const [data, setData] = useState<Data | null>(null);
  const [error, setError] = useState('');
  const [impactError, setImpactError] = useState(false);
  const [loading, setLoading] = useState(true);
  const [period, setPeriod] = useState<(typeof periods)[number]>(30);
  const [qualityOpen, setQualityOpen] = useState(false);
  const [provider, setProvider] = useState('all');
  const [page, setPage] = useState(0);
  const [comparisonPlan, setComparisonPlan] = useState('');
  const [comparisonPage, setComparisonPage] = useState(0);

  async function load() {
    setLoading(true); setError('');
    try {
      const [quality, tariffs, changes, sources, impact] = await Promise.all([
        json<Quality[]>('/api/market/prices/quality'),
        json<Tariff[]>('/api/market/prices/monitor'),
        json<Change[]>('/api/market/prices/changes'),
        json<Source[]>('/api/market/prices/sources'),
        fetchPriceImpact().then(rows => { setImpactError(false); return rows; }).catch(() => { setImpactError(true); return []; }),
      ]);
      setData({ quality, tariffs, changes, sources, impact });
    } catch (reason) { setError(reason instanceof Error ? reason.message : 'Ошибка загрузки рынка'); }
    finally { setLoading(false); }
  }
  useEffect(() => { void load(); }, []);

  const cutoff = Date.now() - period * 86_400_000;
  const changes = useMemo(() => (data?.changes ?? []).filter(row => row.providerCode !== 'aeza' && Date.parse(row.observedAt) >= cutoff && (provider === 'all' || row.providerCode === provider)), [data, cutoff, provider]);
  const observations = useMemo(() => (data?.tariffs ?? []).filter(row => row.providerCode !== 'aeza' && row.priceRub !== null && Date.parse(row.observedOn) >= cutoff && (provider === 'all' || row.providerCode === provider))
    .sort((a, b) => b.observedOn.localeCompare(a.observedOn) || a.provider.localeCompare(b.provider) || a.planName.localeCompare(b.planName)), [data, cutoff, provider]);
  const showChanges = changes.length > 0;
  const feedCount = showChanges ? changes.length : observations.length;
  const providers = (data?.quality ?? []).filter(row => row.configurations > 0);
  const strictAeza = (data?.tariffs ?? []).filter(row => row.providerCode === 'aeza' && row.priceRub !== null && row.marketMedianRub !== null && row.providersAtMedian >= 2);
  const totalAeza = (data?.tariffs ?? []).filter(row => row.providerCode === 'aeza').length;
  const opportunities = strictAeza.map(row => {
    const events = (data?.impact ?? []).filter(event => event.planKey === row.planKey);
    const seen = new Set<string>();
    const study = events.filter(event => { if (seen.has(event.serviceId)) return false; seen.add(event.serviceId); return true; });
    return { row, gap: (row.priceRub! / row.marketMedianRub! - 1) * 100,
      clientChanges: events.length, outcomes: summarizePriceImpact(study) };
  })
    .filter(item => item.gap >= 10).sort((a, b) => b.gap - a.gap).slice(0, 6);
  const freshOffers = (data?.tariffs ?? []).filter(row => row.providerCode !== 'aeza' && row.historyPoints === 1 && Date.parse(row.observedOn) >= cutoff).slice(0, 3);
  const latestChange = changes[0];
  const pages = Math.max(1, Math.ceil(feedCount / 10));
  const safePage = Math.min(page, pages - 1);
  const visibleChanges = changes.slice(safePage * 10, (safePage + 1) * 10);
  const visibleObservations = observations.slice(safePage * 10, (safePage + 1) * 10);
  const providerOptions = [{ code: 'all', name: 'Все провайдеры' }, ...providers.filter(row => row.providerCode !== 'aeza').map(row => ({ code: row.providerCode, name: row.provider }))];
  const pricedTariffs = useMemo(() => (data?.tariffs ?? []).filter(row => row.priceRub !== null && row.priceRub > 0 && row.billingPeriod === 'month' && row.parseStatus === 'complete')
    .sort((a, b) => Number(b.providerCode === 'aeza') - Number(a.providerCode === 'aeza') || a.provider.localeCompare(b.provider) || a.planName.localeCompare(b.planName)), [data]);
  const selectedPlan = pricedTariffs.find(row => offerKey(row) === comparisonPlan) ?? pricedTariffs[0];
  const currentComparisons = useMemo(() => selectedPlan ? compareCurrentOffers(selectedPlan, pricedTariffs) : [], [selectedPlan, pricedTariffs]);
  const comparableNow = currentComparisons.filter(row => row.tier !== 'reference');
  const aezaComparable = useMemo(() => pricedTariffs.filter(row => row.providerCode === 'aeza' && compareCurrentOffers(row, pricedTariffs).some(peer => peer.tier !== 'reference')).length, [pricedTariffs]);
  const priceOpportunities = useMemo(() => pricedTariffs.filter(row => row.providerCode === 'aeza').flatMap(row => {
    const competitor = compareCurrentOffers(row, pricedTariffs).find(peer => peer.tier !== 'reference');
    return competitor ? [{ row, competitor, gap: (row.priceRub! / competitor.offer.priceRub! - 1) * 100 }] : [];
  }).sort((a, b) => Math.abs(b.gap) - Math.abs(a.gap)).slice(0, 6), [pricedTariffs]);
  const comparisonPages = Math.max(1, Math.ceil(currentComparisons.length / 10));
  const currentComparisonPage = Math.min(comparisonPage, comparisonPages - 1);
  const visibleComparisons = currentComparisons.slice(currentComparisonPage * 10, (currentComparisonPage + 1) * 10);

  return <section className="market-monitor" aria-label="Мониторинг рынка">
    <header className="monitor-heading"><div><span className="monitor-kicker"><Activity size={15}/> МОНИТОРИНГ РЫНКА</span><h2>Что изменилось на рынке</h2><p>История цен, полнота тарифов и сигналы, которые стоит проверить.</p></div><button type="button" onClick={() => void load()} disabled={loading}><RefreshCw size={16} className={loading ? 'spin' : ''}/>Обновить</button></header>
    {error && <div className="monitor-error" role="alert"><TriangleAlert size={18}/><span>Не удалось получить данные мониторинга: {error}. {data && 'Ниже сохранён предыдущий результат.'}</span></div>}
    {loading && !data ? <div className="monitor-empty">Загружаем наблюдения и проверяем качество данных…</div> : data ? <>
      <div className="monitor-kpis">
        <article><Database size={19}/><span>Провайдеры</span><strong>{number(providers.length)}</strong><small>с конфигурациями в каталоге</small></article>
        <article><Activity size={19}/><span>Тарифы</span><strong>{number(providers.reduce((sum, row) => sum + row.products, 0))}</strong><small>различимых продуктов</small></article>
        <article><Clock3 size={19}/><span>Наблюдения цен</span><strong>{number(observations.length)}</strong><small>у конкурентов за {period} дней · изменений: {changes.length}</small></article>
        <button type="button" className="monitor-kpi-action" onClick={() => document.getElementById('market-current-comparison')?.scrollIntoView({ behavior: 'smooth', block: 'start' })}><ShieldCheck size={19}/><span>Aéza с близкими предложениями</span><strong>{totalAeza ? `${aezaComparable} / ${totalAeza}` : '—'}</strong><small>по CPU, RAM и диску · сравнить <ArrowRight size={13}/></small></button>
      </div>

      <section className="monitor-panel monitor-comparison" id="market-current-comparison">
        <div className="monitor-panel-head"><div><span className="monitor-kicker">ПОСЛЕДНИЕ НАБЛЮДЕНИЯ</span><h3>Сравнение тарифов по сохранённым ценам</h3><p>Цена за месяц в рублях, характеристики, регион и условия рядом. История изменений для этого сравнения не требуется.</p></div></div>
        {selectedPlan ? <>
          <label className="monitor-comparison-select">Тариф для сравнения
            <GlassSelect value={offerKey(selectedPlan)} onChange={value => { setComparisonPlan(value); setComparisonPage(0); }} searchable searchPlaceholder="Поиск тарифа или провайдера" options={pricedTariffs.map(row => ({ value: offerKey(row), label: `${row.provider} · ${row.planName} · ${decimal(row.cpuCores)} CPU / ${decimal(row.ramGib)} GB · ${row.region ?? 'регион неизвестен'}` }))}/>
          </label>
          <div className="monitor-comparison-summary">
            <div><small>Выбранный тариф</small><strong>{rub(selectedPlan.priceRub!)}</strong><span>{selectedPlan.provider} · {selectedPlan.planName} · {day(selectedPlan.observedOn)}</span></div>
            <div><small>Точных совпадений</small><strong>{number(currentComparisons.filter(row => row.tier === 'exact').length)}</strong><span>характеристики, регион и условия</span></div>
            <div><small>Близких предложений</small><strong>{number(comparableNow.length)}</strong><span>включая точные · от {number(new Set(comparableNow.map(row => row.offer.providerCode)).size)} провайдеров</span></div>
            <div><small>Ценовых ориентиров</small><strong>{number(currentComparisons.length - comparableNow.length)}</strong><span>отличаются регион или параметры</span></div>
          </div>
          {visibleComparisons.length ? <div className="monitor-comparison-table"><table><thead><tr><th>Провайдер / тариф</th><th>CPU · RAM · диск</th><th>Регион и сходство</th><th>Цена / мес.</th><th>К выбранному тарифу</th><th>Наблюдение</th></tr></thead><tbody>{visibleComparisons.map(({ offer, tier, reasons, differencePercent }) => <tr key={offerKey(offer)}>
            <td><strong>{offer.provider}</strong><small>{offer.planName}</small></td>
            <td>{decimal(offer.cpuCores)} CPU · {decimal(offer.ramGib)} GB RAM · {decimal(offer.diskGib)} GB<small>{offer.diskType ?? 'тип диска неизвестен'}</small></td>
            <td><span className={`monitor-comparison-tier is-${tier}`}>{comparisonTierLabel[tier]}</span><small>{offer.region ?? 'регион неизвестен'}</small><small>{reasons.length ? reasons.join(' · ') : 'Условия совпадают'}</small></td>
            <td><strong>{rub(offer.priceRub!)}</strong><small>{offer.originalPrice === null ? 'Исходная цена не указана' : `${decimal(offer.originalPrice)} ${offer.currency ?? ''} / ${offer.billingPeriod ?? 'период не указан'}`}</small></td>
            <td><strong className={differencePercent < 0 ? 'monitor-comparison-cheaper' : 'monitor-comparison-higher'}>{percent(differencePercent)}</strong><small>{differencePercent < 0 ? 'дешевле' : differencePercent > 0 ? 'дороже' : 'та же цена'} выбранного тарифа</small></td>
            <td><small>{day(offer.observedOn)} · {offer.sourceKind === 'archive' ? 'архив' : 'сбор'}</small><a href={offer.sourceUrl} target="_blank" rel="noreferrer">Источник <ExternalLink size={13}/></a></td>
          </tr>)}</tbody></table></div> : <div className="monitor-empty">Для сравнения нужен хотя бы один тариф другого провайдера с сохранённой месячной ценой.</div>}
          {currentComparisons.length > 10 && <div className="monitor-pagination"><span>Показано {currentComparisonPage * 10 + 1}–{Math.min((currentComparisonPage + 1) * 10, currentComparisons.length)} из {currentComparisons.length}</span><div><button type="button" disabled={currentComparisonPage === 0} onClick={() => setComparisonPage(currentComparisonPage - 1)}>Назад</button><button type="button" disabled={currentComparisonPage >= comparisonPages - 1} onClick={() => setComparisonPage(currentComparisonPage + 1)}>Далее</button></div></div>}
          <p className="monitor-comparison-note">Разница = цена конкурента / цена выбранного тарифа − 1. Близкие предложения имеют ресурсы в пределах 0,5–2×; если регион или тип диска неизвестен, это отмечено в строке. Известное несовпадение региона или типа диска переносит тариф в «ценовые ориентиры». НДС, трафик, дата и источник показаны отдельно. Это сравнение цен, а не строгая рыночная медиана.</p>
        </> : <div className="monitor-empty">В сохранённом срезе пока нет тарифов с месячной ценой для сравнения.</div>}
      </section>

      <div className="monitor-main-grid">
        <section className="monitor-panel monitor-events"><div className="monitor-panel-head"><div><span className="monitor-kicker">ЛЕНТА НАБЛЮДЕНИЙ</span><h3>{showChanges ? 'Подтверждённые изменения цен' : 'Последние цены конкурентов'}</h3><p>{showChanges ? 'Один тариф и валюта, одинаковый месячный период оплаты.' : 'В сохранённых снимках цена тарифа пока не менялась. Ниже — последние наблюдения с датой и источником.'}</p></div><div className="monitor-period" role="group" aria-label="Период наблюдений">{periods.map(value => <button type="button" key={value} aria-pressed={period === value} onClick={() => { setPeriod(value); setPage(0); }}>{value} дн.</button>)}</div></div><div className="monitor-filter"><label>Провайдер<GlassSelect value={provider} onChange={value => { setProvider(value); setPage(0); }} options={providerOptions.map(item => ({ value: item.code, label: item.name }))}/></label><span>{feedCount} {showChanges ? 'изменений' : 'ценовых наблюдений'}</span></div>
          {showChanges ? <div className="monitor-event-list">{visibleChanges.map((row, index) => <article key={`${row.providerCode}-${row.planKey}-${row.observedAt}-${index}`}><span className={`monitor-change-icon ${row.changePercent > 0 ? 'up' : 'down'}`}>{row.changePercent > 0 ? <ArrowUpRight size={19}/> : <ArrowDownRight size={19}/>}</span><div className="monitor-event-body"><div><strong>{row.provider} · {row.planName}</strong><b className={row.changePercent > 0 ? 'up' : 'down'}>{percent(row.changePercent)}</b></div><p>{money(row.previousPriceMinor, row.currency)} → {money(row.currentPriceMinor, row.currency)} <span>· {row.cpuCores ?? '?'} CPU / {row.ramGib ?? '?'} GB RAM · {row.region ?? 'регион неизвестен'}</span></p><small>Наблюдение {day(row.observedAt)} · {row.sourceKind === 'archive' ? 'архив' : 'публичный источник'}</small></div><a href={row.sourceUrl} target="_blank" rel="noreferrer" aria-label={`Источник цены ${row.provider} ${row.planName}`}><ExternalLink size={16}/></a></article>)}</div>
            : visibleObservations.length ? <div className="monitor-event-list">{visibleObservations.map(row => <article key={offerKey(row)}><span className="monitor-change-icon observed"><Database size={18}/></span><div className="monitor-event-body"><div><strong>{row.provider} · {row.planName}</strong><b className="observed">{rub(row.priceRub!)}</b></div><p>{decimal(row.cpuCores)} CPU / {decimal(row.ramGib)} GB RAM / {decimal(row.diskGib)} GB диск <span>· {row.region ?? 'регион неизвестен'}</span></p><small>Цена от {day(row.observedOn)} · {row.sourceKind === 'archive' ? 'архив' : 'публичный источник'} · без подтверждённого изменения</small></div><a href={row.sourceUrl} target="_blank" rel="noreferrer" aria-label={`Источник цены ${row.provider} ${row.planName}`}><ExternalLink size={16}/></a></article>)}</div>
              : <div className="monitor-empty">За выбранный период нет сохранённых цен конкурентов. Выберите более длинный период или проверьте источники ниже.</div>}
          <div className="monitor-pagination"><span>Страница {safePage + 1} из {pages}</span><div><button type="button" disabled={safePage === 0} onClick={() => setPage(safePage - 1)}>Назад</button><button type="button" disabled={safePage >= pages - 1} onClick={() => setPage(safePage + 1)}>Далее</button></div></div>
        </section>

        <aside className="monitor-side"><section className="monitor-panel monitor-opportunities"><div className="monitor-panel-head"><div><span className="monitor-kicker">ПОВОД ДЛЯ ПРОВЕРКИ</span><h3>Рыночные возможности и риски</h3><p>{opportunities.length ? 'Цены Aéza относительно строгой медианы.' : 'Aéza и конкуренты с близкими ресурсами; неизвестные условия отмечены.'}</p></div></div>
          {opportunities.length ? opportunities.map(({ row, gap, clientChanges, outcomes }) => <article key={`${row.planKey}-${row.region}-${row.cpuCores}-${row.diskGib}`}><div className="monitor-signal-top"><span>Выше медианы</span><b>{percent(gap)}</b></div><strong>{row.planName}</strong><p>{rub(row.priceRub!)} у Aéza · {rub(row.marketMedianRub!)} медиана · {row.providersAtMedian} аналога</p><small>Наблюдение {day(row.observedOn)}</small><div className="monitor-client-evidence">{impactError?'Внутренняя история временно недоступна':!clientChanges?'По связанным услугам ещё нет зарегистрированных повышений':outcomes.quality === 'insufficient'?`${clientChanges} внутренних повышений · для оценки исходов пока мало полных наблюдений`:`После ${clientChanges} повышений: ${outcomes.renewed} подтверждённых продлений, ${outcomes.adverse} негативных событий; ${outcomes.unknown} исходов неизвестны`}</div><div className="monitor-signal-actions"><Link to={tariffUrl(row)}>Тариф <ArrowRight size={13}/></Link><Link to={`/price-impact?plan=${encodeURIComponent(row.planKey)}`}>Клиенты <ArrowRight size={13}/></Link><Link to={simulatorUrl(row)}>В сценарии <ArrowRight size={13}/></Link></div></article>)
            : priceOpportunities.length ? priceOpportunities.map(({ row, competitor, gap }) => <article key={offerKey(row)}><div className="monitor-signal-top"><span>{gap >= 0 ? 'Aéza дороже' : 'Aéza дешевле'}</span><b>{percent(gap)}</b></div><strong>{row.planName}</strong><p>Aéza {rub(row.priceRub!)} · {competitor.offer.provider} {rub(competitor.offer.priceRub!)}</p><small>{competitor.offer.planName} · снимок {day(competitor.offer.observedOn)}</small><div className="monitor-client-evidence">Близкие ресурсы. {competitor.reasons.join('; ') || 'Характеристики и условия совпадают'}. Сравните полный платёж и доступность в регионе перед решением.</div><div className="monitor-signal-actions"><Link to={tariffUrl(row)}>Тариф Aéza <ArrowRight size={13}/></Link><a href={competitor.offer.sourceUrl} target="_blank" rel="noreferrer">Источник конкурента <ExternalLink size={13}/></a></div></article>)
              : <div className="monitor-empty">Предложений с близкими ресурсами пока нет. Все сохранённые тарифы и их отличия показаны в сравнении выше.</div>}
          {latestChange && <div className="monitor-side-note"><strong>Последнее изменение конкурента</strong><span>{latestChange.provider} · {percent(latestChange.changePercent)} · {day(latestChange.observedAt)}</span><a href={latestChange.sourceUrl} target="_blank" rel="noreferrer">Открыть источник <ExternalLink size={13}/></a></div>}
          {freshOffers.length > 0 && <div className="monitor-side-note"><strong>Впервые зафиксированные предложения</strong>{freshOffers.map(row => <Link key={`${row.providerCode}-${row.planKey}-${row.region}-${row.cpuCores}`} to={tariffUrl(row)}>{row.provider} · {row.planName} <ArrowRight size={13}/></Link>)}</div>}
          <p className="monitor-caveat">Разница цен и события клиентов — отдельные наблюдения. Их совпадение не доказывает причину оттока.</p></section></aside>
      </div>

      <section className="monitor-panel monitor-quality" id="market-quality"><div className="monitor-panel-head"><div><span className="monitor-kicker">ПРОЗРАЧНОСТЬ ДАННЫХ</span><h3>Источники и качество</h3><p>Даты записанных наблюдений и полнота извлечения. Состояние работающего парсера отдельно не измеряется.</p></div><button type="button" onClick={() => setQualityOpen(value => !value)} aria-expanded={qualityOpen}>{qualityOpen ? 'Свернуть' : 'Почему данные не сравниваются'} <CircleHelp size={15}/></button></div><div className="monitor-source-grid">{data.sources.map(source => { const quality = data.quality.find(row => row.providerCode === source.providerCode); return <article key={source.providerCode}><div><span className={source.lastCompleteAt ? 'monitor-source-dot' : 'monitor-source-dot missing'}/><strong>{source.provider}</strong></div><p>{quality?.configurations ?? 0} конфигураций · {source.completeObservations} полных наблюдений</p><small>{source.lastCompleteAt ? `Последняя полная запись ${day(source.lastCompleteAt)}` : 'Полных наблюдений нет'}</small>{source.incompleteObservations > 0 && <small className="monitor-source-warning">Неполных записей: {source.incompleteObservations}</small>}<a href={source.tariffPageUrls[0] ?? source.websiteUrl} target="_blank" rel="noreferrer">Источник <ExternalLink size={13}/></a></article>; })}</div>
        {qualityOpen && <div className="monitor-quality-detail"><p><strong>Почему строгих аналогов может не быть:</strong> сравниваются CPU, RAM, диск, регион, НДС и трафик; нужны минимум два независимых провайдера. Пустое поле не приравнивается к совпадению.</p><div className="monitor-quality-table"><table><thead><tr><th>Провайдер</th><th>Конфигураций</th><th>Нет CPU/RAM/диска</th><th>Нет региона</th><th>Нет типа диска</th><th>Нет условий трафика</th><th>Нет курса</th></tr></thead><tbody>{providers.map(item => <tr key={item.providerCode}><td>{item.provider}</td><td>{number(item.configurations)}</td><td>{number(item.configurations - item.coreSpecsKnown)}</td><td>{number(item.configurations - item.regionsKnown)}</td><td>{number(item.configurations - item.diskTypeKnown)}</td><td>{number(item.configurations - item.trafficKnown)}</td><td>{number(item.missingFxPrices)}</td></tr>)}</tbody></table></div><p>Счётчики описывают пропуски полей в каталоге. Один тариф может входить сразу в несколько столбцов.</p></div>}
      </section>
    </> : null}
  </section>;
}
