import { useState } from 'react';
import { BarChart, Bar, LineChart, Line, CartesianGrid, XAxis, YAxis, Tooltip, Legend, ReferenceLine, ResponsiveContainer } from 'recharts';
import { RefreshCw, Play, Download } from 'lucide-react';
import { Panel } from '../../components/common';
import { Button } from '../../components/ui/button';
import { GlassSelect } from '../../components/ui/GlassSelect';
import { MarketApi, type Scenario, type Sensitivity } from './api';
import type { SeedStatistics, StrategyStatistics } from './statisticsTypes';
import { number2 } from './format';
import { sameScenario } from './scenario';
import {useCurrency} from '../../hooks/useCurrency';

const api = new MarketApi();
const names: Record<string, string> = { baseline: 'Базовая цена', custom: 'Своя стратегия', discount_20: 'Скидка 20%', increase_20: 'Повышение 20%', intro_then_increase: 'Скидка → повышение', rise_36: 'Повышение 36%', cut_28: 'Скидка 28%', discount_10: 'Скидка 10%', increase_15: 'Повышение 15%' };
const palette = ['#89d6ff', '#b9a0ff', '#7cdebd', '#f9bc7c', '#f4a5c2'];
const levels: Sensitivity[] = ['low', 'medium', 'high'];
const sensitivityNames: Record<Sensitivity, string> = { low: 'Низкая', medium: 'Средняя', high: 'Высокая' };
const signedNumber = (n: number) => `${n > 0 ? '+' : ''}${number2(n)}`;
const signedPercent = (value:number,base:number) => base===0?'—':`${value-base>0?'+':''}${number2((value-base)/Math.abs(base)*100)}%`;
const median = (values: number[]) => {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const middle = Math.floor(sorted.length / 2);
  return sorted.length % 2 ? sorted[middle] : (sorted[middle - 1] + sorted[middle]) / 2;
};
const med = (strategy: StrategyStatistics, key: 'revenue_rub' | 'final_active' | 'renewal_events' | 'competitor_switch_out_events' | 'outside_churn_events' | 'loss_events') => median(strategy.points.map(point => point[key]));
const segmentMed = (strategy:StrategyStatistics,segmentIndex:number,key:'modeled_decisions'|'purchases'|'renewals'|'loss_events'|'final_active'|'revenue_rub')=>median(strategy.points.map(point=>point.segments?.find(row=>row.segment_index===segmentIndex)?.[key]??0));
type CachedResult = { data: SeedStatistics; signature: Scenario };

