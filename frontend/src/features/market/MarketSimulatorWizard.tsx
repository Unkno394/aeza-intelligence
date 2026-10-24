import { useEffect, useMemo, useState } from 'react';
import {
  Activity,
  ArrowLeft,
  ArrowRight,
  Check,
  CircleHelp,
  Database,
  Gauge,
  MapPin,
  Play,
  RefreshCw,
  Server,
  ShieldCheck,
  Sparkles,
  Users,
  Wallet
} from 'lucide-react';
import { input2, number2 } from './format';
import type { Plan, Scenario, Sensitivity } from './api';
import { priceChangeMultiplier } from './scenario';
import { GlassSelect } from '../../components/ui/GlassSelect';

type WizardStep = 1 | 2 | 3 | 4;
type MarketType = 'economy' | 'extended';
type PriceSource = 'observed' | 'manual';
type SegmentNumberKey = 'budget_min' | 'budget_max' | 'vcpu' | 'ram_gb' | 'disk_gb';

interface Props {
  draft: Scenario;
  template: Scenario;
  official: Plan[];
  preset: string;
  onPresetChange: (value: string) => void;
  strategies: string[];
  onStrategiesChange: (value: string[]) => void;
  custom: string;
  onCustomChange: (value: string) => void;
  marketType: MarketType;
  onMarketTypeChange: (value: MarketType) => void;
  priceSource: PriceSource;
  onPriceSourceChange: (value: PriceSource) => void;
  onDraftChange: (value: Scenario) => void;
  onApplyObserved: (city: string) => boolean;
  onOpenData: () => void;
  onCalculate: () => void;
  busy: boolean;
  error: string;
  hasReport: boolean;
  stale: boolean;
  statusText: string;
}

const steps = [
  { title: 'Настройка рынка', hint: 'Регион, период, спрос', icon: MapPin },
  { title: 'Цены и стратегии', hint: 'Тарифы и изменения', icon: Wallet },
  { title: 'Допущения модели', hint: 'Бюджеты, затраты, ограничения', icon: Gauge },
  { title: 'Расчёт и результаты', hint: 'Графики и сравнение', icon: Activity }
];

const strategyLabels: Record<string, string> = {
  baseline: 'Базовая цена',
  discount_20: 'Скидка 20%',
  increase_20: 'Повышение 20%',
  intro_then_increase: 'Скидка → повышение',
  custom: 'Своя стратегия'
};
const strategyDescriptions: Record<string, string> = {
  baseline: 'Тарифы остаются по текущей базовой цене.',
  discount_20: 'Проверяем снижение цены на 20% на всём горизонте.',
  increase_20: 'Проверяем повышение цены на 20% на всём горизонте.',
  intro_then_increase: 'Сначала скидка, затем цена возвращается и повышается.',
  custom: 'Задайте своё изменение цены для всех тарифов.'
};
const sensitivityLabels: Record<Sensitivity, string> = { low: 'Низкая', medium: 'Средняя', high: 'Высокая' };
const sensitivityDescriptions: Record<Sensitivity, string> = {
  low: 'Покупатели слабее реагируют на разницу в цене.',
  medium: 'Средняя реакция на изменение цены.',
  high: 'Покупатели сильнее выбирают более дешёвые предложения.'
};
const rub = (value: number) => new Intl.NumberFormat('ru-RU', { style: 'currency', currency: 'RUB', maximumFractionDigits: 2 }).format(value);

