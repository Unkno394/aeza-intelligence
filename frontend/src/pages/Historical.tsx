import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CartesianGrid, Line, LineChart, ReferenceLine, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';
import { ArrowUpRight, CalendarDays, ChevronDown, CircleAlert, Clock3, History, Info } from 'lucide-react';
import { useAnalyzer } from '../hooks/useAnalyzer';
import { Empty, Panel } from '../components/common';
import { AccountPicker } from '../components/ui/AccountPicker';
import './historical.css';

type MetricDeviation = {
  lifetime_z_score: number | null;
  short_term_z_score: number | null;
  long_term_z_score: number | null;
  short_term_mean: number | null;
  long_term_mean: number | null;
  baseline_shift: number | null;
  score: number | null;
  observations: number;
};
type Transition = {
  stream_id: string;
  from: string;
  to: string;
  elapsed_ms: number;
  prior_count: number;
  outgoing_count: number;
  transition_probability: number | null;
  confidence: number;
  expected_elapsed_ms: number | null;
  elapsed_z_score: number | null;
  is_new_transition: boolean;
  is_rare_transition: boolean;
};
type Pending = {
  stream_id: string;
  state: string;
  started_at?: number;
  waiting_ms: number;
  expected_next_event: string;
  prior_count: number;
  typical_wait_ms: number;
  overdue_after_ms: number;
  is_overdue: boolean;
};
type SequenceMatch = { pattern: string; similarity: number; confidence: number; expected: string[]; observed: string[] };
type PersonalSequence = {
  stream_id: string;
  observed: string[];
  closest_historical: string[];
  similarity: number | null;
  prior_windows: number;
  confidence: number;
  started_at: number;
  is_unusual: boolean;
};
type AnomalyReason = {
  kind: string;
  subject: string;
  observed: number | null;
  baseline: number | null;
  score: number | null;
  score_label: string;
  direction: string;
  started_at: number | null;
  explanation: string;
};
type Assessment = {
  snapshot_risk_score?: number;
  synthetic_replay?: boolean;
  familiarity: number;
  event_familiarity: number;
  is_confident: boolean;
  is_unusual: boolean;
  anomaly_score: number | null;
  metric_deviations?: Record<string, MetricDeviation>;
  transition: Transition | null;
  transitions?: Transition[];
  pending_states?: Pending[];
  sequence_matches?: SequenceMatch[];
  personal_sequence_deviations?: PersonalSequence[];
  anomaly_reasons?: AnomalyReason[];
};
type HistoryPoint = { observedOn: string; settingsVersion?: number; historicalMinDays?: number | null; assessment: Assessment; features: Record<string, number> };
type SequencePeer = { accountId: number; similarity: number; sequence: string[] };
type DiscoveredPattern = { sequence: string[]; support: number; supportRatio: number; clientIds: number[] };
type TransitionEntry = Transition & { observedOn: string };
type Period = 7 | 30 | 90;

const metricNames: Record<string, string> = {
  services_count: 'Количество услуг', monthly_spend: 'Расходы в месяц',
  spendable_balance: 'Доступный баланс', payment_coverage: 'Покрытие платежей',
  upcoming_payments: 'Ближайшие платежи', no_auto_prolong_share: 'Без автопродления',
  prolong_wait_share: 'В prolong_wait', failed_prolong_share: 'Неуспешные продления',
  expiring_revenue_share: 'Истекающая выручка',
};
const featureValue = (key: string, value: number | null | undefined) => {
  if (value == null) return '—';
  if (key.endsWith('_share')) return (value * 100).toFixed(1) + '%';
  if (key === 'payment_coverage') return value.toFixed(2) + '×';
  return value.toLocaleString('ru-RU', { maximumFractionDigits: 2 });
};
const sigma = (value: number | null | undefined) => value == null ? '—' : value.toFixed(1) + 'σ';
const percent = (value: number | null | undefined) => value == null ? '—' : Math.round(value * 100) + '%';
const duration = (value: number) => value >= 86_400_000
  ? Math.floor(value / 86_400_000) + ' д ' + Math.round(value % 86_400_000 / 3_600_000) + ' ч'
  : (value / 3_600_000).toFixed(1) + ' ч';
