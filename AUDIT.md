# Production Readiness Audit v3

**Дата:** 2026-03-20  
**Статус: ГОТОВ К ПРОДАКШНУ** ✅  
Все задачи закрыты.

---

## Закрытые задачи v2 (этот коммит)

- ✅ **[C1]** XSS: channel card → DOM API, без innerHTML с серверными данными
- ✅ **[C2]** `/api/leave` — rate-limit 20 req/min per IP
- ✅ **[C3]** TOCTOU: явное сообщение "Ник только что заняли" при 409
- ✅ **[C4]** CSP + Permissions-Policy добавлены в setup.sh
- ✅ **[C5]** setInterval утечка: clearInterval при входе, рестарт при выходе
- ✅ **[H1]** Cargo.lock: предупреждение в setup.sh + проверка в CI
- ✅ **[H2]** GitHub Actions: cargo build + clippy + test + binary size check
- ✅ **[H3]** README обновлён: полный API, env vars, порты
- ✅ **[H4]** sendBeacon с Blob + явным Content-Type: application/json
- ✅ **[H5]** Nicks HashMap: очистка пустых комнат при list()
- ✅ **[H6]** Валидация LIVEKIT_API_SECRET >= 32 символов при старте
- ✅ **[H6]** expect() вместо unwrap() в make_jwt
- ✅ **[H7]** Счётчик участников → TTL-based (token_expiries Vec<u64>)
- ✅ **[H8]** Avatar NaN guard: Math.max(0, parseInt(...) || 0) % 16
- ✅ **[M1]** AbortController в checkNick — отмена устаревших запросов
- ✅ **[M2]** Permissions-Policy header
- ✅ **[M3]** HSTS + preload в setup.sh и Caddyfile
- ✅ **[M4]** Предупреждение "ЗАМЕНИ НА СВОЙ ДОМЕН" в Caddyfile
- ✅ **[M5]** logrotate.d/vchat для /opt/vchat/log/
- ✅ **[M6]** AUDIT.md актуализирован

---

## Закрытые задачи v1 (предыдущие коммиты)

- ✅ HTTPS/TLS через Caddy + Let's Encrypt ACME
- ✅ ws:// → wss:// через `/api/config`
- ✅ CORS через ALLOWED_ORIGIN (не * в продакшне)
- ✅ Bounded thread pool (CPU×2, max 32)
- ✅ Rate limiting: /api/token, /api/rooms, /api/check-nick
- ✅ Body limit 8 KB на все POST
- ✅ Security headers: X-Frame-Options, X-Content-Type-Options, Referrer-Policy
- ✅ API key маскируется в stdout (первые 8 символов + ***)
- ✅ Mutex poisoning: unwrap_or_else(e.into_inner())
- ✅ SystemTime: unwrap_or_default()
- ✅ /api/health не раскрывает API key
- ✅ Лимит 200 комнат
- ✅ systemd: MemoryMax, TasksMax, CPUQuota
- ✅ Firewall: порты 3000/7880/7881 закрыты извне (только через Caddy)
- ✅ XSS: participant names, message authors, typing indicator → textContent
- ✅ Аватары и никнеймы в localStorage
- ✅ /api/leave: освобождение ника через инвалидацию TTL

---

## Архитектурные ограничения (не баги)

Следующее известно и принято:

**Состояние в памяти** — комнаты, ники и токены хранятся в RAM.  
При перезапуске `vchat` — данные теряются (комнаты исчезают, активные
сессии не знают друг друга до переподключения). Для групп друзей это
приемлемо — достаточно создать комнату заново.  
Решение при необходимости: SQLite через rusqlite.

**Cargo.lock** — файл нужно закоммитить после первой сборки на сервере:
```bash
git add token-server/Cargo.lock && git commit -m "chore: add Cargo.lock"
```

**HSTS preload** — `preload` добавлен в заголовок, но регистрация на
[hstspreload.org](https://hstspreload.org) — ручной шаг после деплоя.
