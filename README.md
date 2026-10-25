[![Rust](https://img.shields.io/badge/Rust-1.94-000000?style=for-the-badge&logo=rust&logoColor=white)](https://www.rust-lang.org/)
[![React](https://img.shields.io/badge/React-19-20232A?style=for-the-badge&logo=react&logoColor=61DAFB)](https://react.dev/)
[![TypeScript](https://img.shields.io/badge/TypeScript-5-3178C6?style=for-the-badge&logo=typescript&logoColor=white)](https://www.typescriptlang.org/)
[![PostgreSQL](https://img.shields.io/badge/PostgreSQL-Database-4169E1?style=for-the-badge&logo=postgresql&logoColor=white)](https://www.postgresql.org/)
[![Docker Compose](https://img.shields.io/badge/Docker-Compose-2496ED?style=for-the-badge&logo=docker&logoColor=white)](https://docs.docker.com/compose/)
[![OpenAPI](https://img.shields.io/badge/OpenAPI-3.0-6BA539?style=for-the-badge&logo=swagger&logoColor=white)](https://www.openapis.org/)

# Aéza Intelligence

### Аналитическая платформа для оценки риска клиентов и развития VPS-продукта

Aéza Intelligence объединяет клиентскую аналитику, модель поведения, историю цен и сценарии развития рынка в одной панели. Платформа помогает команде находить клиентов, которым стоит уделить внимание, объяснять вклад сигналов и оценивать финансовый масштаб риска.

> **Risk Score — объяснимый бизнес-индекс, а не обученная вероятность оттока.** Финансовая оценка показывает сценарий валовой выручки и не подменяет фактические потери.

## Проект в одном экране

| Возможность | Результат |
|---|---|
| Текущий риск | Балл 0–100, сегмент, факторы и карточка клиента с услугами и платежами |
| Топ клиентов | Поиск, фильтры, двунаправленная сортировка и экспорт отчётов |
| История поведения | Отклонения от личной нормы, редкие переходы состояний и необычные последовательности |
| Ценовой риск | Сопоставление реальных повышений цен с продлениями, платежами и контрольными клиентами |
| Финансовая оценка | Выручка под риском, дефицит ближайших платежей и отдельная 30-дневная сценарная оценка |
| Рыночная аналитика | Сравнение тарифов, история цен конкурентов, поисковый интерес и сценарный симулятор |
| Управление моделью | Настройки весов и порогов с сохранением версии, автора и времени изменения |
| Обновление без перезагрузки | API пересчитывает оценки и передаёт обновления интерфейсу по WebSocket |

## Архитектура

~~~mermaid
flowchart LR
    analyst[Аналитик] --> ui[React + TypeScript<br/>веб-панель · HTTPS]
    ui -->|логин и пароль| api
    ui -.->|OIDC, опционально| idp[Keycloak или внешний IdP]
    idp -.->|callback| api
    ui <-->|REST + WebSocket| api[Rust API<br/>Axum]
    ui -->|защищённый proxy| simulator[Rust Market Simulator]

    api <--> db[(PostgreSQL<br/>клиенты · услуги · транзакции<br/>настройки · история · рынок)]
    snapshots[client_snapshot<br/>ежедневный сбор] --> db
    market[market-collector<br/>тарифы · валюты] --> db
    trends[TrendsMCP<br/>поисковый интерес · опционально] --> db

    prom[Prometheus] -->|scrape /metrics| api
    db --> api
    api -->|snapshot · scores · settings| ui
~~~

### Основные компоненты

| Компонент | Роль |
|---|---|
| <code>frontend/</code> | React-панель: обзор, список клиентов, профили, история, ценовой риск, рынок и настройки |
| <code>market_api</code> | Rust/Axum API, сессии, чтение данных, расчёт Risk Score, импорт снимка и WebSocket |
| <code>client_snapshot</code> | Ежедневные снимки признаков и событий для Historical Model |
| <code>market_import</code>, <code>market_fx</code> | Загрузка рыночных тарифов и курсов валют в PostgreSQL |
| <code>search_interest</code> | Сбор поискового интереса через TrendsMCP, если задан API-ключ |
| <code>market-simulator/</code> | Изолированный сервис для сценариев спроса, конкуренции и тарифов |
| PostgreSQL | Основное хранилище; сервис <code>db</code> в Compose, данные в томе <code>db-data</code>. Внешняя база подключается через <code>DATABASE_URL</code> |
| Keycloak | Опциональный OIDC-провайдер (профиль <code>sso</code>); можно заменить внешним IdP |
| <code>config.toml</code> | Настройки приложения: адрес, режим входа, первый администратор, OIDC, демо-поток |

## Как рассчитывается риск

Модель строит оценку из доступных признаков текущего снимка. Если признак отсутствует, его вес исключается из знаменателя: нехватка данных не превращается в нулевой риск.

~~~mermaid
flowchart LR
    A[Снимок клиента] --> B[Расчёт признаков]
    B --> C{Признак доступен?}
    C -->|Да| D[Severity 0–1]
    C -->|Нет| E[Исключить из оценки]
    D --> F[Взвешенная сумма]
    E --> F
    F --> G[Нормализация доступных весов]
    G --> H[Risk Score 0–100]
    H --> I[Уровень и объяснение]
~~~

~~~text
Risk Score = round(100 × Σ(weightᵢ × severityᵢ) / Σ(weightᵢ доступных признаков))
~~~

Вес умножается на выраженность сигнала. Для покрытия платежей выраженность растёт, когда доступного баланса не хватает на ближайшие платежи; для долевых признаков используется их доля в портфеле услуг или выручке.

| Фактор | Что показывает | Вес по умолчанию |
|---|---|---:|
| Покрытие ближайших платежей | Отношение доступного баланса к сумме платежей в окне | 35 |
| Услуги без автопродления | Доля действующих услуг без автопродления | 15 |
| <code>prolong_wait</code> | Доля услуг, ожидающих оплаты | 15 |
| Неуспешные продления | Только подтверждённые исходы; <code>not_performed</code> сам по себе не доказывает отказ | 10 |
| Скоро истекающая выручка | Доля месячной выручки от услуг в окне продления | 25 |
| Ценовой риск | Реальные ценовые события при достаточном качестве данных | 0 |

По умолчанию пороги — **средний: 35**, **высокий: 65**. При недостатке признаков клиент получает отдельный статус <code>limited</code> или <code>insufficient</code>; это не то же самое, что низкий риск. Настройки модели можно менять в панели. Они сохраняются в PostgreSQL и применяются ко всем сессиям.

### Добавление признака

Признак — доля или отношение по клиенту в диапазоне 0–1 со своим весом. Каждый шаг добавляет одну запись рядом с существующими; переделывать остальной код не нужно.

1. Посчитать признак в <code>public.client_features</code>: новая миграция переопределяет функцию с ещё одной колонкой (пример — <code>migrations/112_snapshot_data_quality.up.sql</code>).
2. <code>src/model.rs</code>: поле в <code>ClientFeatures</code>.
3. <code>src/bin/market_api/main.rs</code>: вес в <code>SnapshotWeights</code> (с <code>#[serde(default)]</code>, чтобы сохранённые настройки без него оставались валидными), значение по умолчанию в <code>impl Default for SnapshotSettings</code>, вес в <code>is_valid</code> и одна строка <code>(key, label, value, weight, explanation)</code> в <code>calculate_snapshot_risk</code>. Нормализация по доступным весам сама пересчитает вклад остальных факторов.
4. <code>frontend/src</code>: вес в <code>types/index.ts</code> и <code>mocks/data.ts</code>, название и описание в <code>pages/Settings.tsx</code>, фактор в <code>features/risk/model.ts</code> (предпросмотр на странице настроек).
5. По желанию — Historical Model: <code>feature_map</code> в <code>src/bin/client_snapshot.rs</code>.

Тест <code>every_model_weight_changes_its_normalized_factor_contribution</code> собирает <code>SnapshotWeights</code> поле за полем, поэтому <code>cargo test</code> не скомпилируется, пока новый вес не добавлен и в него.

### Ожидаемые потери и денежные метрики

~~~text
Ожидаемые потери за 30 дней =
Σ по клиентам с достаточными данными
  round(месячная выручка активных и приостановленных услуг × Risk Score / 100)
~~~

Это сценарная оценка валовой выручки. Risk Score не откалиброван как вероятность оттока, поэтому результат не является статистическим прогнозом или прибылью. Выручка клиентов с неполной оценкой показывается отдельно. Дефицит ближайших платежей считается отдельно и повторно в ожидаемые потери не добавляется.

- **Выручка под риском** — месячная выручка по активным и приостановленным услугам клиентов высокого риска.
- **Дефицит платежей** — сумма нехватки баланса на предстоящие платежи.
- **Ожидаемые потери** — взвешенный Risk Score сценарий для клиентов с достаточными данными. Баланс аккаунта входит в Risk Score через покрытие ближайших платежей; на панели рядом показана сумма ближайших платежей, которую балансы не покрывают.

Все денежные значения в API хранятся целыми числами в евроцентах: **100 евроцентов = 1 евро**. Интерфейс форматирует суммы в валюту отображения.

## Данные и доменная модель

Основные таблицы источника — <code>accounts</code>, <code>services</code> и <code>transactions</code>. Они поступают как полный текущий снимок; исторические изменения статусов и даты удаления в исходном архиве отсутствуют.

| Поле | Значение |
|---|---|
| <code>balance</code> | Основной доступный баланс пользователя |
| <code>withdrawBalance</code> | Реферальный баланс, доступный для вывода |
| <code>bonusBalance</code>, <code>bonusState</code> | Бонусный баланс и состояние возможности его тратить |
| <code>totalReplenished</code> | Общая сумма пополнений за всё время |
| <code>price</code>, <code>paymentTerm</code> | Цена услуги и цикл оплаты: <code>hour</code>, <code>week</code>, <code>month</code>, <code>quarter_year</code>, <code>half_year</code>, <code>year</code> |
| <code>autoprolong</code> | Автопродление услуги |
| <code>services.status</code> | Текущее состояние услуги |
| <code>transactions.status</code> | <code>performed</code> или <code>not_performed</code> |
| <code>transactions.type</code> | <code>order</code> или <code>prolong</code> |

| Статус услуги | Доменный смысл |
|---|---|
| <code>active</code> | Услуга активна и учитывается как регулярная выручка |
| <code>suspended</code> | Услуга выключена пользователем; списания по текущей модели продолжаются |
| <code>prolong_wait</code> | Услуга ожидает оплаты после <code>expiresAt</code>; доменное окно — 5 дней |
| <code>deleted</code> | Услуга удалена клиентом или из-за неоплаты; снимок не различает причины |
| <code>blocked</code> | Услуга заблокирована, использование и продление невозможны |

<code>not_performed</code> не считается подтверждённой неудачной оплатой: в схеме источника нет окончательного результата отказа. История транзакций и текущий статус дают наблюдаемые факты, но не позволяют восстановить пропущенные переходы.

### Снимок и история

Однократный импорт содержит текущее состояние и не создаёт прошлую историю автоматически. <code>client_snapshot</code> начинает накапливать ежедневные наблюдения и новые события с момента запуска. Поэтому Historical Model становится информативнее со временем; он не может восстановить неизвестные изменения до начала сбора.

Синтетические строки — демо-поток, фикстуры модели, ценовые сценарии — всегда имеют **отрицательные ID**; по этому признаку они исключаются из финансовых метрик, <code>/metrics</code> и <code>/api/v1</code>, а <code>PUT /api/v1/source/snapshot</code> их не принимает. Домен почты ничего не значит: выданный набор <code>scripts/given/data.json</code> сам использует адреса <code>@example.test</code>, и это реальные клиенты. Сценарные повышения хранятся отдельно от услуг и транзакций и никогда не меняют реальный Risk Score, финансовые метрики или журнал операций. На странице ценового риска есть явное переключение между синтетическими сценариями и реальными наблюдениями.

## Historical Model

Historical Model сравнивает клиента с его собственной историей и ищет необычные изменения во времени.

- **Числовая норма:** статистика Welford и короткие/длинные профили с экспоненциальным затуханием.
- **Изменение привычки:** сдвиг среднего между недавним и долгосрочным окном.
- **Переходы услуг:** частота состояний и длительность ожидания, включая необычно долгий <code>prolong_wait</code>.
- **Последовательности:** личные и межклиентские последовательности событий, сравнение окон через LCS и обнаружение повторяющихся паттернов.

При стандартных настройках уверенный вывод требует минимум **10 полных наблюдений** и не менее **30 дней** истории. Период истории изменяется в настройках (7–365 дней). Необычность означает отличие от личной нормы; сама по себе она не доказывает отток или причину поведения.

## Ценовой риск и рынок

Страница ценового риска анализирует повышение цены услуги и последующее поведение: продления, расходы, изменение числа услуг и контрольную группу. Временная связь не доказывает причинность. В общий Risk Score фактор попадает только после серверной проверки объёма выборки, покрытия известных исходов и устойчивости эффекта в контрольных группах. Его вес по умолчанию равен нулю.

Для хакатонного сценария после миграций выполните <code>psql "$DATABASE_URL" -v ON_ERROR_STOP=1 -f scripts/seed_linked_price_demo.sql</code>. Скрипт повторяемо связывает 720 гипотетических повышений с существующими тестовыми клиентами и услугами, выбирает 600 клиентов без повышения в качестве контролей и сохраняет 30-дневные исходы в отдельной таблице <code>price_demo_events</code>. Повторный запуск не дублирует события. Скрипту нужно не меньше 722 клиентов с адресами <code>@example.test</code> (720 с повышением и контроли к ним); на 10 клиентах выданного набора он ничего не создаёт. API <code>/api/client/price-impact-demo</code> возвращает только эту выборку. Сценарный Risk Score в карточке клиента использует те же настраиваемые веса, но начисляет ценовой фактор лишь при достаточном числе известных исходов, контрольных пар и устойчивой связи в двух периодах. Снимок реального Risk Score при этом не меняется.

Рыночная часть включает:

- мониторинг тарифов и сохранённых изменений цены;
- сравнение только сопоставимых конфигураций, периода оплаты и известных условий;
- курсы EUR/RUB и USD/RUB для сравнения цен;
- поисковый спрос по брендам через TrendsMCP (опциональный <code>TRENDSMCP_API_KEY</code>);
- отдельный сценарный симулятор покупателей и реакции на цены.

Результат сценарного симулятора — **модельный сценарий**, а не прогноз фактической выручки. Данные рынка и поведение клиентов отображаются как разные источники наблюдений.

## Быстрый запуск

Нужен только Docker Compose: PostgreSQL входит в него.

~~~sh
cp .env.example .env
~~~

Заполни в <code>.env</code>:

- <code>POSTGRES_PASSWORD</code> — пароль базы; тот же пароль подставь в <code>DATABASE_URL</code>, по нему работают скрипты и <code>cargo test</code>;
- <code>DEFAULT_USER_PASSWORD</code> — не короче 16 символов, пароль первого администратора <code>admin</code>.

~~~sh
docker compose up -d --build
~~~

Compose поднимает PostgreSQL, применяет миграции, загружает <code>scripts/given/data.json</code>, если клиентские таблицы пусты, и запускает API, ежедневный сбор клиентских снимков, рыночный сборщик, симулятор и frontend. Открой:

- Панель: <https://127.0.0.1:8080> — вход <code>admin</code> / <code>DEFAULT_USER_PASSWORD</code>. Сертификат самоподписанный, браузер покажет предупреждение; <code>http://</code> на этом порту перенаправляется на HTTPS.
- Swagger: <https://127.0.0.1:8080/api/v1/docs>
- API напрямую: <http://127.0.0.1:3000>
- PostgreSQL: <code>127.0.0.1:5433</code>

Остановить контейнеры, оставив базу:

~~~sh
docker compose down
~~~

Удалить всё, включая базу, и поднять заново: <code>bash scripts/reset.sh</code>.

### Конфигурация

| Где | Что |
|---|---|
| <code>config.toml</code> | Настройки приложения, хранится в репозитории. Compose монтирует его в контейнеры; после изменения — <code>docker compose up -d api frontend</code>. Неизвестный ключ или неверное значение останавливает API при старте с указанием ключа |
| <code>.env</code> | Только секреты, в git не попадает |
| <code>docker-compose.yml</code> | Инфраструктура: порты (панель 8080, Keycloak 8081, база 5433) и профили <code>sso</code>, <code>demo</code> |

| Ключ <code>config.toml</code> | Назначение |
|---|---|
| <code>server.public_url</code> | Адрес панели; к нему привязаны cookie и проверка Origin. С <code>https://</code> frontend работает по HTTPS |
| <code>server.session_hours</code> | Время сессии, по умолчанию 12 часов |
| <code>auth.mode</code> | <code>password</code> (по умолчанию), <code>oidc</code> или <code>disabled</code> — только для разработки |
| <code>auth.default_user</code> | Первый администратор: создаётся при старте, пока ни у одного админа нет пароля |
| <code>auth.oidc</code> | <code>issuer</code>, <code>client_id</code>, <code>scopes</code> провайдера для <code>mode = "oidc"</code> |
| <code>demo</code> | Параметры демо-потока |

| Переменная <code>.env</code> | Назначение |
|---|---|
| <code>POSTGRES_PASSWORD</code> | Пароль сервиса <code>db</code>; читается при создании тома <code>db-data</code> |
| <code>DATABASE_URL</code> | Подключение для скриптов и тестов; другой сервер вместо сервиса <code>db</code> |
| <code>DEFAULT_USER_PASSWORD</code> | Пароль <code>auth.default_user</code> |
| <code>OIDC_CLIENT_SECRET</code> | Секрет конфиденциального OIDC-клиента, если провайдер его выдал |
| <code>KEYCLOAK_ADMIN_PASSWORD</code>, <code>SSO_USER_PASSWORD</code> | Встроенный Keycloak, только с профилем <code>sso</code> |
| <code>TRENDSMCP_API_KEY</code> | Включает плановый сбор поискового интереса |

`cargo run --bin aeza_intelligence -- config auth.mode` выводит проверенное значение из <code>config.toml</code> (без ключа — всю конфигурацию).

### Вход и пользователи

Панель и все клиентские и модельные API требуют сессию. Доступ даёт активная запись в <code>public.admins</code>; роль <code>viewer</code> только читает, <code>operator</code> и <code>superadmin</code> также меняют настройки модели и загружают снимок. Сессии хранятся в <code>public.admin_sessions</code> (только SHA-256 токена), поэтому переживают перезапуск API, а выход или <code>"isActive" = false</code> завершают их на следующем запросе.

- **Пароль** (<code>auth.mode = "password"</code>, по умолчанию). Пароли — хэши Argon2id в <code>public.admins.password_hash</code>. Новый пользователь:

  ~~~sh
  bash scripts/auth_user.sh add analyst@example.com operator "Имя Аналитика"
  ~~~

  Скрипт спросит пароль и запишет его в базу через образ API; также есть <code>role</code>, <code>passwd</code>, <code>disable</code>, <code>enable</code>, <code>list</code>.
- **OIDC** (<code>auth.mode = "oidc"</code>, опционально). По умолчанию — встроенный Keycloak: <code>docker compose --profile sso up -d</code>, realm <code>aeza</code>, пользователь <code>analyst@aeza.local</code> с паролем <code>SSO_USER_PASSWORD</code>. Адреса возврата клиента Keycloak обновляются по <code>server.public_url</code> при каждом старте. Внешний провайдер задаётся в <code>[auth.oidc]</code>; у него нужно зарегистрировать адрес возврата <code><public_url>/api/auth/callback</code>. <code>auth_user.sh</code> в этом режиме заводит пользователя в Keycloak.

Не используй <code>auth.mode = "disabled"</code> на доступном из сети сервере. Перед production-развёртыванием замени самоподписанный сертификат (<code>certs/tls.crt</code>, <code>certs/tls.key</code>), поменяй секреты и ограничь сетевой доступ к API, документации и <code>/metrics</code>.

## Импорт снимка клиентов

<code>PUT /api/v1/source/snapshot</code> принимает полный JSON-снимок из массивов <code>accounts</code>, <code>services</code> и <code>transactions</code>. Операция проверяет структуру и заменяет клиентский снимок в одной транзакции: при ошибке предыдущие данные сохраняются. Перед заменой рабочего набора сделай резервную копию.

~~~json
{
  "accounts": [
    {
      "id": 101,
      "email": "client@example.com",
      "balance": 12000,
      "withdrawBalance": 0,
      "totalReplenished": 25000,
      "bonusBalance": 0,
      "bonusState": "locked"
    }
  ],
  "services": [],
  "transactions": []
}
~~~

Денежные поля — целые числа евроцентов. Сервис должен ссылаться на существующий аккаунт, транзакция — на существующую услугу. Swagger показывает актуальные схемы и ограничения.

## API

Бизнес-эндпоинты требуют авторизованную сессию. Полная документация доступна в Swagger UI и OpenAPI JSON.

| Метод | Путь | Назначение |
|---|---|---|
| <code>POST</code> | <code>/api/v1/auth/login</code> | Вход по email и паролю (<code>auth.mode = "password"</code>) |
| <code>GET</code> | <code>/api/v1/auth/me</code> | Текущая сессия и роль |
| <code>POST</code> | <code>/api/v1/auth/logout</code> | Завершить сессию |
| <code>GET</code> | <code>/api/v1/customers</code> | Список, поиск, фильтр риска, сортировка и пагинация |
| <code>GET</code> | <code>/api/v1/customers/{id}</code> | Клиент, услуги, транзакции, Risk Score и факторы |
| <code>GET</code> | <code>/api/v1/analytics/summary</code> | Сегменты риска, выручка, дефицит и ожидаемые потери |
| <code>GET</code> | <code>/api/v1/model/settings</code> | Текущие веса, пороги и метаданные версии |
| <code>PATCH</code> | <code>/api/v1/model/settings</code> | Сохранить настройки и пересчитать скоринг |
| <code>PUT</code> | <code>/api/v1/source/snapshot</code> | Атомарно заменить клиентский снимок |
| <code>GET</code> | <code>/api/v1/docs</code> | Swagger UI |
| <code>GET</code> | <code>/api/v1/openapi.json</code> | Спецификация OpenAPI 3.0 |
| <code>GET</code> | <code>/metrics</code> | Prometheus exposition format; также <code>/metics</code>, как в тексте задания |

Пример сортировки клиентов по убыванию балла:

~~~text
GET /api/v1/customers?risk=high&sort=score&direction=desc&page=1&limit=25
~~~

Для таблицы доступны сортировки по <code>id</code>, <code>email</code>, <code>balance</code>, <code>totalReplenished</code> и <code>score</code>; направления — <code>asc</code> и <code>desc</code>.

Интерфейс использует дополнительные эндпоинты <code>/api/client/*</code> для подробной аналитики, финансового журнала, ценовых событий, ежедневной истории и WebSocket <code>/api/client/ws</code>.

> <code>/metrics</code>, Swagger UI и OpenAPI JSON доступны без пользовательской сессии, чтобы их могли читать инструменты мониторинга и документации. Ограничь их доступ на уровне сети или reverse proxy. Доступ к клиентским и модельным API защищён серверной сессией.

### Метрики Prometheus

<code>/metrics</code> публикует клиентские количества по уровням <code>high</code>, <code>medium</code>, <code>low</code>, <code>limited</code>, <code>insufficient</code>, финансовую оценку за 30 дней, MRR в оценке и вне неё, дефицит платежей и время последнего расчёта. Синтетические строки с отрицательными ID в эти метрики не входят; ID клиентов в метках нет.

## Live-обновление

API следит за изменениями <code>operation_logs</code> и обновляет состояние клиентского набора. WebSocket отправляет:

- <code>snapshot</code> — начальные данные, настройки и оценки;
- <code>dataset</code> — изменившиеся аккаунты, услуги или транзакции;
- <code>scores</code> — изменившиеся или удалённые оценки;
- <code>settings</code> — новую версию коэффициентов;
- <code>ack</code> / <code>error</code> — подтверждение запроса из интерфейса.

При изменении коэффициентов сервер сохраняет конфигурацию, пересчитывает Risk Score и передаёт новый результат открытым сессиям. Frontend восстанавливает WebSocket с паузой при временном отключении.

## Демоданные

Для отдельной тестовой базы можно включить профиль <code>demo</code>:

~~~sh
docker compose --profile demo up -d --build demo-stream
~~~

Он добавляет трёх синтетических клиентов с отрицательными ID к любой базе и раз в <code>demo.interval_seconds</code> меняет их услуги и платежи; перед этим может проиграть <code>demo.replay_days</code> дней синтетической истории для Historical Model. Реальные клиенты, финансовые метрики и <code>/metrics</code> этих строк не видят. Остановить поток:

~~~sh
docker compose stop demo-stream
~~~

> Синтетические строки остаются в базе и после остановки потока. На базе реальных клиентов лучше использовать отдельную тестовую базу.

## Структура репозитория

~~~text
.
├── src/
│   ├── bin/                 # API, ежедневные снимки, импортеры, генераторы
│   ├── adapter.rs           # PostgreSQL и клиентский снимок
│   ├── analysis.rs          # поперечная аналитика клиентов
│   ├── historical.rs        # модель личной истории
│   ├── sequences.rs         # последовательности состояний услуг
│   └── sql/                 # запросы рыночных и ценовых метрик
├── frontend/
│   └── src/
│       ├── pages/           # Dashboard, Snapshot, Historical, Market, Settings
│       └── features/        # модели риска, рынок и ценовой анализ
├── market-simulator/        # отдельный Rust-сервис сценарного анализа
├── migrations/              # миграции PostgreSQL
├── docs/openapi-v1.json     # OpenAPI 3.0
├── keycloak/                # realm и entrypoint встроенного Keycloak
├── scripts/                 # пользователи, сбор, демоданные, reset, e2e-проверки
├── tests/                   # интеграционные тесты: модель, миграции, снимок, OpenAPI
├── config.toml              # настройки приложения
├── docker-compose.yml
└── README.md
~~~

## Разработка

Тесты с базой используют <code>#[sqlx::test]</code>: каждый получает временную базу со всеми миграциями и удаляет её после себя, данные из <code>DATABASE_URL</code> не трогаются. Нужен PostgreSQL, где пользователь может создавать базы, например сервис <code>db</code>.

~~~sh
set -a; . ./.env; set +a
cargo test --locked                                   # API, модель, миграции, снимки
(cd market-simulator && cargo test --locked)          # симулятор — отдельный крейт
(cd frontend && npm ci && npm test && npm run build)
python3 -m unittest discover -s scripts/given -p 'test_*.py'   # генератор данных
~~~

GitHub Actions запускает те же проверки на каждый push и pull request (<code>.github/workflows/ci.yml</code>).

Frontend в режиме разработки — <code>cd frontend && npm run dev</code> (<http://127.0.0.1:5173>); dev-сервер проксирует <code>/api</code> и WebSocket на API на <code>127.0.0.1:3000</code> и подставляет <code>server.public_url</code> в заголовок Origin.

Проверки на работающем окружении:

~~~sh
bash scripts/check_auth.sh          # вход через настоящий Keycloak и прокси frontend
bash scripts/check_market_live.sh   # живой сбор тарифов и курсов
~~~

<code>check_auth.sh</code> запускает Keycloak, API и frontend с копией <code>config.toml</code> в режиме <code>oidc</code> и после проверки возвращает их на обычный конфиг. Правила входа — роли, Origin, сессии, деактивация — проверяет <code>cargo test</code>.