export function MarketSimulatorWizard({
  draft,
  template,
  official,
  preset,
  onPresetChange,
  strategies,
  onStrategiesChange,
  custom,
  onCustomChange,
  marketType,
  onMarketTypeChange,
  priceSource,
  onPriceSourceChange,
  onDraftChange,
  onApplyObserved,
  onOpenData,
  onCalculate,
  busy,
  error,
  hasReport,
  stale,
  statusText
}: Props) {
  const [step, setStep] = useState<WizardStep>(1);
  const [city, setCity] = useState(() => draft.aeza_plans.flatMap(plan => plan.regions)[0] ?? 'Moscow');
  const [direction, setDirection] = useState(1);
  const cities = useMemo(() => [...new Set(official.flatMap(plan => plan.regions))].sort(), [official]);

  useEffect(() => {
    if (cities.length && !cities.includes(city)) setCity(cities.includes('Moscow') ? 'Moscow' : cities[0]);
  }, [cities, city]);

  const update = (patch: Partial<Scenario>) => onDraftChange({ ...draft, ...patch });
  const changeStep = (next: WizardStep) => {
    if (next > step) setDirection(1);
    else if (next < step) setDirection(-1);
    setStep(next);
  };
  const changeMarket = (market: string) => {
    const region = market === 'russia' ? 'Russia' : market === 'international' ? (city === 'Moscow' ? 'EU' : city) : null;
    update({ market, segments: draft.segments.map(segment => ({ ...segment, region })) });
  };
  const toggleStrategy = (name: string, checked: boolean) => {
    onStrategiesChange(checked ? [...strategies, name] : strategies.filter(value => value !== name));
  };
  const updateSegment = (index: number, key: SegmentNumberKey, value: number) => {
    const segments = [...draft.segments];
    const next = { ...segments[index], [key]: value };
    if (key === 'budget_min' && value > next.budget_max) next.budget_max = value;
    if (key === 'budget_max' && value < next.budget_min) next.budget_min = value;
    segments[index] = next;
    update({ segments });
  };
  const setCityAndApply = (nextCity: string) => {
    setCity(nextCity);
    if (priceSource === 'observed') onApplyObserved(nextCity);
  };

  const priceOptions = official.filter(plan => plan.regions.includes(city));
  const customMultiplier = priceChangeMultiplier(Number.isFinite(Number(custom)) ? Number(custom) : 0);
  const estimate = `${draft.aeza_plans.length} тариф${draft.aeza_plans.length === 1 ? '' : draft.aeza_plans.length < 5 ? 'а' : 'ов'}`;
  const activeInfo = [
    'Задайте границы рынка и исходный спрос. Сценарий Akash использует историческую форму притока, а масштаб покупателей остаётся вашим допущением.',
    'Укажите цены Aéza и выберите варианты для сравнения. Результаты покажут разницу между стратегиями при одинаковых условиях спроса.',
    'Задайте бюджеты сегментов и при необходимости включите затраты, уход, переходы и ограничения ресурсов.',
    'Запустите расчёт. Это сценарная модель рынка, а не точный прогноз поведения реальных клиентов.'
  ][step - 1];

  return (
    <section className="market-wizard" aria-label="Пошаговая настройка Market Simulator">
      <div className="market-wizard-steps" role="list" aria-label="Этапы симуляции">
        {steps.map((item, index) => {
          const number = (index + 1) as WizardStep;
          const Icon = item.icon;
          const complete = number < step;
          const active = number === step;
          return (
            <button
              key={item.title}
              type="button"
              className="market-wizard-step"
              data-active={active || undefined}
              data-complete={complete || undefined}
              aria-current={active ? 'step' : undefined}
              aria-label={`Шаг ${number}: ${item.title}`}
              disabled={number > step}
              onClick={() => changeStep(number)}
            >
              <span className="market-wizard-step-number">{complete ? <Check size={17} /> : <Icon size={17} />}</span>
              <span className="market-wizard-step-copy"><strong>{item.title}</strong><small>{item.hint}</small></span>
            </button>
          );
        })}
      </div>

      <div className="market-wizard-body">
        <div className="market-wizard-content" key={step} data-direction={direction}>
          {step === 1 && <>
            <header className="market-wizard-heading">
              <span className="market-wizard-kicker">ШАГ 1 ИЗ 4</span>
              <h2>Настройка рынка</h2>
              <p>Выберите рынок и задайте горизонт симуляции. Эти параметры определят доступные предложения и динамику новых покупателей.</p>
            </header>

            <div className="market-wizard-grid">
              <label>Сценарий рынка
                <GlassSelect aria-label="Сценарий рынка" value={preset} onChange={onPresetChange} options={[
                  { value: 'akash', label: 'Динамика Akash' },
                  { value: 'flat', label: 'Ровный приток' },
                  { value: 'budget', label: 'Бюджеты ниже на 30%' }
                ]} />
                <small>Готовая форма притока и бюджета покупателей.</small>
              </label>
              <label>Рынок
                <GlassSelect aria-label="Регион рынка" value={draft.market ?? 'all'} onChange={changeMarket} options={[
                  { value: 'russia', label: 'Россия' },
                  { value: 'international', label: 'Международный' },
                  { value: 'all', label: 'Общий рынок' }
                ]} />
                <small>Определяет географию предложений и размещения.</small>
              </label>
              <label>Город тарифов Aéza
                <GlassSelect aria-label="Город тарифов Aéza" value={city} onChange={setCityAndApply} options={cities.map(value => ({ value, label: value }))} />
                <small>{priceSource === 'observed' ? 'Для выбранного города используются тарифы из проверенного снимка.' : 'Город можно применить при выборе официальных цен.'}</small>
              </label>
              <label>Период симуляции, мес.
                <input aria-label="Количество месяцев" type="number" required min={1} max={12} value={draft.months} onChange={event => {
                  const months = Math.max(1, Math.min(12, Math.floor(Number(event.target.value) || 1)));
                  update({ months, arrival_weights: Array.from({ length: months }, (_, index) => preset === 'akash' ? template.arrival_weights[index % template.arrival_weights.length] : 1) });
                }} />
                <small>На сколько месяцев вперёд выполнить расчёт.</small>
              </label>
              <label>Приток покупателей в месяц
                <input aria-label="Базовый приток покупателей" type="number" required min={0} max={1000} value={draft.buyers_per_month} onChange={event => update({ buyers_per_month: Number(event.target.value) })} />
                <small>Количество новых покупателей в начале периода.</small>
              </label>
              <label>Seed для воспроизводимости
                <input aria-label="Seed для воспроизводимости" type="number" required min={0} max={Number.MAX_SAFE_INTEGER} step={1} value={draft.seed} onChange={event => update({ seed: Number(event.target.value) })} />
                <small>Одинаковый seed и настройки дают одинаковый результат.</small>
              </label>
            </div>

            <div className="market-wizard-divider" />
            <div className="market-wizard-section-title"><div><h3>Тип модели</h3><p>Выберите уровень экономических допущений.</p></div></div>
            <div className="market-wizard-cards market-wizard-models" role="radiogroup" aria-label="Тип модели рынка">
              <button type="button" role="radio" aria-checked={marketType === 'economy'} className="market-wizard-choice" data-selected={marketType === 'economy' || undefined} onClick={() => onMarketTypeChange('economy')}>
                <span className="market-wizard-choice-icon"><Activity size={18} /></span><span><strong>Экономика и удержание</strong><small>Базовая динамика спроса и продлений. Дополнительные затраты и барьер перехода в расчёте равны нулю.</small></span><span className="market-wizard-radio" />
              </button>
              <button type="button" role="radio" aria-checked={marketType === 'extended'} className="market-wizard-choice" data-selected={marketType === 'extended' || undefined} onClick={() => onMarketTypeChange('extended')}>
                <span className="market-wizard-choice-icon"><ShieldCheck size={18} /></span><span><strong>Уход · затраты · стоимость перехода</strong><small>Включает дополнительные финансовые параметры и чувствительность к смене провайдера.</small></span><span className="market-wizard-radio" />
              </button>
            </div>

            <div className="market-wizard-section-title market-wizard-section-spaced"><div><h3>Начальные условия</h3><p>Откуда взять стартовые цены тарифов Aéza.</p></div></div>
            <div className="market-wizard-cards market-wizard-source" role="radiogroup" aria-label="Источник стартовых цен">
              <button type="button" role="radio" aria-checked={priceSource === 'observed'} className="market-wizard-choice" data-selected={priceSource === 'observed' || undefined} onClick={() => { if (onApplyObserved(city)) onPriceSourceChange('observed'); }}>
                <span className="market-wizard-choice-icon"><Database size={18} /></span><span><strong>Текущие рыночные данные</strong><small>{priceOptions.length ? `${priceOptions.length} официальных тарифов Aéza доступны для ${city}.` : 'Тарифы из последнего проверенного снимка.'}</small></span><span className="market-wizard-radio" />
              </button>
              <button type="button" role="radio" aria-checked={priceSource === 'manual'} className="market-wizard-choice" data-selected={priceSource === 'manual' || undefined} onClick={() => { onPriceSourceChange('manual'); onDraftChange({ ...draft, aeza_plans: draft.aeza_plans.map(plan => ({ ...plan, origin: 'scenario_assumption', details: null, source: 'scenario://user-input' })) }); }}>
                <span className="market-wizard-choice-icon"><Sparkles size={18} /></span><span><strong>Задать цены вручную</strong><small>Использовать тарифы как условие сценария и отредактировать их на следующем шаге.</small></span><span className="market-wizard-radio" />
              </button>
            </div>
            {!cities.length && <p className="market-wizard-note"><CircleHelp size={15} />В снимке не найден список городов Aéza. Начальные цены можно задать вручную или проверить в разделе «Данные».</p>}
          </>}

          {step === 2 && <>
            <header className="market-wizard-heading">
              <span className="market-wizard-kicker">ШАГ 2 ИЗ 4</span>
              <h2>Цены и стратегии</h2>
              <p>Сравните базовые цены с изменениями. При редактировании тарифа наблюдаемая цена сохраняется как исходная, а расчёт использует введённое значение.</p>
            </header>
            <div className="market-wizard-section-title"><div><h3>Базовые цены Aéza</h3><p>{estimate} · ежемесячная стоимость, RUB</p></div><span className="market-wizard-source-tag" data-source={priceSource}>{priceSource === 'observed' ? 'Проверенный снимок' : 'Сценарные цены'}</span></div>
            <div className="market-wizard-prices">
              {draft.aeza_plans.map((plan, index) => <label className="market-wizard-price" key={plan.id}>
                <span><strong>{plan.name}</strong><small>{number2(plan.vcpu)} vCPU · {number2(plan.ram_gb)} GB RAM · {number2(plan.disk_gb)} GB диск</small></span>
                <span className="market-wizard-price-input"><input aria-label={`Цена Aéza ${plan.name}`} type="number" required min={0.01} max={1_000_000_000} step={0.01} value={input2(plan.monthly_usd)} onChange={event => {
                  const plans = [...draft.aeza_plans];
                  plans[index] = { ...plan, monthly_usd: input2(Number(event.target.value)), origin: plan.details ? 'scenario_override_of_observed' : 'scenario_assumption' };
                  onDraftChange({ ...draft, aeza_plans: plans });
                  onPriceSourceChange('manual');
                }} /><span>₽ / мес.</span></span>
              </label>)}
            </div>
            <div className="market-wizard-divider" />
            <div className="market-wizard-section-title"><div><h3>Стратегии для сравнения</h3><p>Выберите хотя бы один вариант, чтобы рассчитать сценарий.</p></div></div>
            <div className="market-wizard-strategies">
              {[...template.strategies.map(strategy => strategy.name), 'custom'].map(name => <label className="market-wizard-strategy" key={name} data-selected={strategies.includes(name) || undefined}>
                <input type="checkbox" checked={strategies.includes(name)} onChange={event => toggleStrategy(name, event.target.checked)} />
                <span className="market-wizard-check"><Check size={13} /></span>
                <span><strong>{strategyLabels[name] ?? name}</strong><small>{strategyDescriptions[name] ?? 'Изменение ценовой стратегии для тарифов Aéza.'}</small></span>
              </label>)}
            </div>
            {strategies.includes('custom') && <div className="market-wizard-custom">
              <label>Изменение цены, %<input aria-label="Изменение цены Aéza, %" type="number" required min={-95} max={1900} step={0.01} value={custom} onChange={event => onCustomChange(event.target.value)} /></label>
              <p>Например, +15% повысит базовую цену, −15% снизит её. Цены по стратегии: {draft.aeza_plans.map(plan => rub(plan.monthly_usd * customMultiplier)).join(' · ')}.</p>
            </div>}
            <div className="market-wizard-section-title market-wizard-section-spaced"><div><h3>Ценовая чувствительность</h3><p>Покажет диапазон реакции покупателей на разницу в стоимости.</p></div></div>
            <div className="market-wizard-sensitivity">
              {(['low', 'medium', 'high'] as Sensitivity[]).map(level => <label className="market-wizard-sensitivity-card" key={level} data-selected={draft.sensitivities.includes(level) || undefined}>
                <input type="checkbox" checked={draft.sensitivities.includes(level)} onChange={event => update({ sensitivities: event.target.checked ? [...draft.sensitivities, level] : draft.sensitivities.filter(value => value !== level) })} />
                <span className="market-wizard-check"><Check size={13} /></span><span><strong>{sensitivityLabels[level]}</strong><small>{sensitivityDescriptions[level]}</small></span>
              </label>)}
            </div>
          </>}

          {step === 3 && <>
            <header className="market-wizard-heading">
              <span className="market-wizard-kicker">ШАГ 3 ИЗ 4</span>
              <h2>Допущения модели</h2>
              <p>Уточните бюджеты и потребности сегментов. Расширенные параметры можно оставить свёрнутыми и использовать значения сценария.</p>
            </header>
            <div className="market-wizard-section-title"><div><h3>Бюджеты покупателей</h3><p>Границы месячного бюджета сегмента, RUB.</p></div><Users size={18} /></div>
            <div className="market-wizard-segments">
              {draft.segments.map((segment, index) => <div className="market-wizard-segment" key={segment.name}>
                <div><strong>{segment.name}</strong><small>Доля {number2(segment.weight * 100)}% · минимум ресурса {number2(segment.vcpu)} vCPU / {number2(segment.ram_gb)} GB RAM</small></div>
                <label>От<input type="number" min={0} step={0.01} aria-label={`${segment.name}: минимальный бюджет`} value={input2(segment.budget_min)} onChange={event => updateSegment(index, 'budget_min', input2(Number(event.target.value)))} /></label>
                <label>До<input type="number" min={0} step={0.01} aria-label={`${segment.name}: максимальный бюджет`} value={input2(segment.budget_max)} onChange={event => updateSegment(index, 'budget_max', input2(Number(event.target.value)))} /></label>
              </div>)}
            </div>

            <details className="market-assumptions market-wizard-advanced" open={marketType === 'extended'}>
              <summary>Уход, затраты и стоимость перехода<span>{marketType === 'extended' ? 'Учитываются в расчёте' : 'Не учитываются в базовом режиме'}</span></summary>
              <div className="market-wizard-grid market-wizard-grid-advanced">
                <label>Вероятность ухода в месяц, %<input type="number" min={0} max={100} step={0.01} disabled={marketType === 'economy'} value={Number((draft.monthly_departure_probability * 100).toFixed(2))} onChange={event => update({ monthly_departure_probability: Number(event.target.value) / 100 })} /><small>Базовая вероятность ухода из Aéza за один месяц.</small></label>
                <label>Переменные затраты на аренду, ₽<input type="number" min={0} max={1_000_000} step={0.01} disabled={marketType === 'economy'} value={input2(draft.aeza_variable_cost_usd)} onChange={event => update({ aeza_variable_cost_usd: input2(Number(event.target.value)) })} /><small>Затраты на одну активную аренду в месяц.</small></label>
                <label>Постоянные затраты в месяц, ₽<input type="number" min={0} max={1_000_000_000} step={0.01} disabled={marketType === 'economy'} value={input2(draft.aeza_fixed_cost_usd)} onChange={event => update({ aeza_fixed_cost_usd: input2(Number(event.target.value)) })} /><small>Учитываются при расчёте сценарной прибыли.</small></label>
                <label>Стоимость перехода, utility<input type="number" min={0} max={20} step={0.01} disabled={marketType === 'economy'} value={input2(draft.switching_cost_utility)} onChange={event => update({ switching_cost_utility: input2(Number(event.target.value)) })} /><small>Снижает склонность действующего клиента сменить провайдера.</small></label>
              </div>
              {marketType === 'economy' && <p className="market-wizard-inline-note">В режиме «Экономика и удержание» затраты и стоимость перехода временно равны нулю. Исходные значения сохраняются и снова применятся в расширенном режиме.</p>}
            </details>

            <details className="market-assumptions market-wizard-advanced">
              <summary>Конфигурации и ограничения рынка<span>Дополнительные настройки модели</span></summary>
              <div className="market-wizard-grid market-wizard-grid-advanced">
                <label>Виртуализация
                  <GlassSelect aria-label="Требование виртуализации" value={draft.required_virtualization ?? ''} onChange={value => update({ required_virtualization: value || null })} options={[{ value: '', label: 'Без требования' }, { value: 'kvm', label: 'Только подтверждённый KVM' }]} />
                  <small>Фильтр допустимых предложений.</small>
                </label>
                <label>Класс CPU
                  <GlassSelect aria-label="Требуемый класс CPU" value={draft.required_cpu_class ?? ''} onChange={value => update({ required_cpu_class: value || null })} options={[{ value: '', label: 'Любой или неизвестный' }, { value: 'shared', label: 'Shared' }, { value: 'dedicated', label: 'Dedicated' }]} />
                  <small>Можно исключить неподходящие конфигурации.</small>
                </label>
                {draft.aeza_capacity && <>
                  <label>Лимит vCPU Aéza<input type="number" min={0} value={input2(draft.aeza_capacity.vcpu)} onChange={event => update({ aeza_capacity: { ...draft.aeza_capacity!, vcpu: Number(event.target.value), origin: 'scenario_assumption', source: 'scenario://user-defined-resource-pool' } })} /><small>Сценарная ёмкость, не опубликованный остаток оборудования.</small></label>
                  <label>Лимит RAM, GB<input type="number" min={0} value={input2(draft.aeza_capacity.ram_gb)} onChange={event => update({ aeza_capacity: { ...draft.aeza_capacity!, ram_gb: Number(event.target.value), origin: 'scenario_assumption', source: 'scenario://user-defined-resource-pool' } })} /><small>Суммарное ограничение ресурсов.</small></label>
                  <label>Лимит диска, GB<input type="number" min={0} value={input2(draft.aeza_capacity.storage_gb)} onChange={event => update({ aeza_capacity: { ...draft.aeza_capacity!, storage_gb: Number(event.target.value), origin: 'scenario_assumption', source: 'scenario://user-defined-resource-pool' } })} /><small>Суммарный объём хранилища для новых аренд.</small></label>
                </>}
              </div>
              <div className="market-wizard-toggles">
                <label><input type="checkbox" checked={(draft.accepted_service_types ?? ['vps']).includes('cloud_vm')} onChange={event => update({ accepted_service_types: event.target.checked ? ['vps', 'cloud_vm'] : ['vps'] })} />Разрешить облачные VM как замену VPS</label>
                <label><input type="checkbox" checked={draft.require_complete_monthly_payment ?? false} onChange={event => update({ require_complete_monthly_payment: event.target.checked })} />Учитывать только предложения с подтверждённой полной месячной оплатой</label>
              </div>
              <button className="market-wizard-link" type="button" onClick={onOpenData}>Все данные и ограничения источников <ArrowRight size={15} /></button>
            </details>
          </>}

          {step === 4 && <>
            <header className="market-wizard-heading">
              <span className="market-wizard-kicker">ШАГ 4 ИЗ 4</span>
              <h2>Расчёт и результаты</h2>
              <p>Проверьте параметры. После расчёта появятся графики спроса, выручки, прибыли, продлений и ухода клиентов.</p>
            </header>
            <div className="market-wizard-review">
              <div><span className="market-wizard-review-icon"><MapPin size={17} /></span><span><small>Рынок и горизонт</small><strong>{draft.market === 'russia' ? 'Россия' : draft.market === 'international' ? 'Международный рынок' : 'Общий рынок'} · {city} · {draft.months} мес.</strong></span></div>
              <div><span className="market-wizard-review-icon"><Users size={17} /></span><span><small>Новые покупатели</small><strong>{draft.buyers_per_month} в месяц · seed {draft.seed}</strong></span></div>
              <div><span className="market-wizard-review-icon"><Wallet size={17} /></span><span><small>Цены и стратегии · {priceSource === 'observed' ? 'проверенный снимок' : 'ручные цены'}</small><strong>{estimate} · {strategies.map(name => strategyLabels[name] ?? name).join(', ') || 'стратегии не выбраны'}</strong></span></div>
              <div><span className="market-wizard-review-icon"><Gauge size={17} /></span><span><small>Чувствительность</small><strong>{draft.sensitivities.map(value => sensitivityLabels[value]).join(', ') || 'не выбрана'}</strong></span></div>
            </div>
            <div className="market-wizard-disclaimer"><CircleHelp size={18} /><p><strong>Сценарная оценка, а не прогноз.</strong> Результат зависит от заданных цен, бюджетов, источников и допущений. Реальное поведение покупателей и фактическую прибыльность модель не гарантирует.</p></div>
            {hasReport && <div className="market-wizard-result-status" role="status">{stale ? 'Параметры изменились — после запуска будет создан новый расчёт.' : statusText}</div>}
            {error && <div className="error-box market-error" role="alert">{error}{hasReport && ' Предыдущий успешный результат останется ниже.'}</div>}
            <div className="market-wizard-output-preview">
              <div><Server size={16} /><span><strong>Активные аренды и доля рынка</strong><small>По месяцам и выбранным стратегиям</small></span></div>
              <div><Wallet size={16} /><span><strong>Выручка, затраты и прибыль</strong><small>Сценарные финансовые показатели</small></span></div>
              <div><Users size={16} /><span><strong>Продления, переходы и отток</strong><small>Включая события к конкурентам и без аренды</small></span></div>
            </div>
          </>}
        </div>

        <aside className="market-wizard-help">
          <span className="market-wizard-help-icon"><CircleHelp size={19} /></span>
          <h3>{steps[step - 1].title}</h3>
          <p>{activeInfo}</p>
          <div className="market-wizard-help-divider" />
          <strong>Полезно знать</strong>
          <ul>
            {step === 1 && <><li>Seed позволяет повторить расчёт с теми же исходными условиями.</li><li>Рыночный снимок и сценарные цены можно различить по источнику.</li></>}
            {step === 2 && <><li>Стратегии сравниваются при одинаковом притоке покупателей.</li><li>Можно выбрать несколько уровней ценовой чувствительности.</li></>}
            {step === 3 && <><li>Бюджет и минимальная конфигурация задаются для каждого сегмента отдельно.</li><li>Сценарная ёмкость не означает фактический свободный остаток Aéza.</li></>}
            {step === 4 && <><li>Сравните итоговую прибыль и уход от базового сценария.</li><li>Детали расчёта и графики останутся доступны на этой странице.</li></>}
          </ul>
        </aside>
      </div>

      {error && step !== 4 && <div className="error-box market-error market-wizard-error" role="alert">{error}</div>}
      <footer className="market-wizard-footer">
        <button type="button" className="market-wizard-back" disabled={step === 1 || busy} onClick={() => changeStep((step - 1) as WizardStep)}><ArrowLeft size={16} />Назад</button>
        <div className="market-wizard-progress" aria-label={`Шаг ${step} из 4`}>{steps.map((item, index) => <span key={item.title} data-active={index + 1 === step || undefined} data-complete={index + 1 < step || undefined} />)}</div>
        {step < 4
          ? <button type="button" className="market-wizard-next" onClick={() => changeStep((step + 1) as WizardStep)}>Продолжить<ArrowRight size={16} /></button>
          : <button type="button" className="market-wizard-next" onClick={onCalculate} disabled={busy || !strategies.length || !draft.sensitivities.length}>{busy ? <><RefreshCw className="spin" size={16} />Считаем…</> : <>{hasReport && !stale ? 'Пересчитать' : 'Рассчитать'}<Play size={15} /></>}</button>}
      </footer>
    </section>
  );
}
