import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { CartesianGrid, Line, LineChart, ResponsiveContainer, Tooltip, XAxis, YAxis } from 'recharts';
import { ArrowUpRight, History } from 'lucide-react';
import { useAnalyzer } from '../hooks/useAnalyzer';
import { Empty, Panel } from '../components/common';
import { Button } from '../components/ui/button';
import { GlassSelect } from '../components/ui/GlassSelect';

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
type HistoryPoint = { observedOn: string; assessment: Assessment; features: Record<string, number> };
type SequencePeer = { accountId: number; similarity: number; sequence: string[] };
type DiscoveredPattern = { sequence: string[]; support: number; supportRatio: number; clientIds: number[] };

const metricNames: Record<string, string> = {
  services_count: 'Количество услуг', monthly_spend: 'Расходы в месяц',
  spendable_balance: 'Доступный баланс', payment_coverage: 'Покрытие платежей',
  upcoming_payments: 'Ближайшие платежи', no_auto_prolong_share: 'Без автопродления',
  prolong_wait_share: 'В prolong_wait', failed_prolong_share: 'Неуспешные продления',
  expiring_revenue_share: 'Истекающая выручка',
};
const score = (value: number | null | undefined) => value == null ? '—' : `${value.toFixed(2)}σ`;
const hours = (value: number) => `${(value / 3_600_000).toFixed(1)} ч`;
const moment = (value: number) => new Date(value).toLocaleString('ru-RU');
const featureValue = (key: string, value: number | null | undefined) => {
  if (value == null) return '—';
  if (key.endsWith('_share')) return `${(value * 100).toFixed(1)}%`;
  if (key === 'payment_coverage') return `${value.toFixed(2)}×`;
  return value.toFixed(2);
};

