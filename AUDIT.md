# Production Readiness Audit v2

**Дата:** 2026-03-20  
**Статус: НЕ ГОТОВ К ПРОДАКШНУ**  
Осталось закрыть **19 задач** (5 критических, 8 высоких, 6 средних/низких).

---

## КРИТИЧНО — блокируют продакшн

### [C1] XSS: имя канала из сервера вставляется в `innerHTML` без escaping
**Файл:** `public/index.html`, строка ~471  
`card.innerHTML = \`...<div class="ch-name">${name}</div>...\``  
Если злоумышленник создаст канал с именем `<img src=x onerror=alert(1)>` — XSS у всех.  
**Фикс:** заменить на `textContent` / DOM API для `ch-name` и `ch-meta`.

### [C2] `/api/leave` без аутентификации и rate-limit
Любой может POST `/api/leave {room: "general", username: "alice"}` и:
- освободить чужой ник немедленно (username hijacking)
- декрементировать счётчик участников в минус (счётчик ломается)
- спамить запросами (нет rate-limit)  
**Фикс:** добавить rate-limit 20 req/min per IP; проверять ownership через нерандомный токен при leave, либо убрать leave-endpoint и полагаться на TTL.

### [C3] TOCTOU race condition: `check-nick` → `token`
Между `POST /api/check-nick` (возвращает `available:true`) и `POST /api/token` нет атомарности.  
Два пользователя могут одновременно получить `available:true` для одного ника, затем первый успешно войдёт (409 второму). UI обрабатывает 409 корректно, но пользователь не понимает почему — нужно явное сообщение "Ник только что заняли, выбери другой".

### [C4] CSP отсутствует в `setup.sh` (генерируемом Caddyfile)
`caddy/Caddyfile` содержит CSP, но `setup.sh` генерирует Caddyfile без него — реальный деплой идёт через `setup.sh`. Оба файла должны быть идентичны.  
**Фикс:** добавить CSP в секцию `header {}` в `setup.sh`.

### [C5] `setInterval(loadChannels, 15000)` не очищается при входе в комнату
Когда пользователь вошёл в комнату, опрос каналов продолжается каждые 15 секунд — лишние запросы, утечка таймера. При нескольких переходах lobby→room→lobby таймеры накапливаются.  
**Фикс:** `clearInterval` при входе и выходе из комнаты.

---

## ВЫСОКИЙ ПРИОРИТЕТ

### [H1] `Cargo.lock` не в репозитории
`.gitignore` не исключает `Cargo.lock`, но файл не закоммичен. Для бинарных приложений `Cargo.lock` **обязателен** в git — без него `cargo build` может подтянуть несовместимую версию зависимости.  
**Фикс:** `git add Cargo.lock && git commit`.

### [H2] Нет GitHub Actions CI
Нет автосборки при пуше. Любой коммит с синтаксической ошибкой Rust попадает в репо незамеченным.  
**Фикс:** `.github/workflows/ci.yml` с `cargo build --release` + `cargo test` + `cargo clippy`.

### [H3] README устарел
API в README не совпадает с реальным — нет `/api/check-nick`, `/api/leave`, `/api/config`, `/api/rooms` возвращает `[{name,count}]` а не `["name"]`, `/api/token` требует `avatar`.  
**Фикс:** обновить README.md.

### [H4] `sendBeacon` отправляет JSON как `text/plain`
`navigator.sendBeacon('/api/leave', JSON.stringify(...))` отправляет тело без `Content-Type: application/json`.  
Backend парсит JSON из тела независимо от Content-Type — сейчас работает, но это неявная зависимость от поведения `serde_json`.  
**Фикс:** использовать `Blob` с явным Content-Type: `new Blob([JSON.stringify(...)], {type:'application/json'})`.

### [H5] Nicks `HashMap` не чистит пустые комнаты
`Nicks.map` накапливает пустые `HashMap<String, (u64,u8)>` для удалённых комнат. При большом числе каналов — утечка памяти.  
**Фикс:** в `Rooms::list()` при удалении старых комнат удалять соответствующие записи из `Nicks.map`.

