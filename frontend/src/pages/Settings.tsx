import { useEffect, useMemo, useRef, useState, type CSSProperties } from 'react';
import { AlertCircle, CheckCircle2, Clock3, Info, LoaderCircle, RefreshCw, RotateCcw, Save, SlidersHorizontal } from 'lucide-react';
import { useAnalyzer } from '../hooks/useAnalyzer';
import { useAuth } from '../hooks/useAuth';
import { canEditSettings } from '../services/auth';
import { defaults } from '../mocks/data';
import { Panel } from '../components/common';
import { Button } from '../components/ui/button';
import { GlassSelect } from '../components/ui/GlassSelect';
import type { RiskResult, Settings as Config } from '../types';
import { previewRiskDistribution } from '../features/risk/settingsPreview';
import { useCurrency } from '../hooks/useCurrency';
import { convertCurrency, currencyOptions, formatNativeCurrency, type CurrencyCode } from '../features/market/currency';
import './settings.css';

const factorDescriptions: Record<keyof Config['weights'], { name: string; description: string }> = {
  paymentCoverage: { name: 'Покрытие ближайших платежей', description: 'Достаточно ли средств для оплаты в ближайшем периоде.' },
  noAutoProlong: { name: 'Услуги без автопродления', description: 'Доля услуг, которые не продлеваются автоматически.' },
  prolongWait: { name: 'Услуги в prolong_wait', description: 'Доля услуг, ожидающих завершения продления.' },
  failedProlong: { name: 'Неуспешные продления', description: 'Доля подтверждённых неуспешных попыток продления.' },
  expiringRevenue: { name: 'Выручка по скоро истекающим услугам', description: 'Доля месячной выручки от услуг, срок которых скоро закончится.' },
};

const riskLabels = [
  { key: 'low' as const, label: 'Низкий риск', color: '#27a978' },
  { key: 'medium' as const, label: 'Средний риск', color: '#f2b637' },
  { key: 'high' as const, label: 'Высокий риск', color: '#ec6d70' },
];

function validSettings(settings: Config) {
  const weights = Object.values(settings.weights);
  return weights.every(value => Number.isInteger(value) && value >= 0 && value <= 100)
    && weights.reduce((sum, value) => sum + value, 0) > 0
    && Number.isInteger(settings.mediumThreshold) && settings.mediumThreshold >= 1
    && Number.isInteger(settings.highThreshold) && settings.highThreshold > settings.mediumThreshold && settings.highThreshold <= 100
    && Number.isInteger(settings.renewalWindowDays) && settings.renewalWindowDays >= 1 && settings.renewalWindowDays <= 90
    && Number.isInteger(settings.historicalMinDays) && settings.historicalMinDays >= 7 && settings.historicalMinDays <= 365;
}

function currentDistribution(risks: Record<string, RiskResult>) {
  const counts = { low: 0, medium: 0, high: 0, limited: 0, insufficient: 0 };
  Object.values(risks).forEach(risk => { counts[risk.level] += 1; });
  return counts;
}

function savedAt(value: string | null | undefined) {
  if (!value) return 'Ещё не сохранялись';
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? 'Дата не указана' : new Intl.DateTimeFormat('ru-RU', { dateStyle: 'medium', timeStyle: 'short' }).format(date);
}

