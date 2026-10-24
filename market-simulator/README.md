# Aéza Market Simulator — Rust MVP

Автономный сценарный симулятор рынка VPS/VDS с API для фронтенда. Не реализует Snapshot/Historical ML-модели. Архитектура и анализ исходного проекта: [docs/architecture.md](docs/architecture.md).

## Запуск

Требуется Rust/Cargo с edition 2024 (проверено Rust 1.95.0). Из папки market-simulator:

```sh
cargo test --locked
cargo run --locked
```

По умолчанию `127.0.0.1:8091`. `MARKET_BIND` меняет адрес. Frontend использует same-origin proxy `/api/market` → `http://127.0.0.1:8091`; пример клиента: `docs/client.ts`. Общие CORS права не включены.

```sh
curl -s http://127.0.0.1:8091/api/market/v1/example > scenario.json
curl -s -H 'Content-Type: application/json' --data-binary @scenario.json http://127.0.0.1:8091/api/market/v1/simulate
cargo run --locked -- --scenario data/example.json > result.json
# Необязательный третий аргумент CLI — собственный нормализованный каталог конкурентов:
cargo run --locked -- --scenario data/example.json data/catalog.json
```

Внешние данные для обычного запуска не нужны: снимки включены в поставку. Cargo.lock фиксирует зависимости. На этой машине также проверен полностью офлайн запуск `cargo test --offline --locked`.

## Что умеет MVP

322 наблюдаемых тарифа из VPS Price Index с CPU/RAM/disk, месячной USD ценой, доступными регионами, источником и датой. Все 533 исходные строки сохранены отдельно. Не сравниваем hourly с monthly, не конвертируем EUR и блоковые цены Akash в USD. Исключены неподходящие type, недостающие характеристики и названия с ARM/GPU; это фильтр MVP, не полная сертификация архитектуры.

Покупатели распределены по заданным сегментам. Бюджет равномерно выбирается между min/max; CPU, RAM, диск и регион — жесткие ограничения. Неизвестный регион не проходит требование конкретной локации. Один покупатель арендует максимум один сервер. Начальное состояние пустое; каждый месяц приходит новая когорта; некупившие/ушедшие могут вернуться на следующем шаге. Масштаб рынка задан сценарием.

Для каждого допустимого тарифа utility = `2 + preference_strength × stable_preference(buyer,provider) − beta × price/budget − switching_cost`, где beta = 0.5 / 2 / 5 для low/medium/high. Стоимость перехода применяется при смене провайдера. Выбирается максимальная utility выше outside option; равенство с outside означает отказ, равенство тарифов решается по стабильному ID. Это явное правило сценария, не обученная модель и не оцененная эластичность.

На продлении сначала применяется месячная вероятность ухода. Ушедший пропускает текущий период и затем может вернуться. Остальные переоценивают все тарифы, включая текущий. Текущая цена Aéza = базовая цена × multiplier стратегии; применяется ко всем ее тарифам и продлениям. Нет grandfathering. Смена тарифа внутри провайдера учитывается как продление. Каждый активный сервер оплачивает полный условный месяц, без prorating. Отдельно считаются новые аренды (включая реактивации), продления, входящие/исходящие переходы и уход во outside.

Выручка = сумма цен активных аренд. Сценарные затраты Aéza = fixed за период + variable × active; прибыль = выручка − затраты. Затраты конкурентов неизвестны: `null`. Доля рынка = active у провайдера / active всего моделируемого рынка, 0 при отсутствии аренд. Накопленная выручка не равна выручке последнего месяца. Это сценарная операционная прибыль: CAPEX, налоги, комиссии и загрузка физических машин отдельно не моделируются.

## Воспроизводимость и сравнение

В примере 4 стратегии × 3 чувствительности × 6 периодов. Все получают одинаковых покупателей, бюджеты, предпочтения и keyed random draws (SplitMix64). Смена стратегии не сдвигает случайный поток. Возвращаются seed, полный scenario, engine version, catalog SHA256 и replay SHA256. Два запуска на одинаковой сборке дают одинаковый JSON. Для долгосрочного replay храните код, Cargo.lock и data; побитовая идентичность float между разными платформами не гарантируется. Engine version меняется при изменении правил.