export default function Historical() {
  const { data, settings } = useAnalyzer();
  const [params, setParams] = useSearchParams();
  const [points, setPoints] = useState<HistoryPoint[]>([]);
  const [sequencePeers, setSequencePeers] = useState<SequencePeer[]>([]);
  const [patterns, setPatterns] = useState<DiscoveredPattern[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [patternError, setPatternError] = useState('');
  const account = data?.accounts.find(a => a.id === params.get('client')) ?? data?.accounts[0];

  useEffect(() => {
    const controller = new AbortController();
    fetch('/api/client/historical/patterns', { signal: controller.signal })
      .then(async response => {
        if (!response.ok) throw new Error(`API: ${response.status}`);
        return response.json() as Promise<DiscoveredPattern[]>;
      })
      .then(setPatterns)
      .catch(reason => { if (!controller.signal.aborted) setPatternError(String(reason)); });
    return () => controller.abort();
  }, []);

  useEffect(() => {
    if (!account) { setPoints([]); setSequencePeers([]); return; }
    const controller = new AbortController();
    setLoading(true); setError('');
    fetch(`/api/client/historical/${account.id}`, { signal: controller.signal })
      .then(async response => {
        if (!response.ok) throw new Error(`API: ${response.status}`);
        return response.json() as Promise<HistoryPoint[]>;
      })
      .then(rows => setPoints(rows.sort((a, b) => a.observedOn.localeCompare(b.observedOn))))
      .catch(reason => { if (!controller.signal.aborted) setError(String(reason)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    fetch(`/api/client/historical/${account.id}/sequence-peers`, { signal: controller.signal })
      .then(async response => {
        if (!response.ok) throw new Error(`API: ${response.status}`);
        return response.json() as Promise<SequencePeer[]>;
      })
      .then(setSequencePeers)
      .catch(() => { if (!controller.signal.aborted) setSequencePeers([]); });
    return () => controller.abort();
  }, [account?.id]);

  const chart = useMemo(() => points.map(point => ({
    date: point.observedOn,
    anomaly: point.assessment.anomaly_score ?? 0,
  })), [points]);
  if (!data || !settings) return null;
  const latest = points.at(-1);
  const assessment = latest?.assessment;
  const deviations = Object.entries(assessment?.metric_deviations ?? {});
  const transitions = assessment?.transitions ?? [];
  const pendingStates = assessment?.pending_states ?? [];
  const sequenceMatches = assessment?.sequence_matches ?? [];
  const personalSequences = assessment?.personal_sequence_deviations ?? [];
  const anomalyReasons = assessment?.anomaly_reasons ?? [];

  const anomalyStart = (feature: string) => {
    if (!latest || !assessment?.is_confident) return null;
    let index = points.length - 1;
    const isAnomalous = (point: HistoryPoint) => point.assessment.is_confident
      && (point.assessment.metric_deviations?.[feature]?.score ?? 0) >= 3;
    if (!isAnomalous(latest)) return null;
    while (index > 0 && isAnomalous(points[index - 1])) {
      const gap = Date.parse(points[index].observedOn) - Date.parse(points[index - 1].observedOn);
      if (gap > 2 * 86_400_000) break;
      index -= 1;
    }
    return points[index].observedOn;
  };

  return <>
    <div className="page-heading">
      <div><div className="eyebrow">ПОВЕДЕНЧЕСКАЯ АНАЛИТИКА</div><h1>Historical Model</h1><p>Личный профиль, переходы услуг и изменения относительно привычного поведения.</p></div>
      <label className="client-select">Клиент<GlassSelect value={account?.id ?? ''} onChange={value => setParams({ client: value })} options={data.accounts.map(item => ({value:item.id,label:`${item.name} · ${item.id}`}))}/></label>
    </div>

    <Panel title="Исторический анализ" subtitle={account?.email ?? 'Нет аккаунтов'} action={<span className={`badge ${assessment?.is_confident ? 'low' : 'medium'}`}>
      {assessment?.is_confident ? 'Истории достаточно' : 'Сначала Snapshot Model'}
    </span>}>
      {loading ? <p>Загружаем историю…</p> : error ? <div className="error-box" role="alert">История не загрузилась: {error}</div> : points.length === 0
        ? <Empty icon={<History size={30} />} title="Снимков пока нет">Ежедневный сбор сохраняет историю в PostgreSQL. Граф переходов пополняется событиями из operation_logs.</Empty>
        : <>
          <div className="history-progress"><span>Снимки</span><strong>{points.length} / {settings.historicalMinDays} дней</strong><div style={{ width: `${Math.min(100, points.length / settings.historicalMinDays * 100)}%` }} /></div>
          <p className="body-note">Последний снимок: {latest?.observedOn}. Знакомство с профилем: {Math.round((assessment?.familiarity ?? 0) * 100)}%; событийная история: {Math.round((assessment?.event_familiarity ?? 0) * 100)}%. {assessment?.is_confident ? (assessment.is_unusual ? 'Найдено отклонение от личной нормы.' : 'Выраженного отклонения нет.') : 'Истории пока недостаточно для уверенных выводов; используется Snapshot Model.'}</p>
          <div style={{ height: 260, width: '100%' }}><ResponsiveContainer><LineChart data={chart}><CartesianGrid stroke="#a9dfff1b" vertical={false} /><XAxis dataKey="date" tick={{ fill: '#9bb7c9', fontSize: 11 }} /><YAxis tick={{ fill: '#9bb7c9', fontSize: 11 }} /><Tooltip contentStyle={{ background: '#142d42', color: '#e9f7ff' }} /><Line type="monotone" dataKey="anomaly" name="Отклонение от профиля" stroke="#9edcff" strokeWidth={2} dot={{ r: 3 }} /></LineChart></ResponsiveContainer></div>
        </>}
      {account && <div className="empty-action"><Button asChild variant="default"><Link to={`/snapshot?client=${account.id}`}>Текущий анализ <ArrowUpRight size={15} /></Link></Button></div>}
    </Panel>

    <Panel title="Сценарии, найденные автоматически" subtitle="Похожие последовательности сгруппированы по LCS; порог поддержки — два разных клиента">
      {patternError ? <p className="body-note">Не удалось загрузить найденные сценарии: {patternError}</p> : patterns.length === 0
        ? <p className="body-note">Повторяющихся последовательностей пока недостаточно. Они появятся, когда похожие цепочки событий встретятся у нескольких клиентов.</p>
        : <div className="table-scroll"><table><thead><tr><th>Последовательность</th><th>Клиенты</th><th>Доля клиентов с историей</th></tr></thead><tbody>
          {patterns.map((pattern, index) => <tr key={`${index}-${pattern.sequence.join('|')}`}><td>{pattern.sequence.join(' → ')}</td><td>{pattern.support} · {pattern.clientIds.slice(0, 8).map(id => <Link className="text-link" key={id} to={`/historical?client=${id}`}>{id} </Link>)}</td><td>{Math.round(pattern.supportRatio * 100)}%</td></tr>)}
        </tbody></table></div>}
    </Panel>

    {assessment && <>
      <Panel title="Отклонения от личной нормы" subtitle="Последовательности клиента сравниваются с его предыдущими цепочками событий">
        {personalSequences.length ? personalSequences.map(item => <div className="body-note" key={item.stream_id}>
          <strong>{item.stream_id}</strong> · {item.is_unusual ? 'новая для клиента последовательность' : 'похожа на прежнее поведение'} · уверенность сравнения {Math.round(item.confidence * 100)}%<br />
          Сейчас: {item.observed.join(' → ')}<br />
          Ближайшая прежняя: {item.closest_historical.length ? item.closest_historical.join(' → ') : 'пока нет похожего окна'} · сходство {item.similarity == null ? '—' : `${Math.round(item.similarity * 100)}%`} · последнее событие {moment(item.started_at)}
        </div>) : <p className="body-note">Новых цепочек для сравнения пока нет. При достаточной истории модель сопоставит последние события услуги с её прежними последовательностями.</p>}
      </Panel>

      <Panel title="Почему обнаружено отклонение" subtitle="Показываются изменившиеся признаки, переходы, личная необычность и начало отклонения">
        {anomalyReasons.length ? anomalyReasons.map((reason, index) => {
          const start = reason.kind === 'feature' ? anomalyStart(reason.subject) : null;
          const detail = reason.kind === 'feature'
            ? `Текущее: ${featureValue(reason.subject, latest?.features[reason.subject])} · личная норма: ${featureValue(reason.subject, reason.baseline)}`
            : reason.kind === 'personal_sequence'
              ? `Последовательность: ${reason.explanation}`
              : `Ожидание: ${reason.observed == null ? '—' : hours(reason.observed)} · обычно: ${reason.baseline == null ? '—' : hours(reason.baseline)}`;
          const unusualness = reason.score == null ? '—'
            : reason.score_label === 'standard_deviations' ? score(reason.score)
              : reason.score_label === 'transition_rarity_0_to_1' ? `индекс редкости ${reason.score.toFixed(2)} / 1`
                : reason.score_label === 'multiple_of_typical_wait' ? `${reason.score.toFixed(1)}× обычного ожидания`
                  : `${Math.round(reason.score * 100)}% отличия от личной цепочки`;
          return <div className="body-note" key={`${reason.kind}-${reason.subject}-${index}`}>
            <strong>{metricNames[reason.subject] ?? reason.subject}</strong> · {reason.direction}<br />
            {detail} · необычность: {unusualness}<br />
            Начало: {start ?? (reason.started_at == null ? 'определяется по истории снимков' : moment(reason.started_at))}. {reason.kind === 'personal_sequence' ? `Сходство с прежней последовательностью: ${reason.observed == null ? '—' : `${Math.round(reason.observed * 100)}%`}.` : reason.explanation}
          </div>;
        }) : <p className="body-note">В последнем снимке объяснимых отклонений не отмечено.</p>}
      </Panel>

      <Panel title="Профиль поведения" subtitle="Welford хранит долгую статистику; затухающие нормы адаптируются к изменениям">
        <div className="table-scroll"><table><thead><tr><th>Признак</th><th>Текущая аномальность</th><th>Краткая норма</th><th>Длинная норма</th><th>Сдвиг нормы</th><th>Наблюдения</th></tr></thead><tbody>
          {deviations.map(([key, value]) => <tr key={key}><td>{metricNames[key] ?? key}</td><td>{score(value.score)}</td><td>{value.short_term_mean?.toFixed(2) ?? '—'}</td><td>{value.long_term_mean?.toFixed(2) ?? '—'}</td><td>{value.baseline_shift == null ? '—' : `${value.baseline_shift > 0 ? '+' : ''}${value.baseline_shift.toFixed(2)}`}</td><td>{value.observations}</td></tr>)}
        </tbody></table></div>
      </Panel>

      <Panel title="Переходы и ожидание продления" subtitle="События услуг взяты из журнала PostgreSQL; время сравнивается с личной историей">
        {transitions.length === 0 && pendingStates.length === 0 ? <p className="body-note">Новых переходов с предыдущего снимка нет. Граф пополняется новыми изменениями услуг и продлений.</p> : <>
          {pendingStates.length > 0 && <div className="table-scroll"><table><thead><tr><th>Услуга</th><th>Состояние</th><th>Началось</th><th>Ожидание</th><th>Обычно следующий шаг</th><th>Типичное время</th><th>Срок ожидания</th></tr></thead><tbody>
            {pendingStates.map(item => <tr key={item.stream_id}><td>{item.stream_id}</td><td>{item.state}</td><td>{item.started_at == null ? '—' : moment(item.started_at)}</td><td>{hours(item.waiting_ms)}</td><td>{item.expected_next_event}</td><td>{hours(item.typical_wait_ms)}</td><td>{item.is_overdue ? 'Просрочено' : hours(item.overdue_after_ms)}</td></tr>)}
          </tbody></table></div>}
          {transitions.length > 0 && <div className="table-scroll"><table><thead><tr><th>Услуга</th><th>Переход</th><th>Время</th><th>Повторений</th><th>Обычный интервал</th><th>Редкость</th></tr></thead><tbody>
            {transitions.slice(-12).reverse().map((item, index) => <tr key={`${item.stream_id}-${item.from}-${item.to}-${index}`}><td>{item.stream_id}</td><td>{item.from} → {item.to}</td><td>{hours(item.elapsed_ms)}</td><td>{item.prior_count}</td><td>{item.expected_elapsed_ms == null ? '—' : hours(item.expected_elapsed_ms)}</td><td>{item.is_new_transition ? 'Новый' : item.is_rare_transition ? 'Редкий' : 'Обычный'}</td></tr>)}
          </tbody></table></div>}
        </>}
      </Panel>

      <Panel title="Заданные базовые сценарии (LCS)" subtitle="Дополнительные шаблоны оплаты и сокращения услуг; автоматическое обнаружение показано выше">
        {sequenceMatches.length ? sequenceMatches.map(match => <div className="body-note" key={match.pattern}><strong>{match.pattern}</strong> · сходство {Math.round(match.similarity * 100)}%, уверенность профиля {Math.round(match.confidence * 100)}%<br />Шаблон: {match.expected.join(' → ')}<br />Наблюдалось: {match.observed.join(' → ')}</div>) : <p className="body-note">Совпадений с базовыми шаблонами нет.</p>}
      </Panel>

      <Panel title="Похожие клиенты по последовательности событий" subtitle="LCS сравнивает последние цепочки разных клиентов">
        {sequencePeers.length ? sequencePeers.map(peer => <div className="body-note" key={peer.accountId}><strong>Клиент {peer.accountId}</strong> · сходство {Math.round(peer.similarity * 100)}%<br />{peer.sequence.join(' → ')}</div>) : <p className="body-note">Похожие последовательности пока не найдены.</p>}
      </Panel>
    </>}
  </>;
}
