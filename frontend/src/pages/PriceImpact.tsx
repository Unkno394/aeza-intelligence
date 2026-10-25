import { useEffect, useMemo, useState } from 'react';
import { Link, useSearchParams } from 'react-router-dom';
import { ArrowUpRight, ChevronLeft, ChevronRight, CircleAlert, History, TrendingUp } from 'lucide-react';
import { Panel } from '../components/common';
import { GlassSelect } from '../components/ui/GlassSelect';
import { date, money } from '../lib/utils';
import { useAnalyzer } from '../hooks/useAnalyzer';
import { demoEvidenceReady, demoPriceRisk } from '../features/demoPriceRisk';
import { fetchPriceImpact, needsPriceReview, priceEvidence, priceIncreasePercent, priceOutcome, summarizePriceBands, summarizePriceImpact, type PriceImpactEvent } from '../features/priceImpact';
import './price-impact.css';

const pageSize = 20;
const outcomeLabels = {
  renewed: 'Продление состоялось', adverse: 'Негативное событие', unknown: 'Исход неизвестен',
  pending: 'Окно ещё не завершено', not_due: 'Продление не наступало',
} as const;
const percent = (value: number) => new Intl.NumberFormat('ru-RU', { maximumFractionDigits: 1 }).format(value) + '%';