Это один seed, не доверительный интервал; надежность стратегии следует изучать сеткой независимых seed и сценарных параметров. Смена цены дает сценарную реакцию, не причинный прогноз реального спроса Aéza.

## Данные и ограничения

- `data/vps-price-index-current.json`, `vps-price-index-history.json`: ранее собранные снимки от 23.10.2026. Источник **VPS Price Index — vpslocate.com**, [CC-BY 4.0](https://vpslocate.com/data/). История 166 дней содержит min/median/max при меняющемся составе тарифов, не индивидуальную историю каждого плана. Она сохранена для контекста и не используется для регрессии спроса. USD значения принимаются как опубликованные индексом, включая возможную конверсию на стороне источника. Налоги, IP, CPU performance, доступность и полнота географии не подтверждены. Это прайсы, не продажи.
- `data/akash_daily_market_stats.csv`: исходный дневной ряд. Из новых lease events за апрель–сентябрь 2026 получены веса: month total / средний total этих 6 месяцев. Неполный октябрь исключен. Применение формы к VPS и масштаб 100 покупателей — допущение. По умолчанию пример использует эти веса; единичные веса дают ровный сценарий.
- Полная lease выгрузка не копируется в модуль (32 MB). `data/akash_quality.json` рассчитан потоковым проходом всех 627346 строк: ресурсы у 376212, точные UTC начала у 0. Хеш полного архива сохранен. Не переводим блоки во время, не назначаем USD цену lease и не оцениваем VPS churn по закрытиям Akash. Ресурсная выборка неполная и смещенная; ее распределение не принято за профиль VPS покупателей. Akash — рынок контейнерных ресурсов, включая GPU, а не прямой VPS аналог.
- `data/example.json`: **все Aéza тарифы — вымышленные сценарные предложения**, не реальный прайс. Реальные тарифы Aéza нужно подставить с происхождением, датой, валютой и единым налоговым базисом. Бюджеты, preference, outside, beta, churn, cost, scaling — допущения.

Пересборка из ранее подготовленных файлов:

```sh
python3 scripts/prepare_data.py /path/to/akash/outputs
```

SHA256 входов в `data/manifest.json`. Полный путь источника не нужен в runtime. Оригинальный отчет команды — пользовательский источник ограничений; фактическое качество дополнительно пересчитано ETL.

## Проверки

`cargo test --locked`: воспроизводимость, ценовой переход, sensitivity/outside, бюджеты, ресурсы, регион, churn, stock-flow identity, прибыль, равенство стратегий, внешние результаты моделей, валидация, HTTP и размер body. `cargo clippy --all-targets -- -D warnings`.

API описан в [docs/api.md](docs/api.md), OpenAPI — `docs/openapi.json` и GET `/api/market/v1/openapi.json`.


## Economic regression matrix (market-v1.1)

The economic suite covers 75 fixed-seed scenarios and 9 cross-scenario properties, both directly and through actual TCP/HTTP. It includes price changes of -40/-20/-10/0/+10/+20/+40 percent at all three sensitivity levels, buyer flows, churn, switching friction, expenses, competitor prices and empty markets. No assumption of increasing profit under discounts is imposed.

```sh
MARKET_TEST_REPORT_DIR=/tmp/aeza-economics cargo test --locked -- --nocapture
# With the regular server running:
python3 scripts/check_economics_http.py --engine-results /tmp/aeza-economics/engine-results.json --output /tmp/aeza-economics/standalone-http.json
```

The HTTP matrix binds ephemeral loopback ports; localhost network access is required. It uses the production handlers with immutable controlled catalogs via router_with_catalog; the normal application keeps its bundled catalog and HTTP callers cannot replace it. Sources for injected catalogs are labeled separately. All economic scenarios use seed 42.

Changes in v1.1: zero buyers is accepted (zero revenue and retained fixed expense); equal utility uses global plan ID ordering across every provider; JSON uses float_roundtrip to preserve f64 values across API boundaries. Floating arithmetic still has rounding error; tests use absolute 1e-7 USD or relative 1e-10 tolerance for financial identities. The version/hash change intentionally distinguishes old replays. Tests are in tests/economic_matrix.rs and tests/economic_regressions.rs.

Full economic audit report: [docs/economic-audit/report.md](docs/economic-audit/report.md).
