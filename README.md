# VoiceChat

Голосовой + текстовый групповой чат. Два процесса, ноль зависимостей в рантайме.

## Стек

| Процесс | Что делает | Технология |
|---------|-----------|------------|
| `vchat` | Веб-интерфейс + API | Rust, один статический бинарник |
| `livekit-server` | WebRTC SFU, голос, DataChannel | Go, один статический бинарник |

## API

| Метод | Путь | Тело | Ответ |
|-------|------|------|-------|
| `GET` | `/` | — | `index.html` |
| `GET` | `/api/rooms` | — | `["room1", "room2"]` |
| `POST` | `/api/rooms` | `{"name":"general"}` | `{"ok":true}` |
| `POST` | `/api/token` | `{"room":"general","username":"Маркос"}` | `{"token":"..."}` |
| `GET` | `/api/health` | — | `{"ok":true}` |

## Установка

```bash
sudo bash setup.sh
```

## Ручная сборка

```bash
cd token-server
cargo build --release
# → target/release/vchat
```

## Запуск вручную

```bash
# Переменные окружения
export LIVEKIT_API_KEY=vchat_key
export LIVEKIT_API_SECRET=$(cat /dev/urandom | tr -dc 'a-f0-9' | head -c 64)
export STATIC_DIR=./public
export PORT=3000

# Бэкенд
./target/release/vchat &

# LiveKit (отдельный терминал)
./livekit-server --config livekit.yaml
```

## Управление

```bash
systemctl status vchat livekit
journalctl -u vchat   -f    # логи бэкенда
journalctl -u livekit -f    # логи SFU

systemctl restart vchat
systemctl restart livekit
```

## Порты

| Порт | Протокол | Назначение |
|------|----------|------------|
| 3000 | TCP | Веб + API (`vchat`) |
| 7880 | TCP | LiveKit WebSocket |
| 7881 | TCP | WebRTC TCP fallback |
| **7882** | **UDP** | **WebRTC голос** |

## NAT

```bash
bash nat-diag.sh
```
