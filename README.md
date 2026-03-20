# VoiceChat-RS

Голосовой + текстовый групповой чат для своих. Два статических бинарника, никакого Docker.

## Стек

| Процесс | Описание | Порт |
|---------|----------|------|
| `vchat` | Rust: HTTP API + статика | 3000 (внутренний) |
| `livekit-server` | Go: WebRTC SFU, голос, DataChannel | 7880 WS, 7882 UDP |
| `caddy` | TLS termination + reverse proxy | 443 |

## Быстрый старт

```bash
sudo bash setup.sh chat.example.com admin@example.com
```

Dev-режим (без TLS):
```bash
sudo bash setup.sh localhost dev@local.dev
```

## API

| Метод | Путь | Тело | Ответ |
|-------|------|------|-------|
| `GET` | `/` | — | index.html |
| `GET` | `/api/config` | — | `{livekit_url, version}` |
| `GET` | `/api/rooms` | — | `[{name, count}]` |
| `POST` | `/api/rooms` | `{name}` | `{ok}` |
| `POST` | `/api/check-nick` | `{room, username}` | `{available}` |
| `POST` | `/api/token` | `{room, username, avatar}` | `{token}` |
| `POST` | `/api/leave` | `{room, username}` | 204 |
| `GET` | `/api/health` | — | `{ok}` |

## Переменные окружения

| Переменная | Обязательная | Описание |
|------------|-------------|----------|
| `LIVEKIT_API_SECRET` | ✅ Да (≥32 символов) | Секрет для подписи JWT |
| `LIVEKIT_API_KEY` | Нет | API ключ (default: vchat_key) |
| `LIVEKIT_URL` | Нет | ws:// или wss:// URL LiveKit |
| `ALLOWED_ORIGIN` | Нет | CORS origin (default: *) |
| `STATIC_DIR` | Нет | Путь к public/ (default: ./public) |
| `PORT` | Нет | HTTP порт (default: 3000) |
| `HOST` | Нет | Bind address (default: 127.0.0.1) |

## Управление

```bash
systemctl status vchat livekit caddy
journalctl -u vchat   -f
journalctl -u livekit -f
journalctl -u caddy   -f
systemctl restart vchat
```

## Порты (продакшн)

| Порт | Протокол | Открыт? | Назначение |
|------|----------|---------|------------|
| 80 | TCP | ✅ | Let's Encrypt ACME |
| 443 | TCP | ✅ | HTTPS (Caddy) |
| 7882 | **UDP** | ✅ | WebRTC голос |
| 3000 | TCP | ❌ | vchat (только localhost) |
| 7880 | TCP | ❌ | LiveKit WS (только localhost) |
| 7881 | TCP | ❌ | WebRTC TCP (только localhost) |

## NAT и домен

```bash
bash nat-diag.sh     # диагностика NAT
cat DNS_SETUP.md     # инструкция по домену
```
