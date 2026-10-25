import {number2} from '../features/market/format';
import {useEffect, useRef, useState} from 'react';
import {Play, RefreshCw, Download, Server, Wallet, TrendingUp, Users, AlertTriangle} from 'lucide-react';
import {ResponsiveContainer, LineChart, Line, XAxis, YAxis, CartesianGrid, Tooltip, Legend, BarChart, Bar} from 'recharts';
import {Panel} from '../components/common';
import {Button} from '../components/ui/button';
import {GlassSelect} from '../components/ui/GlassSelect';
import {MarketApi, type Plan, type Scenario, type Report, type Sensitivity} from '../features/market/api';
import './market.css';
import {Statistics} from '../features/market/Statistics';
import {Behavior} from '../features/market/Behavior';
import {Competition} from '../features/market/Competition';
import {MarketData} from '../features/market/MarketData';
import {sameScenario, buildMarketScenario, priceChangeMultiplier, initialAezaPlans} from '../features/market/scenario';
import {MarketSimulatorWizard} from '../features/market/MarketSimulatorWizard';
import {useCurrency} from '../hooks/useCurrency';
import {useLocation} from 'react-router-dom';

const api = new MarketApi();
const sensitivityLabels: Record<Sensitivity, string> = {low: 'Низкая', medium: 'Средняя', high: 'Высокая'};
const strategyLabels: Record<string, string> = {baseline:'Базовая цена', discount_20:'Скидка 20%', increase_20:'Повышение 20%', intro_then_increase:'Скидка → повышение', custom:'Своя стратегия'};
const palette = ['#89d6ff', '#b9a0ff', '#f9bc7c', '#7cdebd', '#f4a5c2'];
const integer = (n: number) => new Intl.NumberFormat('ru-RU').format(n);
const title = (n: string) => strategyLabels[n] ?? n;
const clone = <T,>(x: T): T => structuredClone(x);

