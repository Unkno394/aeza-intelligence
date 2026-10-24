# HTTP API v1

Префикс `/api/market/v1`. JSON, UTF-8. Семантика полей и формулы — README.md. Денежные значения USD, decimal float; реальные биллинговые операции не выполняются.

| Method | Endpoint | Ответ |
|---|---|---|
| GET | /health | status, engine |
| GET | /catalog | Plan[] нормализованных тарифов конкурентов |
| GET | /sources | provenance, входные SHA256, ограничения |
| GET | /example | Scenario, пригодный для POST |
| GET | /openapi.json | OpenAPI 3.1, структуры входа |
| POST | /simulate | Report, все стратегии × sensitivities |

Request — весь Scenario (GET /example дает готовый образец). Сервер не хранит состояние: изменение цены означает новый POST с измененным `aeza_plans[].monthly_usd` или `strategies[].aeza_price_multipliers`. Одно значение multiplier на каждый месяц. API использует bundled каталог; библиотека simulate и CLI принимают свой каталог.

Report: `engine_version`, `replay_sha256`, `catalog_sha256`, `scenario`, `warnings[]`, `runs[]`. Каждый run содержит `strategy`, `sensitivity`, `months[]`, `aeza_total_revenue_usd`, `aeza_total_profit_usd`. Месяц: `month` (с 1), `arrivals`, `cumulative_buyers`, `active_leases`, `outside_buyers`, `providers` — словарь по provider. Метрики провайдера: `active_leases`, `new_leases`, `renewals`, `switches_in`, `switches_out`, `churn_to_outside`, `revenue_usd`, `market_share`, `cost_usd`, `profit_usd`. Последние два null у конкурентов. Значение provider Aéza — ровно `aeza`.

Ошибки JSON `{ "error": "..." }`: 400 malformed JSON, 413 body >128 KiB, 415 неверный Content-Type, 422 invalid DTO/сценарий, 429 заняты два worker, 500 ошибка worker. Неизвестные поля входных объектов отклоняются. Ограничения: 1–24 месяца, 0–1000 базовых покупателей/месяц, веса arrival 0–2, 1–20 сегментов и Aéza тарифов, 1–5 стратегий, 1–3 уникальные чувствительности, multiplier 0.05–20, вероятность 0–1; суммарный верхний бюджет candidate evaluations ≤50 млн. Для большей нагрузки — несколько меньших сравнений. OpenAPI описывает shape; все поведенческие ограничения проверяет сервер.

Будущий ML-вход `external_models` — максимум два элемента:

```json
{
  "slot": "snapshot",
  "schema_version": 1,
  "model_version": "team-model-v2-calibrated-v1",
  "source": "artifact://team/calibration-approved/run-123",
  "semantics": "calibrated_monthly_departure_probability",
  "monthly_departure_probability": [0.04, 0.05, 0.06, 0.05, 0.04, 0.05]
}
```

Второй slot `historical`. Массив длиной months. Пустой список использует базовое сценарное departure; один заменяет его; два усредняются. Прямое подключение сырых risk/familiarity/anomaly оценок недопустимо: требуется калибровка вне симулятора. Нет сетевых вызовов ML-сервисов; передаются готовые версионированные результаты, обеспечивающие replay.


Price audit: Month now includes aeza_effective_prices_usd, a map from plan ID to the actual monthly price taken from the same Rust prices vector used for choice and billing.

Custom strategy UI uses signed price changes: 0 means baseline, +15 means a 15% increase, -15 means a 15% decrease. The frontend sends (100 + change)/100. The HTTP API still takes multipliers: 1.15 / 0.85. A raw multiplier of 0.15 still means 15% of the base price (85% discount). No API contract reinterpretation or change to the economic choice rules.

## Competition v1.2

`POST /api/market/v1/competition`: `{scenario, strategy, month, region}`; month с 1, region строка или null. Ответ Analysis содержит полный Report, comparisons (для каждого тарифа Aéza), recommendations (максимум прибыли среди переданных стратегий отдельно по sensitivity), explanations и limitations. В comparisons всегда сохраняется полный исходный Plan с monthly_usd, ресурсами, регионами, датой и source; effective_price_usd — отдельная сценарная цена; null означает недоступность. difference_percent = (конкурент / Aéza − 1) × 100. Рейтинг только для подходящих по региону и ресурсам предложений. Ресурсы 1–2× по каждой характеристике; exact_specs — точное совпадение. Цена тарифов не является оценкой производительности.

Scenario принимает необязательный `market_events` (по умолчанию []), до 100 событий:

```json
{"plan_id":"index-318","starts_month":3,"price_multiplier":0.7,"available":true,"strategy":null}
```