export default function PriceImpact() {
  const [params, setParams] = useSearchParams();
  const { data, settings, risks } = useAnalyzer();
  const mode = params.get('mode') === 'real' ? 'real' : 'synthetic';
  const [events, setEvents] = useState<PriceImpactEvent[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState('');
  const [query, setQuery] = useState('');
  const [page, setPage] = useState(0);
  const clientId = params.get('client') ?? 'all';
  const planKey = params.get('plan');
  const realEvents = useMemo(() => events.filter(event =>
    !event.accountId.startsWith('-') && !event.accountEmail?.endsWith('@example.test')),
  [events]);
  const demoMode = mode === 'synthetic';
  const availableEvents = demoMode ? events : realEvents;
  const selectedEvents = useMemo(() => availableEvents
    .filter(event => !planKey || event.planKey === planKey), [availableEvents, planKey]);

  useEffect(() => {
    const controller = new AbortController();
    setLoading(true);
    setError('');
    const refresh = () => fetchPriceImpact(controller.signal, mode)
      .then(rows => { setEvents(rows); setError(''); })
      .catch(reason => { if (!controller.signal.aborted) setError(String(reason)); })
      .finally(() => { if (!controller.signal.aborted) setLoading(false); });
    void refresh();
    const timer = mode === 'synthetic' ? window.setInterval(() => { void refresh(); }, 15000) : undefined;
    return () => { controller.abort(); if (timer) window.clearInterval(timer); };
  }, [mode]);

  useEffect(() => {
    if (loading || error || clientId === 'all') return;
    if (availableEvents.some(event => String(event.accountId) === clientId)) return;
    setParams(current => {
      const next = new URLSearchParams(current);
      next.delete('client');
      return next;
    }, { replace: true });
    setPage(0);
  }, [clientId, error, loading, availableEvents, setParams]);

  const latestByService = useMemo(() => {
    const seen = new Set<string>();
    return selectedEvents.filter(event => {
      if (seen.has(event.serviceId)) return false;
      seen.add(event.serviceId);
      return true;
    });
  }, [selectedEvents]);
  const studyEvents = useMemo(() => {
    const seen = new Set<string>();
    return selectedEvents.filter(event => {
      if (!event.completeBaseline || !event.completeFollowup || !event.renewalDueInWindow || seen.has(event.serviceId)) return false;
      seen.add(event.serviceId);
      return true;
    });
  }, [selectedEvents]);
  const summary = useMemo(() => summarizePriceImpact(studyEvents), [studyEvents]);
  const bands = useMemo(() => summarizePriceBands(selectedEvents), [selectedEvents]);
  const evidence = useMemo(() => priceEvidence(selectedEvents), [selectedEvents]);
  const scenarioReady = useMemo(() => demoMode && demoEvidenceReady(events), [demoMode, events]);
  const scenarioScores = useMemo(() => {
    if (!demoMode || !settings || !risks) return new Map<string, number>();
    return new Map(events.map(event => [event.accountId,
      risks[event.accountId] ? demoPriceRisk(risks[event.accountId], settings, event, scenarioReady).score : 0]));
  }, [demoMode, events, risks, settings, scenarioReady]);
  const reviewEvents = latestByService.filter(event => needsPriceReview(event));
  const filtered = selectedEvents.filter(event => (clientId === 'all' || String(event.accountId) === clientId)
    && `${event.accountEmail ?? ''} ${event.planName} ${event.serviceId}`.toLocaleLowerCase().includes(query.toLocaleLowerCase()));
  const pageCount = Math.max(1, Math.ceil(filtered.length / pageSize));
  const safePage = Math.min(page, pageCount - 1);
  const visible = filtered.slice(safePage * pageSize, (safePage + 1) * pageSize);
  const accountOptions = [{ value: 'all', label: 'Все клиенты' }, ...[...new Map(selectedEvents.map(event => [event.accountId, { value: event.accountId, label: event.accountEmail ?? `Клиент ${event.accountId}` }])).values()]];
  if (clientId !== 'all' && !accountOptions.some(option => option.value === clientId)) {
    accountOptions.push({ value: clientId, label: selectedEvents.find(event => String(event.accountId) === clientId)?.accountEmail ?? `Клиент ${clientId}` });
  }

  return <div className="price-impact-page">
    <div className="page-heading"><div><div className="eyebrow">{demoMode ? 'СИНТЕТИЧЕСКИЙ СЦЕНАРИЙ' : 'РЕАЛЬНЫЕ НАБЛЮДЕНИЯ'}</div><h1>Ценовой риск</h1><p>История стоимости услуг и поведения клиентов после повышения цены.</p></div></div>
    <div className="price-impact-mode" role="group" aria-label="Режим данных"><button type="button" className={demoMode ? 'active' : ''} onClick={() => setParams(current => { const next = new URLSearchParams(current); next.set('mode', 'synthetic'); next.delete('client'); return next; })}>Синтетические данные</button><button type="button" className={!demoMode ? 'active' : ''} onClick={() => setParams(current => { const next = new URLSearchParams(current); next.set('mode', 'real'); next.delete('client'); return next; })}>Реальные данные</button></div>
    {demoMode && !loading && !error && <div className="price-impact-demo-note" role="status">{events.length} сценарных повышений связаны с ID клиентов и услуг из тестовой базы ({data?.accounts.length ?? '—'} аккаунтов). Продления, расходы и изменение числа услуг после повышения сгенерированы для демонстрации. {scenarioReady ? 'Проверка групп и контролей пройдена: сценарный ценовой фактор может влиять на сценарный Risk Score при положительном весе.' : 'Проверка достаточности и контрольных групп пока не пройдена: ценовой фактор не начисляет баллы.'} Реальный Risk Score не меняется.</div>}
    {planKey && <div className="price-impact-intro"><span>Фильтр по тарифу: <strong>{planKey}</strong>. В таблице показаны только услуги с подтверждённой связью с этим тарифом. <button type="button" onClick={() => { setParams({}); setPage(0); }}>Показать все</button></span></div>}
    {error && <div className="price-impact-error" role="alert">Не удалось загрузить историю стоимости: {error}</div>}
    {loading ? <div className="price-impact-empty">Загружаем изменения цен и продления…</div> : error ? null : <>
      {selectedEvents.length === 0 && <div className="price-impact-empty">{planKey ? 'По выбранному тарифу событий изменения стоимости нет. Сбросьте фильтр, чтобы увидеть доступную историю.' : 'Пока нет событий повышения стоимости для анализа.'}</div>}
      <div className="price-impact-summary">
        <div><small>Повышения стоимости</small><strong>{selectedEvents.length}</strong><span>{new Set(selectedEvents.map(event => event.accountId)).size} клиентов</span></div>
        <div><small>Полный период до и после</small><strong>{summary.eligible}</strong><span>Срок продления попал в 30 дней после изменения</span></div>
        <div><small>Продлили / негативное событие</small><strong>{summary.renewed} / {summary.adverse}</strong><span>Неясный исход: {summary.unknown}</span></div>
        <div><small>Наблюдаемые негативные события</small><strong>{summary.quality === 'insufficient' ? 'Мало данных' : percent((summary.observedAdverseShare ?? 0) * 100)}</strong><span>{summary.known} известных исходов · покрытие {percent(summary.coverage * 100)}</span></div>
      </div>
      <p className="price-impact-method">В анализ доли входит последнее пригодное изменение каждой услуги: она существовала 30 дней до повышения, срок продления попал в 30 дней после него, а последующий период полностью наблюдался. «Негативное событие» означает отсутствие подтверждённого продления вместе с отключением автопродления, prolong_wait или переходом в неактивный статус. Без этих событий отсутствие платежа остаётся неизвестным исходом. {summary.quality === 'insufficient' ? 'Для описательной доли нужны минимум 20 известных исходов и покрытие 80%.' : summary.quality === 'limited' ? 'Выборка ограничена: менее 100 известных исходов.' : 'Выборка достаточна для описания наблюдений.'}</p>

      <Panel title="Размер повышения и поведение" subtitle="Последнее пригодное повышение на клиента; 30 дней до и после" action={<TrendingUp size={17}/> }>
        <div className="table-scroll"><table className="price-impact-table"><thead><tr><th>Повышение</th><th>Клиентов</th><th>Негативное событие</th><th>Меньше услуг</th><th>Меньше платежей</th></tr></thead><tbody>{bands.map(band => <tr key={band.key}><td><strong>{band.label}</strong></td><td>{band.eligible}<small>{band.known} известных исходов · покрытие {percent(band.coverage * 100)}</small></td><td>{band.quality === 'insufficient' ? 'Мало данных' : percent((band.observedAdverseShare ?? 0) * 100)}<small>{band.adverse} из {band.known} известных</small></td><td>{band.contractionShare === null ? 'Мало данных' : percent(band.contractionShare * 100)}<small>{band.contracted} из {band.infrastructureObserved} с двумя снимками</small></td><td>{band.reducedPaymentsShare === null ? 'Мало данных' : percent(band.reducedPaymentsShare * 100)}<small>{band.reducedPayments} из {band.paymentsObserved} с платежами до повышения</small></td></tr>)}</tbody></table></div>
        <p className="price-impact-method">Число услуг сравнивается по ближайшим ежедневным снимкам около даты повышения и через 30–37 дней. Платежи — сумма выполненных заказов и продлений клиента за два 30-дневных окна; нулевые платежи до повышения не входят в долю сокращений. Доля негативных событий показана при 20 известных исходах и покрытии от 80%, доли сокращения — при 20 сравнимых клиентах. Одновременные изменения нескольких услуг одного клиента и другие причины оттока не исключены. Ценовой фактор добавляет баллы в общий Risk Score только после проверки достаточности и устойчивости реальных данных.</p>
      </Panel>

      <Panel title="До и после повышения" subtitle="Подтверждённые платежи клиента за три периода по 30 дней">
        <div className="table-scroll"><table className="price-impact-table"><thead><tr><th>Повышение</th><th>Сравнимых клиентов</th><th>Платежи раньше / до / после</th><th>Продления раньше / до / после</th></tr></thead><tbody>{bands.map(band => <tr key={band.key}><td>{band.label}</td><td>{band.paymentsObserved} из {band.eligible}</td><td>{money(band.accountSpendEarlierTotal / Math.max(1, band.paymentsObserved))} / {money(band.accountSpendBeforeTotal / Math.max(1, band.paymentsObserved))} / {money(band.accountSpendAfterTotal / Math.max(1, band.paymentsObserved))}<small>Среднее на клиента с платежами до повышения</small></td><td>{band.renewalsEarlierTotal} / {band.renewalsBeforeTotal} / {band.renewalsAfterTotal}<small>{band.eligible} наблюдений</small></td></tr>)}</tbody></table></div>
        <p className="price-impact-method">Суммы учитывают выполненные заказы и продления. Число продлений зависит и от того, наступал ли срок оплаты в каждом окне.</p>
      </Panel>

      <Panel title="Контрольная группа и устойчивость" subtitle="Похожие клиенты без изменения цены и две части годового периода">
        <div className="table-scroll"><table className="price-impact-table"><thead><tr><th>Повышение</th><th>Пар</th><th>Продлили с повышением / без</th><th>Изменение платежей с повышением / без</th></tr></thead><tbody>{bands.map(band => <tr key={band.key}><td>{band.label}</td><td>{band.matched}</td><td>{band.treatedRenewalShare === null ? 'Мало данных' : `${percent(band.treatedRenewalShare * 100)} / ${percent((band.controlRenewalShare ?? 0) * 100)}`}</td><td>{band.treatedSpendChange === null ? 'Мало данных' : `${percent(band.treatedSpendChange * 100)} / ${percent((band.controlSpendChange ?? 0) * 100)}`}<small>{band.matchedSpend} пар с платежами до повышения</small></td></tr>)}</tbody></table></div>
        <div className="table-scroll"><table className="price-impact-table"><thead><tr><th>Период</th><th>До 10%</th><th>10–25%</th><th>От 25%</th></tr></thead><tbody>{evidence.periods.map((period, index) => <tr key={index}><td>{index === 0 ? 'Предыдущие 6 месяцев' : 'Последние 6 месяцев'}</td>{period.map(band => <td key={band.key}>{band.eligible} клиентов<small>{band.known} известных исходов · {band.matched} пар</small></td>)}</tr>)}</tbody></table></div>
        <div className="table-scroll"><table className="price-impact-table"><thead><tr><th>Исходные расходы</th><th>До 10%</th><th>От 25%</th></tr></thead><tbody>{evidence.segments.map((segment, index) => <tr key={index}><td>{index === 0 ? `До медианы (${money(evidence.medianSpend ?? 0)})` : `Выше медианы (${money(evidence.medianSpend ?? 0)})`}</td>{[segment[0], segment[2]].map(band => <td key={band.key}>{band.eligible} клиентов<small>{band.known} известных исходов · {band.distinctControls} разных контролей</small><small>Негативные события: {band.quality === 'insufficient' ? 'мало данных' : percent((band.observedAdverseShare ?? 0) * 100)}</small></td>)}</tr>)}</tbody></table></div>
        <p className="price-impact-method">Контроль: тот же тариф и срок оплаты, близкие расходы (±30%) и число услуг (±1), одинаковое число продлений до события, сходная динамика платежей в двух предыдущих окнах, срок продления в том же окне и неизменная цена в течение 60 дней. Ухудшение исходной динамики исключает событие из проверки достаточности. Один контрольный клиент может попасть в несколько пар. Для устойчивого вывода нужны в каждой группе каждого периода 100 известных исходов, покрытие от 80% и 100 контрольных пар. Статус: {evidence.sufficient ? evidence.stable ? 'тенденция повторяется' : 'тенденция не повторяется' : 'данных недостаточно'}. {demoMode && 'Учебные события не подтверждают эффект цены у реальных клиентов.'}</p>
      </Panel>

      <Panel title="Клиенты для проверки" subtitle="Недавнее повышение, продление ожидается или прошло, подтверждённого продления после изменения нет" action={<CircleAlert size={17}/> }>
        {reviewEvents.length === 0 ? <p className="price-impact-empty">По текущей истории нет клиентов с таким сочетанием сигналов.</p> : <div className="price-impact-review-list">{reviewEvents.slice(0, 12).map(event => <div key={event.eventId}><div><strong>{event.accountEmail ?? `Клиент ${event.accountId}`}</strong><span>{event.planName} · услуга {event.serviceId} · повышение {percent(priceIncreasePercent(event))} · {date(event.changedAt)}{demoMode && scenarioScores.has(event.accountId) ? ` · сценарный Risk Score ${scenarioScores.get(event.accountId)}/100` : ''}</span></div><Link to={`/snapshot?client=${event.accountId}${demoMode ? '&mode=synthetic' : ''}`}>Открыть клиента <ArrowUpRight size={14}/></Link></div>)}</div>}
        {reviewEvents.length > 12 && <p className="price-impact-method">Показаны первые 12 из {reviewEvents.length}. Полная история изменений — в таблице ниже.</p>}
      </Panel>

      <Panel title="История изменений стоимости" subtitle="По каждой услуге: цена, продления за 30 дней до и после, качество наблюдения" action={<History size={17}/> }>
        <div className="price-impact-filters"><label>Клиент<GlassSelect value={clientId} onChange={value => { setParams(current => { const next = new URLSearchParams(current); if (value === 'all') next.delete('client'); else next.set('client', value); return next; }); setPage(0); }} searchable searchPlaceholder="Поиск клиента или email" options={accountOptions}/></label><label>Поиск<input value={query} onChange={event => { setQuery(event.target.value); setPage(0); }} placeholder="Тариф, email или ID услуги"/></label></div>
        <div className="table-scroll"><table className="price-impact-table"><thead><tr><th>Клиент / услуга</th><th>Когда</th><th>Было → стало</th><th>Продления до / после</th><th>После повышения</th><th>Наблюдение</th></tr></thead><tbody>{visible.map(event => { const outcome = priceOutcome(event); return <tr key={event.eventId}><td>{demoMode ? <span>{event.accountEmail ?? `Клиент ${event.accountId}`}</span> : <Link to={`/snapshot?client=${event.accountId}`}>{event.accountEmail ?? `Клиент ${event.accountId}`}</Link>}<small>{event.planName} · услуга {event.serviceId}</small></td><td>{date(event.changedAt)}<small>Срок продления: {date(event.nextRenewalAt)}</small></td><td>{money(event.previousPriceCents)} → {money(event.currentPriceCents)}<strong>+{percent(priceIncreasePercent(event))}</strong></td><td>{event.renewalsBefore} / {event.renewalsAfter}<small>Подтверждённые операции</small></td><td><span className={`price-impact-outcome is-${outcome}`}>{outcomeLabels[outcome]}</span><small>{event.autoRenewDisabledAfter ? 'Автопродление отключено · ' : ''}{event.prolongWaitAfter ? 'Был prolong_wait · ' : ''}{event.inactiveAfter ? 'Была приостановка/удаление' : ''}</small></td><td>{event.completeBaseline && event.completeFollowup && event.renewalDueInWindow ? 'Полное окно' : 'Недостаточно данных'}<small>{event.paymentTerm} · 30 дней до/после</small></td></tr>; })}{visible.length === 0 && <tr><td colSpan={6}>Подходящих изменений стоимости нет.</td></tr>}</tbody></table></div>
        <div className="price-impact-pagination"><span>Показано {filtered.length ? safePage * pageSize + 1 : 0}–{Math.min((safePage + 1) * pageSize, filtered.length)} из {filtered.length}</span><div><button type="button" aria-label="Предыдущая страница" disabled={safePage === 0} onClick={() => setPage(safePage - 1)}><ChevronLeft size={16}/></button><span>{safePage + 1} / {pageCount}</span><button type="button" aria-label="Следующая страница" disabled={safePage >= pageCount - 1} onClick={() => setPage(safePage + 1)}><ChevronRight size={16}/></button></div></div>
      </Panel>
    </>}
  </div>;
}