export default function MarketSimulator() {
  const location=useLocation();
  const marketParams=new URLSearchParams(location.search);
  const marketPlan=marketParams.get('from')==='market'?marketParams.get('name'):null;
  const {convertRub,currency} = useCurrency();
  const [template, setTemplate] = useState<Scenario|null>(null);
  const [draft, setDraft] = useState<Scenario|null>(null);
  const [catalog, setCatalog] = useState<Plan[]>([]);
  const [official,setOfficial]=useState<Plan[]>([]);
  const [source, setSource] = useState<Awaited<ReturnType<MarketApi['sources']>>|null>(null);
  const [loading, setLoading] = useState(true), [busy, setBusy] = useState(false), [error, setError] = useState('');
  const [report, setReport] = useState<Report|null>(null);
  const [preset, setPreset] = useState('flat'), [custom, setCustom] = useState('0');
  const [strategies, setStrategies] = useState<string[]>([]);
  const [marketType, setMarketType] = useState<'economy'|'extended'>('extended');
  const [priceSource, setPriceSource] = useState<'observed'|'manual'>('observed');
  const [sensitivity, setSensitivity] = useState<Sensitivity>('medium'), [focus, setFocus] = useState('baseline');
  const [pageView,setPageView]=useState('simulation');
  const [observedMarketPlanIds,setObservedMarketPlanIds]=useState<string[]>([]);
  const [marketMatched,setMarketMatched]=useState(false);
  const usd = (amount: number) => convertRub(amount, report?.scenario.valuation_fx ?? draft?.valuation_fx ?? source?.fx);
  const controller = useRef<AbortController|null>(null);
  const generation = useRef(0);
  async function load() {
    const id = ++generation.current; setLoading(true); setError('');
    try {
      const [s, c, provenance, own] = await Promise.all([api.example(), api.catalog(), api.sources(),api.aezaPlans()]);
      if (id !== generation.current) return;
      const requestedName=marketParams.get('name');
      const cpu=Number(marketParams.get('cpu')),ram=Number(marketParams.get('ram')),disk=Number(marketParams.get('disk'));
      const requestedRegion=marketParams.get('region');
      const matching=marketPlan?own.find(plan=>plan.name.toLocaleLowerCase()===requestedName?.toLocaleLowerCase()
        && plan.vcpu===cpu && Math.abs(plan.ram_gb-ram)<.05 && Math.abs(plan.disk_gb-disk)<.05
        && (!requestedRegion || plan.regions.includes(requestedRegion))):undefined;
      setMarketMatched(Boolean(matching));
      setTemplate(s); setDraft({...clone(s),arrival_weights:Array(s.months).fill(1),...(matching?{aeza_plans:[clone(matching)]}:{})}); setCatalog(c); setSource(provenance); setOfficial(own);
      setStrategies(matching?['baseline','custom']:[...new Set(['baseline', ...s.strategies.map(x => x.name)])]); setCustom(matching?'10':'0'); setPreset('flat'); setReport(null);
      setMarketType('extended');
      setPriceSource(s.aeza_plans.some(plan => plan.origin === 'observed_list_price' || plan.details) ? 'observed' : 'manual');
    } catch (e) { if (id === generation.current) setError(e instanceof Error ? e.message : 'Не удалось загрузить API'); }
    finally { if (id === generation.current) setLoading(false); }
  }
  useEffect(() => { void load(); return () => { generation.current++; controller.current?.abort(); }; }, []);
  if (loading) return <div className="loading-state simulator-state"><RefreshCw className="spin" size={26}/><h2>Подключаем Market API</h2><p>Загружаем сценарий, каталог и происхождение данных…</p></div>;
  if (!draft || !template) return <div className="loading-state simulator-state" role="alert"><AlertTriangle/><h2>Market API недоступен</h2><p>{error}</p><p>Запустите Rust-сервис на 127.0.0.1:8091. Настроен прокси /api/market.</p><Button onClick={() => void load()}>Повторить подключение</Button></div>;
  const update = <K extends keyof Scenario,>(key: K, value: Scenario[K]) => setDraft({...draft, [key]: value});
  function weights(months: number, selected: string) {
    return Array.from({length: months}, (_, i) => selected === 'akash' ? template!.arrival_weights[i % template!.arrival_weights.length] : 1);
  }
  function choosePreset(selected: string) {
    setPreset(selected);
    const segments = clone(template!.segments);
    if (selected === 'budget') segments.forEach(s => { s.budget_min *= .7; s.budget_max *= .7; });
    setDraft({...draft!, arrival_weights: weights(draft!.months, selected), segments});
  }
  const customChange = Number(custom);
  const previewChange = Number.isFinite(customChange) ? customChange : 0;
  const customMultiplier = priceChangeMultiplier(previewChange);
  const preparedScenario = buildMarketScenario(draft, template, strategies, previewChange);
  const scenario = marketType === 'economy'
    ? {...preparedScenario, aeza_variable_cost_usd: 0, aeza_fixed_cost_usd: 0, switching_cost_utility: 0}
    : preparedScenario;
  const stale = report !== null && !sameScenario(scenario, report.scenario);
  const sensitivities = report ? [...new Set(report.runs.map(r => r.sensitivity))] : [];
  const visibleSensitivity = sensitivities.includes(sensitivity) ? sensitivity : sensitivities[0];
  const resultSensitivity = visibleSensitivity ?? sensitivity;
  const runs = report?.runs.filter(r => r.sensitivity === visibleSensitivity) ?? [];
  const selectedRun = runs.find(r => r.strategy === focus) ?? runs[0];
  const latest = selectedRun?.months.at(-1);
  const aeza = latest?.providers.aeza;
  const chartData = selectedRun?.months.map((m, i) => Object.fromEntries([['month', m.month], ...runs.map((r, j) => [`s${j}`, r.months[i].providers.aeza.active_leases])])) ?? [];
  const financial = selectedRun?.months.map(m => ({month:m.month, revenue:m.providers.aeza.revenue_usd, cost:m.providers.aeza.cost_usd, profit:m.providers.aeza.profit_usd})) ?? [];
  const movements = selectedRun?.months.map(m => ({month:m.month, renewals:m.providers.aeza.renewals, new:m.providers.aeza.new_leases, switches:m.providers.aeza.switches_in, lost:m.providers.aeza.churn_to_outside + m.providers.aeza.switches_out})) ?? [];
  async function simulate() {
    if (strategies.includes('custom') && (!custom.trim() || !Number.isFinite(customChange) || customChange < -95 || customChange > 1900)) { setError('Изменение цены должно быть от −95% до +1900%.'); return; }
    if (!scenario.strategies.length || !scenario.sensitivities.length) { setError('Выберите хотя бы одну стратегию и чувствительность.'); return; }
    controller.current?.abort(); const abort = new AbortController(); controller.current = abort;
    setBusy(true); setError('');
    try { const result = await api.simulate(clone(scenario), abort.signal); if (!abort.signal.aborted) setReport(result); }
    catch (e) { if (!abort.signal.aborted) setError(e instanceof Error ? e.message : 'Ошибка расчёта'); }
    finally { if (!abort.signal.aborted) setBusy(false); }
  }
  function download() {
    if (!report) return;
    const url = URL.createObjectURL(new Blob([JSON.stringify(report, null, 2)], {type:'application/json'}));
    const a = document.createElement('a'); a.href = url; a.download = `market-${report.replay_sha256.slice(0,12)}.json`; a.click(); URL.revokeObjectURL(url);
  }
  function applyObservedPrices(city: string) {
    if (!draft) return false;
    const matching = initialAezaPlans(official, city);
    if (!matching.length) {
      setError(`В проверенном снимке нет тарифов Aéza для города ${city}. Выберите ручные цены или другой город.`);
      return false;
    }
    setDraft({...draft, aeza_plans: clone(matching)});
    setPriceSource('observed');
    setError('');
    return true;
  }
  return <div className="simulator-page">
    <div className="page-heading"><div><div className="eyebrow">СЦЕНАРНЫЙ ЭКСПЕРИМЕНТ</div><h1>Сценарии рынка<span className="title-dot"/></h1><p>Проверяйте гипотезы о тарифах при заданных допущениях. Результаты не являются прогнозом спроса или оттока.</p></div></div>
    {marketPlan && <div className="market-import-note"><strong>Тариф из мониторинга: {marketPlan}</strong><span>{marketMatched?`Тариф выбран в сценарии; цена и предложения конкурентов загружены из проверенного снимка. Наблюдаемая цена: ${marketParams.get('price') ?? '—'} ₽/мес. Своё изменение цены установлено на +10% — его можно изменить ниже.`:'Точного тарифа с такой конфигурацией нет в снимке симулятора. Выберите тариф вручную; наблюдение рынка не подставлено в расчёт.'}</span></div>}
    <div className="market-context"><span>{draft.market==='russia'?'Россия':draft.market==='international'?'Международный рынок':'Общий рынок'}</span><span>Расчёт в RUB · показ {currency}</span><button type="button" onClick={()=>setPageView('data')}>Сценарная модель · об источниках ↗</button></div>
    <nav className="market-navigation" aria-label="Разделы Market Simulator">{[{id:'simulation',label:'Симуляция',icon:TrendingUp},{id:'behavior',label:'Сегменты спроса',icon:Users},{id:'competition',label:'Конкуренты',icon:Server},{id:'statistics',label:'Статистика',icon:TrendingUp},{id:'data',label:'Данные',icon:Wallet}].map(({id,label,icon:Icon})=><button key={id} type="button" aria-pressed={pageView===id} onClick={()=>setPageView(id)}><Icon size={16}/>{label}</button>)}</nav>
    <div hidden={pageView!=='simulation'}>
    <MarketSimulatorWizard
      draft={draft}
      template={template}
      official={official}
      preset={preset}
      onPresetChange={choosePreset}
      strategies={strategies}
      onStrategiesChange={setStrategies}
      custom={custom}
      onCustomChange={setCustom}
      marketType={marketType}
      onMarketTypeChange={setMarketType}
      priceSource={priceSource}
      onPriceSourceChange={setPriceSource}
      onDraftChange={setDraft}
      onApplyObserved={applyObservedPrices}
      onOpenData={() => setPageView('data')}
      onCalculate={() => void simulate()}
      busy={busy}
      error={error}
      hasReport={Boolean(report)}
      report={report}
      resultSensitivity={resultSensitivity}
      stale={stale}
      statusText={report ? stale ? 'Настройки изменены — нужен новый расчёт.' : 'Результат соответствует текущим настройкам.' : 'После запуска отчёт появится ниже.'}
      onOpenStatistics={() => setPageView('statistics')}
    />

    {report && selectedRun && latest && aeza ? <section aria-label="Результаты Market API" aria-busy={busy}>
      <div className="market-result-heading"><div><h2>Результаты симуляции</h2><p>{report.engine_version} · seed {report.scenario.seed} · {report.scenario.months} мес. · базовый приток {report.scenario.buyers_per_month}/мес. · всего {selectedRun.months.at(-1)!.cumulative_buyers} покупателей{stale ? ' · Предыдущий расчёт' : ''}</p></div><Button onClick={download}><Download size={15}/>JSON отчёта</Button></div>
      {stale && <div className="notice" role="status">Показан предыдущий расчёт. Новые настройки ещё не применены к графикам.</div>}
      <div className="market-view-controls"><label>Показать чувствительность<GlassSelect aria-label="Чувствительность результатов" value={visibleSensitivity} onChange={value => setSensitivity(value as Sensitivity)} options={sensitivities.map(s => ({value:s,label:sensitivityLabels[s]}))}/></label><label>Детали стратегии<GlassSelect aria-label="Стратегия результатов" value={selectedRun.strategy} onChange={setFocus} options={runs.map(r => ({value:r.strategy,label:title(r.strategy)}))}/></label></div>
      <div className="metrics">{[{label:'Активные аренды Aéza',value:integer(aeza.active_leases),foot:`Последний месяц · доля ${(aeza.market_share*100).toFixed(1)}%`,icon:Server},{label:'Выручка за период',value:usd(selectedRun.aeza_total_revenue_usd),foot:title(selectedRun.strategy),icon:Wallet},{label:'Сценарная прибыль',value:usd(selectedRun.aeza_total_profit_usd),foot:'Выручка − сценарные постоянные и переменные затраты',icon:TrendingUp},{label:'Уход из Aéza за период',value:integer(selectedRun.months.reduce((n,m) => n+m.providers.aeza.churn_to_outside+m.providers.aeza.switches_out,0)),foot:'События: к конкурентам + без аренды',icon:Users}].map(m => <div className="metric" key={m.label}><div className="metric-top">{m.label}<m.icon size={17}/></div><strong>{m.value}</strong><div className="metric-foot">{m.foot}</div></div>)}</div>
      <div className="market-charts"><Panel title="Спрос при разных стратегиях" subtitle={`Активные аренды Aéza · чувствительность: ${sensitivityLabels[visibleSensitivity]}`}><div className="simulator-chart"><ResponsiveContainer width="100%" height="100%"><LineChart data={chartData} margin={{top:10,right:24,left:0,bottom:0}}><CartesianGrid strokeDasharray="3 4" stroke="#8aa5bb30"/><XAxis dataKey="month" tick={{fill:'var(--muted)'}}/><YAxis allowDecimals={false} tick={{fill:'var(--muted)'}}/><Tooltip/><Legend/>{runs.map((r,i) => <Line key={r.strategy} dataKey={`s${i}`} name={title(r.strategy)} stroke={palette[i]} strokeWidth={2} isAnimationActive={false}/>)}</LineChart></ResponsiveContainer></div></Panel>
      <Panel title="Выручка, затраты и прибыль" subtitle={`${title(selectedRun.strategy)} · ${currency} за каждый месяц`}><div className="simulator-chart"><ResponsiveContainer width="100%" height="100%"><LineChart data={financial} margin={{top:10,right:24,left:0,bottom:0}}><CartesianGrid strokeDasharray="3 4" stroke="#8aa5bb30"/><XAxis dataKey="month" tick={{fill:'var(--muted)'}}/><YAxis tickFormatter={number2} tick={{fill:'var(--muted)'}}/><Tooltip formatter={v => usd(Number(v))}/><Legend/><Line dataKey="revenue" name="Выручка" stroke={palette[0]} strokeWidth={2} isAnimationActive={false}/><Line dataKey="cost" name="Затраты" stroke={palette[2]} strokeWidth={2} isAnimationActive={false}/><Line dataKey="profit" name="Прибыль" stroke={palette[3]} strokeWidth={2} isAnimationActive={false}/></LineChart></ResponsiveContainer></div></Panel></div>
      <Panel title="Набор аренд и уход" subtitle={`${title(selectedRun.strategy)} · исходящие события показаны отдельным столбцом`}><div className="simulator-chart"><ResponsiveContainer width="100%" height="100%"><BarChart data={movements} margin={{top:10,right:24,left:0,bottom:0}}><CartesianGrid strokeDasharray="3 4" stroke="#8aa5bb30"/><XAxis dataKey="month" tick={{fill:'var(--muted)'}}/><YAxis allowDecimals={false} tick={{fill:'var(--muted)'}}/><Tooltip/><Legend/><Bar dataKey="renewals" name="Продления" stackId="active" fill={palette[0]} isAnimationActive={false}/><Bar dataKey="new" name="Новые / вернувшиеся" stackId="active" fill={palette[3]} isAnimationActive={false}/><Bar dataKey="switches" name="Пришли от конкурентов" stackId="active" fill={palette[1]} isAnimationActive={false}/><Bar dataKey="lost" name="Ушли из Aéza" fill={palette[2]} isAnimationActive={false}/></BarChart></ResponsiveContainer></div></Panel>
      <Panel title="Сравнение стратегий" subtitle="Итоги всего периода; аренды и доля — в последнем месяце"><div className="table-scroll"><table><thead><tr><th>Стратегия</th><th>Чувствительность</th><th>Аренды</th><th>Доля рынка</th><th>Выручка</th><th>Прибыль</th></tr></thead><tbody>{report.runs.map(r => {const m=r.months.at(-1)!.providers.aeza; return <tr key={`${r.strategy}-${r.sensitivity}`}><td><button type="button" className="text-link" onClick={() => {setFocus(r.strategy);setSensitivity(r.sensitivity);}}>{title(r.strategy)}</button></td><td>{sensitivityLabels[r.sensitivity]}</td><td>{m.active_leases}</td><td>{(m.market_share*100).toFixed(1)}%</td><td>{usd(r.aeza_total_revenue_usd)}</td><td>{usd(r.aeza_total_profit_usd)}</td></tr>;})}</tbody></table></div></Panel>
      <Panel title="Продления, переходы и отток Aéza" subtitle="Доля ухода = (к конкурентам + без аренды) / активные аренды предыдущего месяца"><div className="table-scroll"><table><thead><tr><th>Мес.</th><th>Цена ×</th><th>Цены тарифов, {currency}</th><th>Активные</th><th>CPU / RAM / storage</th><th>Продления</th><th>К конкурентам</th><th>Без аренды</th><th>Доля ухода</th><th>Выручка</th><th>Прибыль</th></tr></thead><tbody>{selectedRun.months.map((m,i) => {const a=m.providers.aeza;const previous=selectedRun.months[i-1]?.providers.aeza.active_leases??0;const multiplier=report.scenario.strategies.find(s=>s.name===selectedRun.strategy)!.aeza_price_multipliers[i];return <tr key={m.month}><td>{m.month}</td><td>{multiplier.toFixed(2)}</td><td>{report.scenario.aeza_plans.map(p => m.aeza_effective_prices_usd?.[p.id] === undefined ? '—' : usd(m.aeza_effective_prices_usd[p.id])).join(' / ')}</td><td>{a.active_leases}</td><td>{number2(m.aeza_capacity_usage.vcpu)} / {number2(m.aeza_capacity_usage.ram_gb)} / {number2(m.aeza_capacity_usage.storage_gb)} GB</td><td>{a.renewals}</td><td>{a.switches_out}</td><td>{a.churn_to_outside}</td><td>{previous?`${((a.switches_out+a.churn_to_outside)/previous*100).toFixed(1)}%`:'—'}</td><td>{usd(a.revenue_usd)}</td><td>{usd(a.profit_usd??0)}</td></tr>;})}</tbody></table></div><div className="market-hint">replay SHA256: <code>{report.replay_sha256}</code></div></Panel>
    </section> : <div className="market-empty"><Server size={25}/><h2>Рынок готов к расчёту</h2><p>Задайте цены выше и нажмите «Рассчитать через API».</p></div>}
    </div>
    <div hidden={pageView!=='statistics'}><Statistics scenario={scenario}/></div>
    <div hidden={pageView!=='behavior'}><Behavior scenario={scenario} priceSource={priceSource}/></div>
    <div hidden={pageView!=='competition'}><Competition draft={draft} catalog={catalog} source={source} observedPricePlanIds={observedMarketPlanIds} onOpenData={()=>setPageView('data')}/></div>
    <div hidden={pageView!=='data'}>
    <MarketData draft={draft} setDraft={setDraft} official={official} source={source} catalog={catalog} onObservedMatchesChange={setObservedMarketPlanIds} onPriceSourceChange={setPriceSource} onOpenSimulation={()=>setPageView('simulation')} onRefresh={async()=>{const [c,s,o]=await Promise.all([api.catalog(),api.sources(),api.aezaPlans()]);setCatalog(c);setSource(s);setOfficial(o);}}/>
    </div>
  </div>;
}