Ссылки только на существующие тарифы серверного каталога. Для заданного тарифа и стратегии действует последнее наступившее событие; multiplier всегда относительно **исходной наблюдаемой** цены, не накопительный. available=false исключает предложение из выбора и billing; его нет в effective_prices_usd. До первого события тариф доступен по исходной цене. Для появления с месяца 3 задайте событие available=false с месяца 1 и true с месяца 3. Это допущение о времени доступности, не исторический факт. strategy=null действует во всех стратегиях; именованная strategy изолирует эксперимент. Смешивание глобальных и именованных событий одного тарифа запрещено. Повышение Aéza с третьего месяца по-прежнему задаётся собственной стратегией [1,1,1.3,...].

Каждый Month дополнительно содержит effective_prices_usd, plan_active_leases (для проверки выручки), flows (from/to/events/unique_buyers), до 8 примеров decisions с фактическими utility, компонентами и лучшей доступной альтернативой конкурента. Run.flows агрегирует события и независимо дедуплицирует buyer_id по направлению за весь период. Суммировать unique_buyers по разным направлениям как число людей нельзя. outside→provider включает первые аренды и повторный вход; для разделения используйте first_leases и returning_leases. Возвращение означает повторный вход **на рынок**, не обязательно к прежнему провайдеру. Прямой конкурент→Aéza — switches_in. ProviderMetrics.at_risk = активные клиенты провайдера перед началом периода; знаменатель оттока (switches_out+churn_to_outside)/at_risk. Нулевая база даёт неопределённый коэффициент, интерфейс показывает «—».

EU — явно сценарное покрытие городов ЕС (список в competition.rs/limitations), а не подтверждённый регион Aéza. Цифры площадок нормализуются; неизвестные города и отсутствующие регионы не сопоставляются с ограничением. При region=null сравнение требует пересечения известных регионов двух тарифов; ограничения покупателей берутся из Scenario.

Проверка: `cargo test --offline --locked`; запущенный сервис: `python3 scripts/check_competition_http.py --base http://127.0.0.1:8091/api/market/v1 --output /tmp/competition-audit --engine /absolute/path/to/aeza-market-simulator`. Скрипт сохраняет запросы/ответы, проверяет воспроизводимость и равенство прямому Rust-движку. Для UI новые эксперименты добавлены в существующий `scenario.test.ts` и запускаются `npm test`.

## v1.3: RUB, observed offers, runtime snapshots and capacity

Primary API `/api/market/v2`: `health`, `sources`, `catalog`, `example`, `aeza-plans`, `contexts`, `market-data`, `POST simulate`, `POST competition`. Monetary DTO fields use `_rub`: `monthly_rub`, `aeza_variable_cost_rub`, `aeza_fixed_cost_rub`, `revenue_rub`, `profit_rub`, `exact_prices_rub`, `billing_ledger_rub`, `exact_totals_rub`. Budgets are RUB. `currency=RUB`. The existing v1 USD API remains a compatibility boundary; it converts inputs into RUB before the same engine runs, and converts the response back. No competing economic implementation exists. Some internal Rust/view-model identifiers retain the old `_usd` suffix for compatibility; their native engine values are RUB and v2 renames the public keys.

All billing, cost accumulation and strategy multiplication settle in integer nano-RUB with half-even rounding (9 decimal places). Utility/preference and market shares remain floating point. Exact ledger strings are authoritative; numeric compatibility fields and UI are rounded views. Source `details` retain original amount/currency/unit, original hourly price where present, normalized monthly RUB, pinned CBR date/source/rates, components, VAT, geography, service type and availability. Monthly Aéza prices are used directly; hourly ×720 is not substituted for its separately advertised month price. FinOps configurations use the source's 720-hour convention, billable disk, one IPv4 and 100GiB internet egress; only `priced` quotes are included. GiB → GB is explicit ×1.073741824. Cloud VM is a separate service class; GPU is never inserted into a VPS catalog.

Strict competition ranking requires known compatible virtualization, CPU class, service class, region/resources and verified full payment/tax. Unknown fields produce explicit exclusion reasons, no invented KVM/tax or zero fees. `indicative_difference_percent` is an unverified list-price comparison; `difference_percent` and rank remain null for exclusions. Aéza VAT is unknown; cloud quote virtualization is unknown. Segment competitor coverage explains budget exclusions independently of the strict comparison table. Buyers can explicitly allow `cloud_vm`, unknown virtualization or incomplete payment; that scenario permission is distinct from verified direct comparability.

`market`: all/russia/international. `accepted_service_types`: vps, optionally cloud_vm. `required_virtualization`, `required_cpu_class`, `require_complete_monthly_payment` impose hard requirements. A country-wide Russia buyer accepts known Russian cities; an exact Moscow buyer does not accept an unspecified Russia location. Unknown geography never satisfies a regional requirement. Unavailable official Aéza offers cannot be sold. `aeza_capacity` has vcpu/ram_gb/storage_gb, origin and source. Default limits are explicitly scenario assumptions, not measured stock. Existing active allocations reserve resources at renewal; each buyer releases its old allocation before reconsidering, then commits its chosen allocation. Stable buyer-ID order resolves competition for scarce capacity. Capacity-filtered candidates are counted, not equated to lost unique buyers.

