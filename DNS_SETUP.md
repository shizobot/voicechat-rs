# DNS и домен — пошаговая инструкция

## Шаг 1. Получить домен

### Бесплатные домены
- **FreeDNS** (freedns.afraid.org) — бесплатные поддомены типа `vchat.mooo.com`
- **DuckDNS** (duckdns.org) — `vchat.duckdns.org`, есть скрипт для динамического IP
- **Cloudflare** — бесплатные домены `.pages.dev` только для Pages, для серверов нет

### Платные (от $1/год)
- **Namecheap** — `chat.myname.com` от $1–12/год
- **REG.RU** — российский регистратор, .ru и .com
- **Cloudflare** — по себестоимости, отличная панель DNS
- **Porkbun** — самые дешёвые новые домены

---

## Шаг 2. Настроить DNS A-запись

Зайди в панель своего регистратора/DNS-провайдера и добавь запись:

```
Тип:  A
Имя:  @          (или chat, если хочешь chat.yourdomain.com)
Значение: 1.2.3.4  ← IP твоего сервера
TTL:  300          (5 минут — для первой настройки)
```

Узнать IP сервера:
```bash
curl ifconfig.me
```

Проверить что DNS обновился (занимает 1–60 минут):
```bash
nslookup chat.example.com
# или
dig chat.example.com A
```

---

## Шаг 3. Запустить setup.sh

```bash
# PROD — домен + HTTPS (Let's Encrypt)
sudo bash setup.sh chat.example.com admin@example.com

# DEV — localhost без TLS (для тестирования)
sudo bash setup.sh localhost admin@local.dev
```

Setup.sh сам:
- Установит Caddy
- Получит TLS-сертификат от Let's Encrypt через ACME
- Настроит reverse proxy для vchat и LiveKit WebSocket
- Создаст systemd сервисы
- Откроет нужные порты

---

## Как работает Let's Encrypt (ACME)

```
Caddy запускается → видит домен в конфиге
         ↓
Отправляет запрос на acme-v02.api.letsencrypt.org
         ↓
Let's Encrypt: "докажи что ты владеешь доменом"
         ↓
Caddy создаёт временный файл на /.well-known/acme-challenge/TOKEN
         ↓
Let's Encrypt проверяет: http://chat.example.com/.well-known/...
         ↓
Домен подтверждён → выдаёт сертификат (90 дней)
         ↓
Caddy автоматически обновляет за 30 дней до истечения
```

**Важно для ACME:**
- Порт 80 и 443 должны быть открыты и доступны из интернета
- A-запись должна указывать именно на этот сервер
- Если сервер за NAT — нужен проброс портов 80 и 443

---

## Порты которые нужно открыть

| Порт  | Протокол | Для чего |
|-------|----------|----------|
| 80    | TCP | Let's Encrypt ACME challenge |
| 443   | TCP | HTTPS (Caddy) |
| 7882  | **UDP** | WebRTC голос (LiveKit) |

Порты 3000 (vchat), 7880 и 7881 (LiveKit) **закрыты** от внешнего мира — доступ только через Caddy.

---

## Проверка после установки

```bash
# Сертификат получен?
curl -v https://chat.example.com/api/health 2>&1 | grep -E "SSL|TLS|HTTP"

# LiveKit WebSocket работает?
curl -v --include \
  -H "Upgrade: websocket" \
  -H "Connection: Upgrade" \
  https://chat.example.com/ws/

# Конфиг отдаётся фронтенду?
curl https://chat.example.com/api/config
# → {"livekit_url":"wss://chat.example.com/ws","version":"1.0.0"}

# Логи Caddy
journalctl -u caddy -f

# Сертификат — срок и CN
echo | openssl s_client -connect chat.example.com:443 2>/dev/null | openssl x509 -noout -dates -subject
```

---

## DuckDNS — бесплатный домен за 2 минуты

```bash
# 1. Зарегистрируйся на duckdns.org
# 2. Добавь поддомен: vchat → ваш IP
# 3. Установи автообновление IP:

sudo apt install curl -y

TOKEN="ваш-token-с-duckdns"
DOMAIN="vchat"  # vchat.duckdns.org

# Cron каждые 5 минут обновляет IP
(crontab -l 2>/dev/null; echo "*/5 * * * * curl -s 'https://www.duckdns.org/update?domains=${DOMAIN}&token=${TOKEN}&ip=' > /tmp/duckdns.log") | crontab -

# Запускаем setup с duckdns-доменом
sudo bash setup.sh vchat.duckdns.org admin@example.com
```
