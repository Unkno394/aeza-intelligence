# Aéza Intelligence frontend

React 19, TypeScript, Vite, Tailwind CSS, Recharts и React Router.

## Запуск

Сначала запустите PostgreSQL и Rust API из корня проекта (`cargo run --bin market_api`), затем:

```sh
cd frontend
npm install
npm run dev
```

Vite слушает `http://127.0.0.1:5173` и проксирует `/api` (включая WebSocket
`/api/client/ws`) на `127.0.0.1:3000`.
Проверка сборки: `npm run build`. Тест модели: `npm test`.

## Данные

Dashboard, Snapshot и Settings работают через WebSocket `/api/client/ws`
(`src/services/live.ts`, протокол описан в корневом README). При подключении
сервер присылает аккаунты, услуги, транзакции, настройки и Risk Score, затем —
только изменения: новые данные, изменившиеся score и настройки. Settings
отправляет изменения настроек в тот же канал; они хранятся на сервере и
применяются во всех открытых вкладках. Индикатор Live/Offline в шапке
показывает состояние канала; при обрыве клиент переподключается сам.
ID преобразованы в строки для URL, суммы передаются в евроцентах, цены услуг
приведены к месяцу. Клиентские данные доступны только для чтения: изменение
баланса и автопродления отключено.

Snapshot дополнительно показывает отклонения из Rust
`GET /api/client/analysis`. Его индекс Risk Score остаётся отдельной
объяснимой эвристикой браузера, а расстояние Махаланобиса Rust не является
вероятностью оттока.

Historical читает ежедневные снимки из
`GET /api/client/historical/{id}`. Снимки создаёт Rust-команда
`cargo run --bin client_snapshot`; с `--schedule` она проверяет данные каждый
час и сохраняет не больше одного снимка на клиента за UTC-день.

Market Intelligence читает поисковый интерес, тарифы и географию из трёх
маршрутов `/api/market/*`. Временной ряд TrendsMCP и география из совместного
Google Trends CSV имеют разные источники; карта остаётся пустой до импорта CSV.

Файлы `mocks/` оставлены для автономных тестов фронтенд-модели. В рабочем
приложении `useAnalyzer` использует `HttpApi`, а не `mockApi`.

## Market Simulator

Страница `/market-simulator` работает с самостоятельным Rust Market API.
Запустите его из корня проекта:
`cargo run --locked --manifest-path market-simulator/Cargo.toml`.
Vite проксирует `/api/market/v1` и `/api/market/v2` на `127.0.0.1:8091`
(dev и preview), остальные `/api/*` идут в `market_api`. В production нужен
такой же reverse proxy и SPA fallback.

Страница загружает сценарий и каталог через API; результаты появляются только после POST /simulate. После изменения настроек старый отчет помечается как предыдущий. Ошибки API не заменяются демонстрационными ответами. Прибыль, бюджеты, отток и тарифы Aéza обозначены как сценарные; существующие Snapshot/Historical модели не подключены и не изменены. `npm test` включает тесты canonical сравнения сценариев.

Стили симулятора лежат в `src/pages/market.css` и `src/features/market/competition.css`.
Корневые классы у него `simulator-page`, `simulator-chart`, `simulator-state`:
одноимённые `market-*` уже заняты страницей Market Intelligence в `index.css`.