const eventDate = (value: number) => new Date(value).toLocaleString('ru-RU', { dateStyle: 'medium', timeStyle: 'short' });
const calendarDate = (value: string) => new Date(value + 'T00:00:00').toLocaleDateString('ru-RU', { day: 'numeric', month: 'short', year: 'numeric' });
const compactDate = (value: string) => new Date(value + 'T00:00:00').toLocaleDateString('ru-RU', { day: 'numeric', month: 'short' });
const eventName = (value: string) => ({
  active: 'Активна', suspended: 'Приостановлена', prolong_wait: 'Ожидает оплаты', deleted: 'Удалена',
  blocked: 'Заблокирована', payment: 'Оплата', top_up: 'Пополнение', auto_renew_disabled: 'Автопродление выключено',
} as Record<string, string>)[value] ?? value.replaceAll('_', ' ');
const transitionList = (point: HistoryPoint): Transition[] => point.assessment.transitions ?? (point.assessment.transition ? [point.assessment.transition] : []);

export default function Historical() {
  const { data, settings } = useAnalyzer();
  const [params, setParams] = useSearchParams();
  const [points, setPoints] = useState<HistoryPoint[]>([]);
  const [sequencePeers, setSequencePeers] = useState<SequencePeer[]>([]);
  const [patterns, setPatterns] = useState<DiscoveredPattern[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [patternError, setPatternError] = useState('');
  const [period, setPeriod] = useState<Period>(30);
  const [selectedDate, setSelectedDate] = useState('');
  const account = data?.accounts.find(a => a.id === params.get('client')) ?? data?.accounts[0];

  useEffect(() => {
    const controller = new AbortController();
    fetch('/api/client/historical/patterns', { signal: controller.signal })
      .then(async response => {
        if (!response.ok) throw new Error('API: ' + response.status);
        return response.json() as Promise<DiscoveredPattern[]>;
      })
      .then(setPatterns)
      .catch(reason => { if (!controller.signal.aborted) setPatternError(String(reason)); });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    if (!account) { setPoints([]); setSequencePeers([]); return; }
    const controller = new AbortController();
    setLoading(true); setError(''); setSelectedDate('');
    fetch('/api/client/historical/' + account.id, { signal: controller.signal })
      .then(async response => {
        if (!response.ok) throw new Error('API: ' + response.status);
        return response.json() as Promise<HistoryPoint[]>;
      })
      .then(rows => setPoints(rows.sort((a, b) => a.observedOn.localeCompare(b.observedOn))))
      .catch(reason => { if (!controller.signal.aborted) setError(String(reason)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    fetch('/api/client/historical/' + account.id + '/sequence-peers', { signal: controller.signal })
      .then(async response => {
        if (!response.ok) throw new Error('API: ' + response.status);
        return response.json() as Promise<SequencePeer[]>;
      })
      .then(setSequencePeers)
      .catch(() => { if (!controller.signal.aborted) setSequencePeers([]); });
    return () => controller.abort();
  }, [account?.id, data?.asOf]);

  const latest = points.at(-1);
  const chartPoints = useMemo(() => {
    const latestTime = latest ? Date.parse(latest.observedOn) : 0;
    return points.filter(point => Date.parse(point.observedOn) >= latestTime - period * 86_400_000)
      .map(point => {
        const metricScores = Object.values(point.assessment.metric_deviations ?? {}).map(item => item.score).filter((value): value is number => value != null);
        const transitionScores = transitionList(point).map(item => item.elapsed_z_score).filter((value): value is number => value != null);
        return { date: point.observedOn, anomaly: Math.max(0, ...metricScores, ...transitionScores), riskScore: point.assessment.synthetic_replay ? point.assessment.snapshot_risk_score : undefined };
      });
  }, [points, latest, period]);
  const selectedPoint = chartPoints.length
    ? points.find(point => point.observedOn === selectedDate && chartPoints.some(row => row.date === point.observedOn)) ?? points.find(point => point.observedOn === chartPoints.at(-1)?.date)
    : undefined;
  const selectedChartScore = chartPoints.find(row => row.date === selectedPoint?.observedOn)?.anomaly ?? null;
  const assessment = latest?.assessment;
  const selectedAssessment = selectedPoint?.assessment;
  const deviations = Object.entries(assessment?.metric_deviations ?? {});
  const transitions = assessment?.transitions ?? (assessment?.transition ? [assessment.transition] : []);
  const pendingStates = assessment?.pending_states ?? [];
  const sequenceMatches = assessment?.sequence_matches ?? [];
  const personalSequences = assessment?.personal_sequence_deviations ?? [];
  const anomalyReasons = assessment?.anomaly_reasons ?? [];
  const selectedReasons = selectedAssessment?.anomaly_reasons ?? [];
  const historyDays = points.length > 1 ? Math.floor((Date.parse(points.at(-1)!.observedOn) - Date.parse(points[0].observedOn)) / 86_400_000) + 1 : points.length ? 1 : 0;
  const observationCounts = deviations.map(([, value]) => value.observations).filter(value => value > 0);
  const minObservations = observationCounts.length ? Math.min(...observationCounts) : 0;
  const maxObservations = observationCounts.length ? Math.max(...observationCounts) : 0;
  const observationLabel = !observationCounts.length ? 'Нет наблюдений' : minObservations === maxObservations ? minObservations + ' на признак' : minObservations + '–' + maxObservations + ' на признак';
  const accountPatterns = patterns.filter(pattern => pattern.clientIds.includes(Number(account?.id)));

  const transitionHistory = useMemo(() => points.flatMap(point => transitionList(point).map(item => ({ ...item, observedOn: point.observedOn }))), [points]);
  const transitionChains = useMemo(() => {
    const grouped = new Map<string, TransitionEntry[]>();
    transitionHistory.forEach(item => grouped.set(item.stream_id, [...(grouped.get(item.stream_id) ?? []), item]));
    return Array.from(grouped.entries()).map(([streamId, entries]) => {
      const states = entries.length ? [entries[0].from] : [];
      entries.forEach(item => { if (states.at(-1) !== item.to) states.push(item.to); });
      return { streamId, entries, states, latest: entries.at(-1)! };
    }).sort((a, b) => b.latest.observedOn.localeCompare(a.latest.observedOn)).slice(0, 8);
  }, [transitionHistory]);

  const anomalyStart = (feature: string) => {
    if (!latest || !assessment?.is_confident) return null;
    let index = points.length - 1;
    const anomalous = (point: HistoryPoint) => point.assessment.is_confident && (point.assessment.metric_deviations?.[feature]?.score ?? 0) >= 3;
    if (!anomalous(latest)) return null;
    while (index > 0 && anomalous(points[index - 1])) {
      if (Date.parse(points[index].observedOn) - Date.parse(points[index - 1].observedOn) > 2 * 86_400_000) break;
      index -= 1;
    }
    return points[index].observedOn;
  };

  const mainChanges = (assessment?.familiarity ?? 0) >= 1 ? deviations.filter(([, value]) => (value.score ?? 0) >= 3)
    .sort((a, b) => (b[1].score ?? 0) - (a[1].score ?? 0)).slice(0, 4) : [];

  const checks = anomalyReasons.map((reason, index) => {
    if (reason.kind === 'feature') {
      const current = latest?.features[reason.subject];
      const featureDeviation = latest?.assessment.metric_deviations?.[reason.subject];
      const baseline = Math.abs(featureDeviation?.short_term_z_score ?? 0) >= Math.abs(featureDeviation?.long_term_z_score ?? 0) ? featureDeviation?.short_term_mean ?? featureDeviation?.long_term_mean : featureDeviation?.long_term_mean ?? featureDeviation?.short_term_mean;
      return { key: reason.kind + reason.subject + index, title: 'Проверить: ' + (metricNames[reason.subject] ?? reason.subject), detail: 'Сейчас ' + featureValue(reason.subject, current) + ' · личная норма ' + featureValue(reason.subject, baseline) + ' · ' + sigma(reason.score) + ' от нормы.', href: '/snapshot?client=' + account?.id, link: 'Открыть услуги и платежи' };
    }
    if (reason.kind === 'pending_state') {
      const pending = pendingStates.find(item => reason.subject.includes(item.state) && reason.subject.includes(item.expected_next_event));
      return { key: reason.kind + reason.subject + index, title: 'Проверить ожидание услуги' + (pending ? ' ' + pending.stream_id : ''), detail: (pending ? eventName(pending.state) + ' уже ' + duration(pending.waiting_ms) + ' · обычно ' + duration(pending.typical_wait_ms) : reason.subject) + '. Это сигнал сверить текущее состояние услуги.', href: '#history-transitions', link: 'Переходы услуг' };
    }
    if (reason.kind === 'transition') {
      const transition = transitionHistory.slice().reverse().find(item => (item.from + ' → ' + item.to) === reason.subject);
      return { key: reason.kind + reason.subject + index, title: 'Проверить переход' + (transition ? ' услуги ' + transition.stream_id : ''), detail: reason.subject + (transition ? ' · время ' + duration(transition.elapsed_ms) + (transition.expected_elapsed_ms == null ? '' : ' · обычно ' + duration(transition.expected_elapsed_ms)) : '') + '. Необычность перехода не объясняет его причину.', href: '#history-transitions', link: 'История переходов' };
    }
    return { key: reason.kind + reason.subject + index, title: 'Проверить последовательность услуги ' + reason.subject, detail: 'Текущая цепочка отличается от прежних окон. Сверьте события этой услуги с журналом.', href: '#history-advanced', link: 'Анализ последовательностей' };
  }).slice(0, 4);

  if (!data || !settings) return null;
  if (!account) return <Empty icon={<History size={30} />} title="Нет клиентов">Добавьте аккаунты через API для анализа истории.</Empty>;

  return <div className="historical-page">
    <div className="page-heading historical-heading">
      <div><div className="eyebrow">ПОВЕДЕНЧЕСКАЯ АНАЛИТИКА</div><h1>Historical Model</h1><p>Что изменилось у клиента, насколько это необычно и что стоит проверить.</p></div>
      <AccountPicker accounts={data.accounts} value={account.id} onChange={value => setParams(current => { const next = new URLSearchParams(current); next.set('client', value); return next; })} />
    </div>

    <section className="historical-summary" aria-label="Сводка истории клиента">
      <article className={'historical-summary-card historical-status ' + (assessment?.is_confident ? assessment.is_unusual ? 'is-unusual' : 'is-normal' : 'is-limited')}>
        <small>Статус поведения</small><strong>{!assessment ? 'Истории пока нет' : !assessment.is_confident ? 'Недостаточно истории для вывода' : assessment.is_unusual ? 'Есть отклонения' : 'Выраженных отклонений нет'}</strong>
        <span>{assessment?.is_confident ? 'Статус рассчитан по личной истории клиента.' : 'Оценка числовой и событийной истории показана отдельно.'}</span>
      </article>
      <article className="historical-summary-card"><small>Числовая история</small><strong>{percent(assessment?.familiarity)}</strong><span>{observationLabel} · снимков: {points.length}</span><div className="historical-meter"><i style={{ width: percent(assessment?.familiarity) }} /></div></article>
      <article className="historical-summary-card"><small>Уверенность событийной модели</small><strong>{percent(assessment?.event_familiarity)}</strong><span>{transitionHistory.length} переходов зафиксировано в снимках</span><div className="historical-meter"><i style={{ width: percent(assessment?.event_familiarity) }} /></div></article>
      <article className="historical-summary-card"><small>Снимки профиля</small><strong>{points.length}</strong><span>{latest ? 'Последний: ' + calendarDate(latest.observedOn) : 'Ожидается первый снимок'}</span></article>
      <article className="historical-summary-card"><small>Длительность истории</small><strong>{historyDays} дн.</strong><span>{points.length > 1 ? calendarDate(points[0].observedOn) + ' — ' + calendarDate(points.at(-1)!.observedOn) : 'Накопление личной нормы'}</span></article>
    </section>

    <Panel title="Историческая аномальность" subtitle="Числовые отклонения от личной нормы · выберите точку, чтобы увидеть сигналы на эту дату" className="historical-chart-panel" action={<div className="historical-period-switch" role="group" aria-label="Период графика">{([7, 30, 90] as Period[]).map(value => <button type="button" key={value} aria-pressed={period === value} className={period === value ? 'active' : ''} onClick={() => setPeriod(value)}>{value} дней</button>)}</div>}>
      {loading ? <p className="historical-empty-state">Загружаем историю клиента…</p> : error ? <p className="historical-empty-state is-error" role="alert">История не загрузилась: {error}</p> : chartPoints.length === 0 ? <p className="historical-empty-state">Снимков пока нет. График появится после накопления первой истории.</p> : <>
        <div className="historical-chart"><ResponsiveContainer><LineChart data={chartPoints} margin={{ top: 14, right: 22, bottom: 6, left: 0 }} onClick={state => { const index = (state as unknown as { activeTooltipIndex?: number })?.activeTooltipIndex; if (typeof index === 'number' && chartPoints[index]) setSelectedDate(chartPoints[index].date); }}>
          <CartesianGrid stroke="#a9dfff18" vertical={false} />
          <XAxis dataKey="date" tickFormatter={(value: string) => compactDate(value)} minTickGap={24} tick={{ fill: '#8eabc0', fontSize: 10 }} axisLine={{ stroke: '#a9dfff22' }} tickLine={false} />
          <YAxis domain={[0, (dataMax: number) => Math.max(dataMax, 3.5)]} tick={{ fill: '#8eabc0', fontSize: 10 }} axisLine={false} tickLine={false} width={38} />
          <Tooltip labelFormatter={value => { const point = points.find(item => item.observedOn === String(value)); return `${calendarDate(String(value))} · ${point?.settingsVersion ? `конфигурация v${point.settingsVersion}` : 'старая конфигурация'}`; }} formatter={value => [Number(value).toFixed(1) + 'σ', 'Отклонение числового профиля']} contentStyle={{ background: '#10283d', border: '1px solid #6ab9e533', borderRadius: 9, color: '#e4f4ff' }} />
          <ReferenceLine y={3} stroke="#ff7778" strokeDasharray="5 5" label={{ value: 'Порог 3σ', position: 'insideTopRight', fill: '#ff8b8d', fontSize: 10 }} />
          {selectedPoint && <ReferenceLine x={selectedPoint.observedOn} stroke="#7bc8f0" strokeDasharray="3 4" />}
          <Line type="monotone" dataKey="anomaly" name="Аномальность" stroke="#67cefb" strokeWidth={2.4} dot={{ r: 3, fill: '#10283d', stroke: '#67cefb', strokeWidth: 2, cursor: 'pointer' }} activeDot={{ r: 6, fill: '#c4eeff', stroke: '#42b9f0', strokeWidth: 2 }} />
        </LineChart></ResponsiveContainer></div>
        {chartPoints.some(point => point.riskScore != null) && <div className="historical-replay-strip"><small>Сценарный Risk Score · синтетические данные</small><div className="historical-replay-bars">{chartPoints.map(point => <button key={point.date} type="button" title={`${calendarDate(point.date)}: ${point.riskScore ?? 'нет оценки'} / 100`} aria-label={`${calendarDate(point.date)}: сценарный Risk Score ${point.riskScore ?? 'нет оценки'} из 100`} className={point.date === selectedPoint?.observedOn ? 'selected' : ''} style={{height: `${Math.max(6, point.riskScore ?? 0)}%`}} onClick={() => setSelectedDate(point.date)} />)}</div></div>}
        {selectedPoint && <div className="historical-selected-point"><div className="historical-selected-heading"><div><small>Выбранная дата</small><strong>{calendarDate(selectedPoint.observedOn)}</strong><small>Конфигурация {selectedPoint.settingsVersion ? `v${selectedPoint.settingsVersion}` : 'до версионирования'} · минимум истории: {selectedPoint.historicalMinDays ? `${selectedPoint.historicalMinDays} дней` : 'не записан'}</small></div><b className={selectedAssessment?.is_unusual ? 'is-unusual' : ''}>Числовое отклонение {sigma(selectedChartScore)} · {selectedAssessment?.is_unusual ? 'есть сигналы' : 'без выраженного отклонения'}</b></div>
          {selectedAssessment?.synthetic_replay && <p>Сценарный Risk Score: <strong>{selectedAssessment.snapshot_risk_score ?? '—'} / 100</strong>. Рассчитан по виртуальным признакам и коэффициентам конфигурации v{selectedPoint.settingsVersion}; это тестовый результат.</p>}
          {selectedReasons.length ? <div className="historical-point-reasons">{selectedReasons.slice(0, 4).map((reason, index) => <div key={reason.kind + reason.subject + index}><strong>{metricNames[reason.subject] ?? reason.subject}</strong><span>{reason.kind === 'feature' ? featureValue(reason.subject, reason.observed) + ' при личной норме ' + featureValue(reason.subject, reason.baseline) : reason.kind === 'pending_state' ? 'Ожидание ' + (reason.observed == null ? '—' : duration(reason.observed)) + ' · обычно ' + (reason.baseline == null ? '—' : duration(reason.baseline)) : reason.subject}</span></div>)}</div> : <p>Для выбранной даты модель не отметила отдельных сигналов.</p>}
          <small>Отклонение и редкость помогают выбрать, что проверить; сами по себе они не доказывают причину поведения.</small>
        </div>}
      </>}
    </Panel>

    <div className="historical-main-grid">
      <Panel title="Главные изменения поведения" subtitle="Текущее значение, личная норма, отклонение и начало" className="historical-changes">
        {mainChanges.length ? <div className="historical-change-list">{mainChanges.map(([key, item]) => {
          const current = latest?.features[key];
          const baseline = Math.abs(item.short_term_z_score ?? 0) >= Math.abs(item.long_term_z_score ?? 0) ? item.short_term_mean ?? item.long_term_mean : item.long_term_mean ?? item.short_term_mean;
          const start = anomalyStart(key);
          return <article className="historical-change-card" key={key}><div className="historical-change-title"><strong>{metricNames[key] ?? key}</strong><b>{sigma(item.score)}</b></div><div className="historical-change-values"><div><small>Сейчас</small><strong>{featureValue(key, current)}</strong></div><div><small>Личная норма</small><strong>{featureValue(key, baseline)}</strong></div></div><p>{item.observations} наблюдений · начало: {start ? calendarDate(start) : 'определяется по снимкам'}</p></article>;
        })}</div> : <p className="historical-compact-note">{(assessment?.familiarity ?? 0) < 1 ? 'Числовая история пока недостаточно полна для уверенной интерпретации.' : 'Числовые показатели не превышают порог 3σ.'}</p>}
      </Panel>

      <Panel title="Что стоит проверить" subtitle="Проверочные действия по сигналам, без утверждений о причинах ухода" className="historical-actions">
        {checks.length ? <div className="historical-check-list">{checks.map(item => <article className="historical-check-card" key={item.key}><CircleAlert size={17} /><div><strong>{item.title}</strong><p>{item.detail}</p><Link to={item.href}>{item.link}<ArrowUpRight size={13} /></Link></div></article>)}</div> : <p className="historical-compact-note">Сейчас нет новых сигналов, требующих отдельной проверки.</p>}
        <Link className="historical-snapshot-link" to={'/snapshot?client=' + account.id}>Открыть текущий профиль клиента <ArrowUpRight size={14} /></Link>
      </Panel>
    </div>

    {(transitionChains.length > 0 || pendingStates.some(item => item.is_overdue)) && <div id="history-transitions"><Panel title="История переходов услуг" subtitle="Цепочки состояний и время шага сравниваются с привычной длительностью клиента" className="historical-transitions">
      {transitionChains.length > 0 && <div className="historical-transition-list">{transitionChains.map(chain => <article className="historical-transition-card" key={chain.streamId}><div className="historical-transition-service"><small>Услуга · {calendarDate(chain.latest.observedOn)}</small><strong>{chain.streamId}</strong></div><div className="historical-transition-chain"><small>Последовательность</small><strong>{chain.states.map(eventName).join(' → ')}</strong></div><div className="historical-transition-duration"><small>Последний шаг</small><strong>{duration(chain.latest.elapsed_ms)}</strong><span>{chain.latest.expected_elapsed_ms == null ? 'Личная норма ещё не сформирована' : 'обычно ' + duration(chain.latest.expected_elapsed_ms)}</span></div><span className={chain.latest.is_new_transition || chain.latest.is_rare_transition ? 'historical-transition-tag is-unusual' : 'historical-transition-tag'}>{chain.latest.is_new_transition ? 'Новый' : chain.latest.is_rare_transition ? 'Редкий' : 'Наблюдался ' + chain.latest.prior_count + ' раз'}</span></article>)}</div>}
      {pendingStates.some(item => item.is_overdue) && <div className="historical-pending-note"><Clock3 size={15} /><span>{pendingStates.filter(item => item.is_overdue).map(item => item.stream_id + ': ' + eventName(item.state) + ' ' + duration(item.waiting_ms) + ', обычно ' + duration(item.typical_wait_ms)).join(' · ')}</span></div>}
    </Panel></div>}

    <details className="historical-advanced" id="history-advanced">
      <summary><span><strong>Анализ последовательностей и подробная статистика</strong><small>LCS, похожие клиенты, личные цепочки и профиль числовых отклонений</small></span><ChevronDown size={18} /></summary>
      <div className="historical-advanced-content">
        {personalSequences.length > 0 && <section className="historical-analysis-section"><h3>Личные последовательности</h3>{personalSequences.map(item => <article className="historical-sequence-card" key={item.stream_id}><strong>{item.stream_id}</strong><span>{item.is_unusual ? 'Новая для клиента' : 'Похожа на прежние'} · сходство {item.similarity == null ? '—' : percent(item.similarity)} · уверенность {percent(item.confidence)}</span><p>Сейчас: {item.observed.map(eventName).join(' → ')}</p><p>Прежде: {item.closest_historical.length ? item.closest_historical.map(eventName).join(' → ') : 'Сопоставимого окна ещё нет'}</p><small>Начало последовательности: {eventDate(item.started_at)} · прошлых окон: {item.prior_windows}</small></article>)}</section>}
        {sequenceMatches.length > 0 && <section className="historical-analysis-section"><h3>Сходство с заданными сценариями · LCS</h3>{sequenceMatches.map(match => <article className="historical-sequence-card" key={match.pattern}><strong>{match.pattern} · сходство {percent(match.similarity)}</strong><p>Шаблон: {match.expected.map(eventName).join(' → ')}</p><p>Наблюдалось: {match.observed.map(eventName).join(' → ')}</p></article>)}</section>}
        {accountPatterns.length > 0 && <section className="historical-analysis-section"><h3>Паттерны, найденные у этого клиента и других</h3><div className="table-scroll"><table><thead><tr><th>Последовательность</th><th>Клиенты</th><th>Доля базы с историей</th></tr></thead><tbody>{accountPatterns.map((pattern, index) => <tr key={pattern.sequence.join('|') + index}><td>{pattern.sequence.map(eventName).join(' → ')}</td><td>{pattern.support}</td><td>{percent(pattern.supportRatio)}</td></tr>)}</tbody></table></div></section>}
        {patternError && <p className="historical-compact-note">Общие найденные паттерны временно недоступны: {patternError}</p>}
        {sequencePeers.length > 0 && <section className="historical-analysis-section"><h3>Похожие клиенты по последовательности</h3><div className="historical-peer-list">{sequencePeers.map(peer => <Link to={'/historical?client=' + peer.accountId} key={peer.accountId}><strong>Клиент {peer.accountId}</strong><span>сходство {percent(peer.similarity)}</span><small>{peer.sequence.map(eventName).join(' → ')}</small><ArrowUpRight size={14} /></Link>)}</div></section>}
        {deviations.length > 0 && <section className="historical-analysis-section"><h3>Короткая и длинная нормы · Welford и Concept Drift</h3><div className="table-scroll"><table><thead><tr><th>Показатель</th><th>Отклонение</th><th>Короткая норма</th><th>Длинная норма</th><th>Сдвиг нормы</th><th>Наблюдений</th></tr></thead><tbody>{deviations.map(([key, value]) => <tr key={key}><td>{metricNames[key] ?? key}</td><td>{sigma(value.score)}</td><td>{featureValue(key, value.short_term_mean)}</td><td>{featureValue(key, value.long_term_mean)}</td><td>{value.baseline_shift == null ? '—' : (value.baseline_shift > 0 ? '+' : '') + featureValue(key, value.baseline_shift)}</td><td>{value.observations}</td></tr>)}</tbody></table></div><p className="historical-compact-note">Числа описывают отличие от личной нормы. Они помогают исследовать динамику, но не устанавливают причину изменения.</p></section>}
        {!personalSequences.length && !sequenceMatches.length && !accountPatterns.length && !sequencePeers.length && !deviations.length && <p className="historical-compact-note">Истории пока недостаточно для анализа последовательностей и статистического профиля.</p>}
      </div>
    </details>
  </div>;
}