`MARKET_DATA_PATH` points to an atomic envelope JSON `{payload: <JSON string>, sha256: <SHA256 of UTF-8 payload>}`. Each request reads and validates it; bad/missing files retain the last verified snapshot in memory. An initially absent file uses the verified bundled snapshot. No HTTP file-write endpoint, no network downloads during a simulation. Snapshots retain original source receipts and dates. `--check-data path` validates a snapshot offline. Replaying after market updates requires the same saved snapshot as well as seed/scenario. Arbitrary FX supplied by HTTP clients is rejected if it differs from the verified dataset. CBR reference conversion does not predict a payment processor's rate.

Refresh without recompilation:

```sh
python3 market-simulator/scripts/refresh_market_data.py --raw-dir /tmp/aeza-source-receipts
# Supplemental GPU Parquet ingestion requires Python >=3.10 and pyarrow==26.0.0:
python3 market-simulator/scripts/refresh_market_context.py --snapshot market-simulator/data/market-snapshot.json --raw-dir /tmp/aeza-context-receipts
```

A failed primary refresh never replaces the last snapshot. Context refresh retains prior per-source data on failure. Source VPS Price Index `current.json` currently returns HTTP403; its verified 322-offer archive remains. GPU source is timestamped S3 Parquet (CC-BY4.0), with a diagnostic sample of up to five listings per provider. Spot and on-demand stay separate. Akash supplies only a relative arrival profile, not VPS tariffs, elasticity, churn or Aéza capacity. The historical joined CSV's implausible provider-count column is excluded. The context API also records discovery via public-apis and the primary CBR choice.

Verification: `cargo test --offline --locked`, `python3 scripts/check_rub_http.py --base http://127.0.0.1:8091/api/market/v2 --output /tmp/aeza-rub-audit`, existing `npm test` and production build. No ML models were modified or called.

## Buyer behavior: RUB v2

POST `/api/market/v2/behavior` with `{scenario, strategy, sensitivity}`. Use a complete RUB v2 scenario and an existing strategy name. The server evaluates control, its prices times 0.8, 0.9 and 1.15. Cohorts, seed, costs, market snapshot and competitor events stay fixed; strategy-specific events are copied to each replay. Aggregate workload limits and trusted FX checks apply. Reduce buyers/months if the four-run workload exceeds the limit.

Returns `report`, `candidates` (exact profit ranking, price schedules in report, segment effects), `recommended_strategy`, `recommendation`, `diagnostic_actions`, `limitations`. Control wins unless another candidate strictly increases profit. This is a bounded scenario search, not a global optimum or a forecast.

`months[].buyer_behavior` is a census of every decision grouped by stable `segment_index`. `loss_rate` divides Aéza losses by the opening Aéza client base; zero denominator returns null. The mutually exclusive diagnostic funnel is policy/availability, resources, region, budget, capacity, utility. Exogenous departures have unknown causes and never receive a price explanation. Equal-price competitor diagnostics are local utility comparisons, not longitudinal causal estimates.

`behavior_unique_clients` counts distinct buyers per reason for the entire run. Reasons and flow directions have overlapping client sets, so unique counts cannot be summed. Monthly counts and lease-months are events. `market_returns` means re-entry into the rental market, not necessarily return of a former Aéza client. Non-price diagnostic actions (resources, regions, capacity) are explicitly unverified; investment costs are unknown. Only price actions are replay-verified.

All v2 money is RUB; `exact_*_rub` fields contain decimal strings with nine places. Negative profits are supported. The control monthly report matches a focused direct `/simulate` call. Run live audit: `python3 scripts/check_behavior_http.py --base http://127.0.0.1:8091/api/market/v2 --output /tmp/buyer-audit`.

## Statistics: 100 paired seeds (RUB v2)

`POST /api/market/v2/statistics` with `{scenario, sensitivity}`. Runs exactly seed 0..99 at one requested sensitivity; the scenario's seed is ignored and recorded as 0 in the normalized batch input. All other inputs, strategies, FX and the loaded catalog are frozen for the whole batch. Existing simulate/competition/behavior semantics are unchanged.

Returns observed profit min/max, median, linearly interpolated quantiles (2.5%, 25%, 75%, 97.5%), sample standard deviation (denominator 99), loss runs (strictly negative exact profit), leadership counts and per-seed points. Mean profit is not exposed. Exact profit strings preserve integer nano-RUB accounting. Risk and top-frequency confidence intervals use two-sided 95% Wilson score. Tied maxima receive separate tied counts, not sole wins; top frequencies can exceed 100% when summed across tied strategies. Empirical ranges/quantiles are not confidence intervals of mean or real future profit.

The existing per-run workload validation applies, plus a 750 million candidate evaluation bound for the whole batch. One concurrent statistics batch and the shared simulation worker limit apply; 429 indicates busy workers. 422 indicates invalid inputs, mismatched FX or excessive workload. A disconnected client may leave the bounded worker finishing its batch. No precomputed observations or fabricated fallback results are returned.