### [H6] `unwrap()` в `make_jwt` и заголовках HTTP
`HmacSha256::new_from_slice(...).unwrap()` — паникует если секрет пустой (теоретически возможно при пустом `LIVEKIT_API_SECRET`). Аналогично `Header::from_bytes(...).unwrap()` — panic на невалидном значении заголовка.  
**Фикс:** `expect("secret must not be empty")` + валидация секрета при старте.

### [H7] Счётчик участников `count` недостоверен
`rooms.inc()` вызывается при выдаче токена, `rooms.dec()` — только при `/api/leave`. Если пользователь закрыл вкладку без `beforeunload` (crash, kill -9, потеря сети) — счётчик не уменьшится и "зависнет".  
**Фикс:** сверять счётчик с реальными данными LiveKit Server API (`GET /twirp/livekit.RoomService/ListParticipants`), или использовать TTL-счётчик вместо inc/dec.

### [H8] Нет валидации `avatar` диапазона на клиенте
`localStorage.getItem(LS_AV)` возвращает строку — `parseInt` может вернуть `NaN` или отрицательное число. `NaN % 16 === NaN`, что ломает `AV_COLORS[NaN]` → `undefined`.  
**Фикс:** `Math.max(0, parseInt(...) || 0) % 16`.

---

## СРЕДНИЙ / НИЗКИЙ ПРИОРИТЕТ

### [M1] `fetch('/api/check-nick')` без AbortController
Быстрый ввод ника создаёт гонку запросов — последний ответ может прийти раньше предпоследнего, UI покажет устаревший статус.  
**Фикс:** AbortController с отменой предыдущего запроса при каждом новом вызове `checkNick`.

### [M2] Нет `Permissions-Policy` заголовка
Не ограничены браузерные API (camera, geolocation, payment).  
**Фикс:** добавить в Caddyfile: `Permissions-Policy "camera=(), geolocation=(), payment=()"`.

### [M3] `HSTS` без `preload`
`Strict-Transport-Security: max-age=31536000; includeSubDomains` — хорошо, но для максимальной защиты нужен `preload` и регистрация в hstspreload.org.

### [M4] `caddy/Caddyfile` содержит `chat.example.com` — заглушка в репо
Пользователь может случайно задеплоить без замены домена.  
**Фикс:** добавить в файл явный комментарий `# ЗАМЕНИ НА СВОЙ ДОМЕН` + проверку в `setup.sh`.

### [M5] Нет `logrotate` конфига для `/opt/vchat/log/`
Caddy логи ротируются через сам Caddy, но `vchat.log` и `livekit.log` — нет.  
**Фикс:** добавить `/etc/logrotate.d/vchat`.

### [M6] AUDIT.md устарел (v1)
Содержит задачи из прошлого аудита, часть уже закрыта.  
**Фикс:** этот файл заменяет предыдущий.

---

## Закрытые задачи (с прошлого аудита)

- ✅ HTTPS/TLS через Caddy с Let's Encrypt
- ✅ ws:// → wss:// через `/api/config`  
- ✅ CORS через `ALLOWED_ORIGIN` (не `*` в продакшне)
- ✅ Unbounded thread pool → bounded (CPU×2, max 32)
- ✅ Rate limiting на `/api/token`, `/api/rooms`, `/api/check-nick`
- ✅ Body limit 8KB
- ✅ Security headers: X-Frame-Options, X-Content-Type-Options, Referrer-Policy
- ✅ API key маскируется в логах
- ✅ Mutex poisoning: `unwrap_or_else(e.into_inner())`
- ✅ `SystemTime::unwrap` → `unwrap_or_default`
- ✅ `/api/health` не раскрывает API key
- ✅ Лимит 200 комнат
- ✅ MemoryMax, TasksMax, CPUQuota в systemd
- ✅ Порты 3000/7880/7881 закрыты в продакшне (только через Caddy)
- ✅ XSS: participant names, authors, typing indicator → textContent
- ✅ Аватары и никнеймы сохраняются в localStorage