export function Statistics({ scenario }: { scenario: Scenario }) {
  const {convertRub} = useCurrency();
  const rub = (amount: number) => convertRub(amount, scenario.valuation_fx);
  const range = (a: number, b: number) => `${rub(a)} — ${rub(b)}`;
  const signedRub = (amount: number) => `${amount > 0 ? '+' : ''}${rub(amount)}`;
  const [level, setLevel] = useState<Sensitivity>('medium');
  const [mode, setMode] = useState<'selected' | 'audit'>('selected');
  const [cache, setCache] = useState<Partial<Record<Sensitivity, CachedResult>>>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  const input = structuredClone(scenario);
  input.seed = 0;
  input.sensitivities = [level];
  if (mode === 'audit') {
    input.strategies = [
      { name: 'baseline', aeza_price_multipliers: Array(input.months).fill(1) },
      { name: 'rise_36', aeza_price_multipliers: Array(input.months).fill(1.36) },
      { name: 'cut_28', aeza_price_multipliers: Array(input.months).fill(0.72) }
    ];
  } else if (!input.strategies.some(strategy => strategy.name === 'baseline')) {
    input.strategies = [{ name: 'baseline', aeza_price_multipliers: Array(input.months).fill(1) }, ...input.strategies];
  }
  const signature = structuredClone(input);
  signature.sensitivities = [];
  const current = cache[level];
  const stale = current !== undefined && !sameScenario(signature, current.signature);
  const result = current?.data;
  const matching = levels.flatMap(sensitivity => {
    const item = cache[sensitivity];
    return item && sameScenario(item.signature, signature) ? [[sensitivity, item.data] as const] : [];
  });

  async function run() {
    setBusy(true);
    setError('');
    try {
      const data = await api.statistics(input, level);
      setCache(previous => ({ ...previous, [level]: { data, signature } }));
    } catch (e) {
      setError(e instanceof Error ? e.message : 'Ошибка статистики');
    } finally {
      setBusy(false);
    }
  }

  function download() {
    if (!result) return;
    const u = URL.createObjectURL(new Blob([JSON.stringify(result, null, 2)], { type: 'application/json' }));
    const a = document.createElement('a');
    a.href = u;
    a.download = `statistics-${result.replay_sha256.slice(0, 12)}.json`;
    a.click();
    URL.revokeObjectURL(u);
  }

  const baseline = result?.strategies.find(strategy => strategy.strategy === 'baseline');
  const chart = result?.seeds.map((seed, i) => ({ seed, ...Object.fromEntries(result.strategies.map(strategy => [strategy.strategy, strategy.points[i]?.profit_rub])) })) ?? [];
  const frequencies = result?.strategies.map(strategy => ({ name: names[strategy.strategy] ?? strategy.strategy, risk: strategy.loss_probability_percent, sole: strategy.sole_wins, tie: strategy.tied_wins })) ?? [];
  const leaders = result?.strategies.filter(strategy => strategy.leading_percent === Math.max(...result.strategies.map(item => item.leading_percent))) ?? [];

  return <section aria-label="Статистика сценариев"><Panel title="Сравнение сценариев" subtitle="Базовая линия, разброс результатов и устойчивость при разных допущениях">
    <div className="market-fields">
      <label>Набор стратегий<GlassSelect aria-label="Стратегии статистики" value={mode} onChange={value => setMode(value as 'audit' | 'selected')} options={[{ value: 'audit', label: 'Базовая · +36% · −28%' }, { value: 'selected', label: 'Выбранные в симуляции' }]} /></label>
      <label>Ценовая чувствительность<GlassSelect aria-label="Чувствительность статистики" value={level} onChange={value => setLevel(value as Sensitivity)} options={levels.map(value => ({ value, label: sensitivityNames[value] }))} /></label>
    </div>
    <p className="market-hint">Каждый запуск считает 100 вариантов seed 0–99. Настройки рынка фиксированы; разброс отражает только случайность внутри заданной модели и не доказывает точность прогноза реального рынка. В режиме выбранных стратегий базовая цена добавляется автоматически, если её нет. Чтобы сравнить чувствительность, отдельно запустите расчёт для каждого уровня: результаты сохраняются и не запускают дополнительные вычисления сами.</p>
    <div className="market-submit"><Button disabled={busy || !input.strategies.length} onClick={() => void run()}>{busy ? <RefreshCw size={16} className="spin" /> : <Play size={16} />}{busy ? 'Считаем 100 seed…' : `Рассчитать · ${sensitivityNames[level].toLowerCase()} чувствительность`}</Button>{result && <Button onClick={download}><Download size={15} />JSON статистики</Button>}<span role="status">{busy ? 'Результаты появятся после завершения запусков' : stale ? 'Настройки изменились — ниже предыдущий расчёт' : ''}</span></div>
    {error && <p className="notice" role="alert">{error}</p>}

    {matching.length > 1 && <>
      <h3>Устойчивость к ценовой чувствительности</h3>
      <p className="market-hint">Сравнение базовой стратегии по уже выполненным запускам. Здесь нет автоматических дополнительных расчётов.</p>
      <div className="table-scroll"><table><thead><tr><th>Чувствительность</th><th>Медиана прибыли</th><th>Медиана выручки</th><th>Активные аренды в конце</th><th>Продления</th><th>События ухода</th><th>Seed</th></tr></thead><tbody>{matching.map(([sensitivity, data]) => {
        const base = data.strategies.find(strategy => strategy.strategy === 'baseline');
        return base ? <tr key={sensitivity}><td>{sensitivityNames[sensitivity]}</td><td>{rub(base.profit.median_rub)}</td><td>{rub(med(base, 'revenue_rub'))}</td><td>{number2(med(base, 'final_active'))}</td><td>{number2(med(base, 'renewal_events'))}</td><td>{number2(med(base, 'loss_events'))}</td><td>{data.seed_count}</td></tr> : null;
      })}</tbody></table></div>
    </>}

    {result && <>
      <p className="market-hint">Текущий набор: {sensitivityNames[result.sensitivity].toLowerCase()} чувствительность · {result.seed_count} seed · {result.strategies.length} стратегий{stale ? ' · параметры уже изменены' : ''}</p>
      <div className="market-recommendation"><span className="eyebrow">ЧАЩЕ ВСЕГО ЛУЧШИЙ РЕЗУЛЬТАТ В ЭТИХ SEED</span><strong>{leaders.map(strategy => names[strategy.strategy] ?? strategy.strategy).join(' / ')}</strong><span>{number2(leaders[0]?.leading_percent ?? 0)}% запусков, включая ничьи</span></div>
      {result.tied_seed_count > 0 && <p className="market-hint">В {result.tied_seed_count} запусках несколько стратегий получили одинаковую максимальную прибыль. Это ничьи, а не отдельные победы каждой стратегии.</p>}
      <h3>Результаты за весь период · медиана 100 запусков</h3>
      <div className="table-scroll"><table><thead><tr><th>Стратегия</th><th>Прибыль</th><th>Δ прибыли к базе</th><th>Выручка</th><th>Δ выручки</th><th>Активные в конце</th><th>Δ активных</th><th>Продления</th><th>Δ продлений</th><th>Уходы, события</th><th>Δ уходов</th><th>Риск убытка</th></tr></thead><tbody>{result.strategies.map(strategy => {
        const base = baseline;
        const revenue=med(strategy,'revenue_rub'),active=med(strategy,'final_active'),renewals=med(strategy,'renewal_events'),losses=med(strategy,'loss_events');
        const delta=(value:number,baseValue:number)=>`${signedNumber(value-baseValue)} · ${signedPercent(value,baseValue)}`;
        return <tr key={strategy.strategy}>
          <td>{names[strategy.strategy] ?? strategy.strategy}</td>
          <td>{rub(strategy.profit.median_rub)}</td>
          <td>{strategy===base?'Базовый сценарий':base?`${signedRub(strategy.profit.median_rub-base.profit.median_rub)} · ${signedPercent(strategy.profit.median_rub,base.profit.median_rub)}`:'—'}</td>
          <td>{rub(revenue)}</td><td>{strategy===base?'—':base?`${signedRub(revenue-med(base,'revenue_rub'))} · ${signedPercent(revenue,med(base,'revenue_rub'))}`:'—'}</td>
          <td>{number2(active)}</td><td>{strategy===base?'—':base?delta(active,med(base,'final_active')):'—'}</td>
          <td>{number2(renewals)}</td><td>{strategy===base?'—':base?delta(renewals,med(base,'renewal_events')):'—'}</td>
          <td>{number2(losses)}</td><td>{strategy===base?'—':base?delta(losses,med(base,'loss_events')):'—'}</td>
          <td>{number2(strategy.loss_probability_percent)}%</td>
        </tr>;
      })}</tbody></table></div>
      <div className="statistics-cards">{result.strategies.map(strategy => <article className="statistics-card" key={strategy.strategy}><h4>{names[strategy.strategy] ?? strategy.strategy}</h4><div className="statistics-range"><span>Минимум<strong>{rub(strategy.profit.min_rub)}</strong></span><span>Максимум<strong>{rub(strategy.profit.max_rub)}</strong></span></div><dl className="statistics-metrics"><div><dt>Разброс SD</dt><dd>{rub(strategy.profit.sample_sd_rub)}</dd></div><div><dt>Риск убытков</dt><dd>{number2(strategy.loss_probability_percent)}%<small>{strategy.loss_runs}/{result.seed_count} · ДИ {number2(strategy.loss_probability_ci95_percent[0])}–{number2(strategy.loss_probability_ci95_percent[1])}%</small></dd></div><div><dt>Лидерство</dt><dd>{number2(strategy.leading_percent)}%<small>{strategy.sole_wins} побед · {strategy.tied_wins} ничьих</small></dd></div></dl></article>)}</div>
      <h3>Результаты по сегментам · медиана 100 запусков</h3><p className="market-hint">Показаны абсолютные значения и отклонение от базовой цены: решения модели, покупки, продления, уходы, активные аренды в конце и выручка Aéza. Это распределение по заданным сегментам, не данные о реальных клиентах.</p><div className="table-scroll"><table><thead><tr><th>Стратегия</th><th>Сегмент</th><th>Решения модели</th><th>Покупки</th><th>Продления</th><th>Уходы, события</th><th>Активные в конце</th><th>Выручка</th></tr></thead><tbody>{result.strategies.flatMap(strategy=>scenario.segments.map((segment,segmentIndex)=>{const base=baseline;const value=(key:'modeled_decisions'|'purchases'|'renewals'|'loss_events'|'final_active'|'revenue_rub')=>segmentMed(strategy,segmentIndex,key);const cell=(key:'modeled_decisions'|'purchases'|'renewals'|'loss_events'|'final_active'|'revenue_rub',moneyValue=false)=>{const current=value(key);if(strategy===base)return moneyValue?rub(current):number2(current);if(!base)return '—';const baseValue=segmentMed(base,segmentIndex,key);const delta=current-baseValue;return `${moneyValue?rub(current):number2(current)} / ${moneyValue?signedRub(delta):signedNumber(delta)} (${signedPercent(current,baseValue)})`;};return <tr key={`${strategy.strategy}-${segmentIndex}`}><td>{names[strategy.strategy]??strategy.strategy}</td><td>{segment.name}</td><td>{cell('modeled_decisions')}</td><td>{cell('purchases')}</td><td>{cell('renewals')}</td><td>{cell('loss_events')}</td><td>{cell('final_active')}</td><td>{cell('revenue_rub',true)}</td></tr>;}))}</tbody></table></div>
      <details className="market-assumptions"><summary>Квантили и статистика сравнения</summary><div className="table-scroll"><table><thead><tr><th>Стратегия</th><th>Минимум → максимум</th><th>Разброс SD</th><th>Риск убытка</th><th>Лидерство</th></tr></thead><tbody>{result.strategies.map(strategy => <tr key={strategy.strategy}><td>{names[strategy.strategy] ?? strategy.strategy}<details><summary>Квантили</summary><p>Медиана: {rub(strategy.profit.median_rub)}</p><p>Центральные 95%: {range(strategy.profit.p025_rub, strategy.profit.p975_rub)}</p><p>Квартильный диапазон: {range(strategy.profit.p25_rub, strategy.profit.p75_rub)}</p></details></td><td>{range(strategy.profit.min_rub, strategy.profit.max_rub)}</td><td>{rub(strategy.profit.sample_sd_rub)}</td><td>{number2(strategy.loss_probability_percent)}% · {strategy.loss_runs}/{result.seed_count}<small className="statistics-sub">95% ДИ: {number2(strategy.loss_probability_ci95_percent[0])}–{number2(strategy.loss_probability_ci95_percent[1])}%</small></td><td>{strategy.sole_wins} побед · {strategy.tied_wins} ничьих<small className="statistics-sub">Лучший результат: {number2(strategy.leading_percent)}%<br />95% ДИ: {number2(strategy.leading_ci95_percent[0])}–{number2(strategy.leading_ci95_percent[1])}%</small></td></tr>)}</tbody></table></div></details>
      <h3>Прибыль по seed</h3><div className="simulator-chart"><ResponsiveContainer width="100%" height="100%"><LineChart data={chart}><CartesianGrid strokeDasharray="3 4" stroke="#8aa5bb30" /><XAxis dataKey="seed" minTickGap={24} /><YAxis tickFormatter={value => number2(Number(value) / 1000) + ' тыс.'} /><ReferenceLine y={0} stroke="#f4a5c2" strokeDasharray="4 3" /><Tooltip formatter={value => rub(Number(value))} labelFormatter={value => `Seed ${value}`} /><Legend />{result.strategies.map((strategy, i) => <Line key={strategy.strategy} dataKey={strategy.strategy} name={names[strategy.strategy] ?? strategy.strategy} stroke={palette[i % palette.length]} dot={false} isAnimationActive={false} />)}</LineChart></ResponsiveContainer></div>
      <div className="market-charts"><div><h3>Риск убытков, %</h3><div className="simulator-chart"><ResponsiveContainer width="100%" height="100%"><BarChart data={frequencies} layout="vertical" margin={{ left: 10, right: 15 }}><CartesianGrid strokeDasharray="3 4" stroke="#8aa5bb30" /><XAxis type="number" domain={[0, 100]} unit="%" /><YAxis type="category" dataKey="name" width={100} tick={{ fontSize: 14 }} /><Tooltip formatter={value => `${number2(Number(value))}%`} /><Bar dataKey="risk" name="Убыточные запуски" fill="#f4a5c2" isAnimationActive={false} /></BarChart></ResponsiveContainer></div></div><div><h3>Устойчивость лидерства · из {result.seed_count} запусков</h3><div className="simulator-chart"><ResponsiveContainer width="100%" height="100%"><BarChart data={frequencies} layout="vertical" margin={{ left: 10, right: 15 }}><CartesianGrid strokeDasharray="3 4" stroke="#8aa5bb30" /><XAxis type="number" domain={[0, 100]} /><YAxis type="category" dataKey="name" width={100} tick={{ fontSize: 14 }} /><Tooltip /><Legend /><Bar dataKey="sole" name="Победы" stackId="wins" fill="#89d6ff" isAnimationActive={false} /><Bar dataKey="tie" name="Ничьи на первом месте" stackId="wins" fill="#b9a0ff" isAnimationActive={false} /></BarChart></ResponsiveContainer></div></div></div>
      <details className="market-assumptions"><summary>Как читать результаты и ограничения</summary><p>Минимум и максимум — крайние результаты рассчитанных seed. Центральные 95% результатов не являются доверительным интервалом среднего или прогнозом реальной прибыли.</p><p>Риск — доля запусков с прибылью строго ниже нуля. Интервалы риска и лидерства: 95% Wilson. Ноль убытков из {result.seed_count} не доказывает нулевой риск. Лидерство сравнивает только показанные стратегии; при ничьих частота лучших результатов может превышать 100%.</p><p>Снимок рынка и условия сценария фиксируются на весь расчёт. Между seed меняются только случайные события покупателей. Это разброс внутри модели, без оценки ошибок рыночных данных, бюджетов и затрат.</p><p>{result.engine_version} · replay: {result.replay_sha256} · каталог: {result.catalog_sha256}</p>{result.limitations.map(limitation => <p key={limitation}>{limitation}</p>)}</details>
    </>}
  </Panel></section>;
}
