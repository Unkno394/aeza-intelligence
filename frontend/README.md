# Aéza Intelligence frontend

React 19, TypeScript, Vite, Tailwind CSS, Recharts и React Router.

## Запуск

Сначала запустите PostgreSQL и Rust API из корня проекта (`cargo run --bin market_api`), затем:

```sh
cd frontend
npm install
npm run dev
```

Vite слушает `http://127.0.0.1:5173` и проксирует `/api` на `127.0.0.1:3000`.
Проверка сборки: `npm run build`. Тест модели: `npm test`.

## Данные

Dashboard и Snapshot получают аккаунты, услуги и транзакции из
`GET /api/client/dataset`. ID преобразованы в строки для URL, суммы передаются
в евроцентах, цены услуг приведены к месяцу. Клиентские данные доступны только
для чтения: изменение баланса и автопродления отключено. Личные настройки
эвристического Risk Score хранятся в `localStorage` (`aeza-settings-v1`).

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