export default function Settings() {
  const { settings, settingsMetadata, risks, saveSettings } = useAnalyzer();
  const { admin } = useAuth();
  const { currency, setCurrency, fx, checkedAt, warning, loading: fxLoading, refresh: refreshFx } = useCurrency();
  const readOnly = !canEditSettings(admin);
  const [draft, setDraft] = useState<Config | null>(null);
  const [busy, setBusy] = useState(false);
  const [feedback, setFeedback] = useState<{ kind: 'success' | 'error'; text: string } | null>(null);
  const [convertAmount, setConvertAmount] = useState('100');
  const [convertFrom, setConvertFrom] = useState<CurrencyCode>('USD');
  const [convertTo, setConvertTo] = useState<CurrencyCode>('RUB');
  const [refreshingFx, setRefreshingFx] = useState(false);
  const serverSettingsRef = useRef('');

  useEffect(() => {
    if (!settings) return;
    const serialized = JSON.stringify(settings);
    setDraft(current => !current || JSON.stringify(current) === serverSettingsRef.current ? settings : current);
    serverSettingsRef.current = serialized;
  }, [settings]);

  const dirty = Boolean(settings && draft && JSON.stringify(draft) !== JSON.stringify(settings));
  const valid = draft ? validSettings(draft) : false;
  const totalWeight = draft ? Object.values(draft.weights).reduce((sum, value) => sum + value, 0) : 0;
  const preview = useMemo(() => risks && draft ? previewRiskDistribution(risks, draft) : null, [risks, draft]);
  const current = useMemo(() => risks ? currentDistribution(risks) : null, [risks]);
  const parsedAmount = convertAmount.trim() ? Number(convertAmount) : Number.NaN;
  const convertedAmount = convertCurrency(parsedAmount, convertFrom, convertTo, fx);

  async function updateFx() {
    setRefreshingFx(true);
    try { await refreshFx(); }
    finally { setRefreshingFx(false); }
  }

  if (!settings || !draft) return null;

  async function save() {
    if (!draft || !valid || readOnly || busy) return;
    setBusy(true);
    setFeedback(null);
    try {
      const metadata = await saveSettings(draft);
      setFeedback({ kind: 'success', text: `Настройки сохранены, Risk Score пересчитан. Версия ${metadata.version}.` });
    } catch (error) {
      setFeedback({ kind: 'error', text: `Не удалось сохранить настройки: ${error instanceof Error ? error.message : 'ошибка сервера'}. Ваши значения оставлены в форме.` });
    } finally {
      setBusy(false);
    }
  }

  function restoreDefaults() {
    if (readOnly || busy) return;
    if (!window.confirm('Восстановить рекомендуемые значения всех настроек? Изменения применятся только после сохранения.')) return;
    setDraft(structuredClone(defaults));
    setFeedback(null);
  }

  const updateWeight = (key: keyof Config['weights'], value: number) => {
    setDraft(currentDraft => currentDraft ? { ...currentDraft, weights: { ...currentDraft.weights, [key]: value } } : currentDraft);
    setFeedback(null);
  };
  const updateNumber = (key: 'mediumThreshold' | 'highThreshold' | 'renewalWindowDays' | 'historicalMinDays', value: number) => {
    setDraft(currentDraft => currentDraft ? { ...currentDraft, [key]: value } : currentDraft);
    setFeedback(null);
  };
  const share = (count: number) => {
    const classified = preview ? preview.low + preview.medium + preview.high : 0;
    return classified ? Math.round(count / classified * 100) : 0;
  };

  return <div className="settings-page">
    <div className="settings-heading">
      <div>
        <div className="eyebrow">КОНФИГУРАЦИЯ ПЛАТФОРМЫ</div>
        <h1>Настройки</h1>
        <p>Настройте расчёт Risk Score и исторической модели под особенности вашей клиентской базы.</p>
      </div>
      <div className="settings-heading-actions">
        <Button variant="default" disabled={readOnly || !valid || busy || !dirty} onClick={() => void save()}>
          {busy ? <LoaderCircle size={15} className="spin" /> : <Save size={15} />}
          {busy ? 'Сохранение…' : 'Сохранить изменения'}
        </Button>
        <div className={`settings-save-state ${dirty ? 'is-dirty' : 'is-saved'}`} role="status">
          {dirty ? <AlertCircle size={15} /> : <CheckCircle2 size={15} />}
          <span>{dirty ? 'Есть несохранённые изменения' : 'Нет несохранённых изменений'}</span>
        </div>
        <small>Версия {settingsMetadata?.version ?? 0} · сохранено {savedAt(settingsMetadata?.updatedAt)}{settingsMetadata?.updatedBy ? ` · ${settingsMetadata.updatedBy}` : ''}</small>
      </div>
    </div>

    <div className="settings-notice"><Info size={17} /><span>{readOnly
      ? 'Роль viewer: настройки доступны только для просмотра. Изменять их могут operator и superadmin.'
      : 'Изменения применяются после сохранения. Risk Score пересчитывается сразу и обновляется во всех открытых сессиях.'}</span></div>

    {feedback && <div className={`settings-feedback ${feedback.kind}`} role={feedback.kind === 'error' ? 'alert' : 'status'}>
      {feedback.kind === 'error' ? <AlertCircle size={16} /> : <CheckCircle2 size={16} />}{feedback.text}
    </div>}
    {dirty && settingsMetadata && <div className="settings-conflict-note" role="status">Текущий сохранённый вариант — версия {settingsMetadata.version}. Предпросмотр ниже показывает черновик.</div>}

    <div className="settings-layout">
      <Panel title="Коэффициенты Risk Score" subtitle="Относительный вклад каждого признака в итоговый риск" action={<SlidersHorizontal size={18} />}>
        <div className="settings-weight-list">
          {(Object.keys(factorDescriptions) as (keyof Config['weights'])[]).map(key => {
            const factor = factorDescriptions[key];
            const value = draft.weights[key];
            return <div className="settings-weight" key={key}>
              <div className="settings-weight-label"><strong>{factor.name}</strong><span>{totalWeight ? Math.round(value / totalWeight * 100) : 0}% вклада</span></div>
              <small>{factor.description}</small>
              <div className="settings-weight-controls">
                <input aria-label={`${factor.name}: ползунок`} type="range" min="0" max="100" step="1" value={value} disabled={readOnly} onChange={event => updateWeight(key, Number(event.currentTarget.value))} />
                <label><input aria-label={`${factor.name}: вес`} type="number" min="0" max="100" step="1" value={value} disabled={readOnly} onChange={event => updateWeight(key, Number(event.currentTarget.value))} />%</label>
              </div>
            </div>;
          })}
        </div>
        <div className="settings-total"><span>Сумма весов нормализуется до 100%</span><strong>{totalWeight}</strong></div>
      </Panel>

      <div className="settings-side">
        <Panel title="Пороги риска" subtitle="Границы классификации клиентов по Risk Score (0–100)">
          <div className="settings-threshold-fields">
            <label>Средний риск от<input type="number" min="1" max="99" step="1" value={draft.mediumThreshold} disabled={readOnly} onChange={event => updateNumber('mediumThreshold', Number(event.currentTarget.value))} /></label>
            <label>Высокий риск от<input type="number" min="2" max="100" step="1" value={draft.highThreshold} disabled={readOnly} onChange={event => updateNumber('highThreshold', Number(event.currentTarget.value))} /></label>
          </div>
          <div className="settings-threshold-track" style={{ '--medium-stop': `${draft.mediumThreshold}%`, '--high-stop': `${draft.highThreshold}%` } as CSSProperties} />
          <div className="settings-threshold-labels"><span>0</span><span>{draft.mediumThreshold}</span><span>{draft.highThreshold}</span><span>100</span></div>
          <div className="settings-threshold-legend">{riskLabels.map(item => <span key={item.key}><i style={{ background: item.color }} />{item.label}</span>)}</div>
        </Panel>

        <Panel title="Параметры моделей" subtitle="Периоды анализа и минимальная история для выводов">
          <div className="settings-model-fields">
            <label><span><strong>Окно продлений, дней</strong><small>На какой период вперёд искать предстоящие продления и платежи для Risk Score.</small></span><input type="number" min="1" max="90" step="1" value={draft.renewalWindowDays} disabled={readOnly} onChange={event => updateNumber('renewalWindowDays', Number(event.currentTarget.value))} /></label>
            <label><span><strong>Минимум истории, дней</strong><small>При меньшей длительности Historical Model показывает данные, но не делает уверенный вывод. Новое значение применяется при следующем снимке истории.</small></span><input type="number" min="7" max="365" step="1" value={draft.historicalMinDays} disabled={readOnly} onChange={event => updateNumber('historicalMinDays', Number(event.currentTarget.value))} /></label>
          </div>
        </Panel>

        <Panel title="Предпросмотр влияния настроек" subtitle="Распределение по новым весам и порогам на текущих факторах">
          {preview && current ? <>
            <div className="settings-preview-bar" aria-label="Предпросмотр распределения по уровню риска">
              {riskLabels.map(item => <span key={item.key} style={{ width: `${share(preview[item.key])}%`, background: item.color }} title={`${item.label}: ${preview[item.key]} клиентов`}>
                {share(preview[item.key]) >= 8 ? `${share(preview[item.key])}%` : ''}
              </span>)}
            </div>
            <div className="settings-preview-stats">
              {riskLabels.map(item => <div key={item.key}><i style={{ background: item.color }} /><span>{item.label}<small>{preview[item.key].toLocaleString('ru-RU')} клиентов · было {current[item.key].toLocaleString('ru-RU')}</small></span></div>)}
            </div>
            <p className="settings-preview-note">Предпросмотр использует текущие значения факторов; новое окно продлений будет учтено сервером после сохранения. {preview.limited + preview.insufficient > 0 && `${preview.limited + preview.insufficient} клиентов с неполными данными показаны отдельно и не входят в цветовую шкалу.`}</p>
            {(preview.movedUp > 0 || preview.movedDown > 0) && <p className="settings-preview-shift">По сравнению с текущей конфигурацией: {preview.movedUp} клиентов перейдут в более высокий риск, {preview.movedDown} — в более низкий.</p>}
          </> : <p className="settings-preview-note">Предпросмотр появится после загрузки клиентов и их Risk Score.</p>}
        </Panel>
      </div>
    </div>

    <Panel title="Валюта и конвертация" subtitle="Выберите валюту отображения. Выбор сохраняется в этом браузере, исходные цены остаются без изменений.">
      <div className="settings-currency-content">
        <div className="settings-currency-topline">
          <div className="settings-currency-primary"><span>Основная валюта</span>
            <GlassSelect className="settings-currency-select" aria-label="Основная валюта" value={currency} onChange={value => setCurrency(value as CurrencyCode)} options={currencyOptions} />
          </div>
          <div className="settings-fx-status">
            <div className={`settings-fx-indicator ${warning ? 'is-stale' : fx ? 'is-ready' : 'is-missing'}`}>
              <span />{warning ? 'Последний проверенный курс' : fxLoading ? 'Загружаем курсы…' : fx ? 'Курсы доступны' : 'Курс недоступен'}
            </div>
            <small>{fx ? `Ставка ЦБ от ${savedAt(fx.date)} · API проверен ${savedAt(checkedAt)} · проверка каждые 15 минут` : 'Нет проверенного курса в API или локальном кеше.'}</small>
          </div>
          <Button variant="outline" disabled={refreshingFx} onClick={() => void updateFx()}>
            {refreshingFx ? <LoaderCircle size={14} className="spin" /> : <RefreshCw size={14} />}
            {refreshingFx ? 'Проверка…' : 'Проверить API'}
          </Button>
        </div>
        <div className="settings-fx-rates">
          <div><span>1 USD</span><strong>{fx ? formatNativeCurrency(1, 'USD', 'RUB', fx) : '—'}</strong></div>
          <div><span>1 EUR</span><strong>{fx ? formatNativeCurrency(1, 'EUR', 'RUB', fx) : '—'}</strong></div>
          <a href={fx?.source ?? 'https://www.cbr.ru/scripts/XML_daily.asp'} target="_blank" rel="noreferrer">Источник курса: Банк России</a>
        </div>
        {warning && <p className="settings-fx-warning" role="status"><AlertCircle size={14} />{warning}</p>}
        <div className="settings-converter">
          <strong>Ручной пересчёт</strong>
          <div className="settings-converter-row">
            <label>Сумма<input type="number" min="0" step="any" value={convertAmount} onChange={event => setConvertAmount(event.currentTarget.value)} /></label>
            <div className="settings-converter-field"><span>Из валюты</span><GlassSelect className="settings-currency-select" aria-label="Из валюты" value={convertFrom} onChange={value => setConvertFrom(value as CurrencyCode)} options={currencyOptions.map(option => ({ ...option, label: option.value }))} /></div>
            <span className="settings-converter-arrow" aria-hidden="true">→</span>
            <div className="settings-converter-field"><span>В валюту</span><GlassSelect className="settings-currency-select" aria-label="В валюту" value={convertTo} onChange={value => setConvertTo(value as CurrencyCode)} options={currencyOptions.map(option => ({ ...option, label: option.value }))} /></div>
            <output aria-live="polite">{!Number.isFinite(parsedAmount) ? 'Введите сумму' : convertedAmount === null ? 'Курс недоступен' : formatNativeCurrency(convertedAmount, convertTo, convertTo, fx)}</output>
          </div>
          <small>Используется последний проверенный снимок Market API; серверный сборщик обновляет его из источника курсов. Если API недоступен, показывается сохранённый курс с датой, без пометки «актуальный».</small>
        </div>
      </div>
    </Panel>

    {!valid && <div className="settings-validation" role="alert">Проверьте значения: сумма весов должна быть больше нуля, пороги — целыми числами с 0 &lt; средний &lt; высокий ≤ 100; окно продлений — 1–90 дней, минимум истории — 7–365 дней.</div>}
    <div className="settings-footer">
      <Button variant="outline" disabled={readOnly || busy} onClick={restoreDefaults}><RotateCcw size={14} />Восстановить по умолчанию</Button>
      <span><Clock3 size={13} />Версия конфигурации хранится вместе с автором и временем сохранения.</span>
    </div>
  </div>;
}
