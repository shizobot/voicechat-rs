#!/usr/bin/env bash
# =====================================================================
#  setup.sh — VoiceChat: Rust + LiveKit + Caddy (TLS) + systemd
#  sudo bash setup.sh chat.example.com admin@example.com
# =====================================================================
set -euo pipefail

RED='\033[0;31m'; GRN='\033[0;32m'; YEL='\033[1;33m'
BLU='\033[0;34m'; WHT='\033[1;37m'; NC='\033[0m'
ok()   { echo -e "${GRN}✓${NC} $*"; }
fail() { echo -e "${RED}✗${NC} $*"; exit 1; }
info() { echo -e "${BLU}→${NC} $*"; }
warn() { echo -e "${YEL}⚠${NC} $*"; }
hdr()  { echo -e "\n${WHT}━━ $* ━━${NC}"; }

[[ $EUID -eq 0 ]] || fail "Запусти с sudo"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="/opt/vchat"
LIVEKIT_VER="1.8.2"
REAL_USER="${SUDO_USER:-$USER}"
REAL_HOME=$(getent passwd "$REAL_USER" | cut -d: -f6)

DOMAIN="${1:-}"
EMAIL="${2:-}"

if [[ -z "$DOMAIN" || -z "$EMAIL" ]]; then
  echo "Использование: sudo bash setup.sh <домен> <email>"
  echo "Пример:        sudo bash setup.sh chat.example.com admin@example.com"
  echo "Dev-режим:     sudo bash setup.sh localhost admin@local.dev"
  exit 1
fi

DEV_MODE=0
[[ "$DOMAIN" == "localhost" || "$DOMAIN" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] && DEV_MODE=1

# ── Cargo ────────────────────────────────────────────────────
hdr "Rust"
CARGO=""
for p in "$REAL_HOME/.cargo/bin/cargo" /usr/local/bin/cargo /usr/bin/cargo; do
  [[ -x "$p" ]] && { CARGO="$p"; break; }
done
[[ -n "$CARGO" ]] || fail "Cargo не найден. Установи: curl https://sh.rustup.rs | sh"
ok "$(sudo -u "$REAL_USER" "$CARGO" --version 2>/dev/null || "$CARGO" --version)"

# ── Сборка ───────────────────────────────────────────────────
hdr "Сборка vchat"
pushd "$SCRIPT_DIR/token-server" > /dev/null
sudo -u "$REAL_USER" "$CARGO" build --release 2>&1 | grep -E "^error|Compiling vchat|Finished" || true
[[ -f "target/release/vchat" ]] || fail "Сборка не удалась"
ok "$(du -sh target/release/vchat | cut -f1)"
popd > /dev/null

# ── LiveKit ──────────────────────────────────────────────────
hdr "LiveKit v${LIVEKIT_VER}"
mkdir -p "$INSTALL_DIR"/{bin,log,public}
if [[ ! -f "$INSTALL_DIR/bin/livekit-server" ]]; then
  ARCH=$(uname -m); case "$ARCH" in x86_64) LA="amd64";; aarch64) LA="arm64";; *) fail "Неизвестная архитектура";; esac
  TMP=$(mktemp -d)
  curl -L --progress-bar "https://github.com/livekit/livekit/releases/download/v${LIVEKIT_VER}/livekit_${LIVEKIT_VER}_linux_${LA}.tar.gz" | tar xz -C "$TMP"
  install -m 755 "$TMP/livekit-server" "$INSTALL_DIR/bin/livekit-server"
  rm -rf "$TMP"; ok "livekit-server установлен"
else
  ok "Уже установлен"
fi

# ── Caddy ────────────────────────────────────────────────────
hdr "Caddy"
if ! command -v caddy &>/dev/null; then
  info "Устанавливаем Caddy..."
  apt-get install -y debian-keyring debian-archive-keyring apt-transport-https -q 2>/dev/null || true
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/gpg.key' | gpg --dearmor -o /usr/share/keyrings/caddy-stable-archive-keyring.gpg 2>/dev/null || true
  curl -1sLf 'https://dl.cloudsmith.io/public/caddy/stable/debian.deb.txt' | tee /etc/apt/sources.list.d/caddy-stable.list > /dev/null
  apt-get update -q && apt-get install caddy -y -q
fi
ok "Caddy $(caddy version)"

# ── Файлы ────────────────────────────────────────────────────
hdr "Файлы"
install -m 755 "$SCRIPT_DIR/token-server/target/release/vchat" "$INSTALL_DIR/bin/vchat"
cp -r "$SCRIPT_DIR/public/"* "$INSTALL_DIR/public/"
ok "Скопированы"

# ── Секрет ───────────────────────────────────────────────────
hdr "Ключ"
SECRET_FILE="$INSTALL_DIR/secret"
if [[ ! -f "$SECRET_FILE" ]]; then
  tr -dc 'a-f0-9' < /dev/urandom | head -c 64 > "$SECRET_FILE"
  chmod 600 "$SECRET_FILE"; ok "Сгенерирован"
else
  ok "Существующий"
fi
SECRET=$(cat "$SECRET_FILE")

# ── LiveKit конфиг ───────────────────────────────────────────
hdr "LiveKit конфиг"
PUBLIC_IP=$(curl -s --max-time 4 https://api4.my-ip.io/ip 2>/dev/null || curl -s --max-time 4 https://ifconfig.me 2>/dev/null || echo "")
LOCAL_IP=$(ip route get 8.8.8.8 2>/dev/null | awk '/src/{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' | head -1 || echo "")

cat > "$INSTALL_DIR/livekit.yaml" << YAML
port: 7880
rtc:
  tcp_port: 7881
  udp_port: 7882
  use_external_ip: true
$([ "${PUBLIC_IP:-}" != "${LOCAL_IP:-}" ] && [ -n "${PUBLIC_IP:-}" ] && echo "  external_ip: ${PUBLIC_IP}" || true)
keys:
  vchat_key: ${SECRET}
logging:
  level: warn
  json: false
room:
  max_participants: 20
  empty_timeout: 300s
YAML
ok "livekit.yaml"

# ── Caddyfile ────────────────────────────────────────────────
hdr "Caddyfile"
mkdir -p /var/log/caddy
chown caddy:caddy /var/log/caddy 2>/dev/null || true

if [[ $DEV_MODE -eq 1 ]]; then
  LIVEKIT_WS_URL="ws://${DOMAIN}:7880"
  cat > /etc/caddy/Caddyfile << CADDYEOF
http://${DOMAIN} {
    reverse_proxy /ws/* 127.0.0.1:7880 {
        header_up Connection {>Connection}
        header_up Upgrade {>Upgrade}
    }
    reverse_proxy /* 127.0.0.1:3000
    encode gzip
}
CADDYEOF
else
  LIVEKIT_WS_URL="wss://${DOMAIN}/ws"
  cat > /etc/caddy/Caddyfile << CADDYEOF
{
    email ${EMAIL}
    admin off
}

${DOMAIN} {
    reverse_proxy /ws/* 127.0.0.1:7880 {
        header_up Host {upstream_hostport}
        header_up Connection {>Connection}
        header_up Upgrade {>Upgrade}
    }
    reverse_proxy /* 127.0.0.1:3000

    header {
        X-Frame-Options "DENY"
        X-Content-Type-Options "nosniff"
        Referrer-Policy "strict-origin-when-cross-origin"
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        Content-Security-Policy "default-src 'self'; script-src 'self' https://cdn.jsdelivr.net; connect-src 'self' wss://${DOMAIN}; media-src 'self' blob:; img-src 'self' data:"
        Permissions-Policy "camera=(), geolocation=(), payment=()"
        -Server
    }

    encode gzip

    log {
        output file /var/log/caddy/vchat.log {
            roll_size 10mb
            roll_keep 5
        }
    }
}
CADDYEOF
fi

caddy validate --config /etc/caddy/Caddyfile && ok "Caddyfile валиден"

# ── systemd ──────────────────────────────────────────────────
hdr "systemd"

cat > /etc/systemd/system/vchat.service << UNIT
[Unit]
Description=VoiceChat API (Rust)
After=network.target

[Service]
Type=simple
User=nobody
Group=nogroup
Environment=LIVEKIT_API_KEY=vchat_key
Environment=LIVEKIT_API_SECRET=${SECRET}
Environment=LIVEKIT_URL=${LIVEKIT_WS_URL}
Environment=ALLOWED_ORIGIN=https://${DOMAIN}
Environment=HOST=127.0.0.1
Environment=PORT=3000
Environment=STATIC_DIR=${INSTALL_DIR}/public
ExecStart=${INSTALL_DIR}/bin/vchat
WorkingDirectory=${INSTALL_DIR}
Restart=always
RestartSec=2
StandardOutput=append:${INSTALL_DIR}/log/vchat.log
StandardError=append:${INSTALL_DIR}/log/vchat.log
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ReadWritePaths=${INSTALL_DIR}/log
MemoryMax=256M
TasksMax=512
CPUQuota=80%

[Install]
WantedBy=multi-user.target
UNIT

cat > /etc/systemd/system/livekit.service << UNIT
[Unit]
Description=LiveKit SFU
After=network.target

[Service]
Type=simple
User=nobody
Group=nogroup
ExecStart=${INSTALL_DIR}/bin/livekit-server --config ${INSTALL_DIR}/livekit.yaml
WorkingDirectory=${INSTALL_DIR}
Restart=always
RestartSec=3
StandardOutput=append:${INSTALL_DIR}/log/livekit.log
StandardError=append:${INSTALL_DIR}/log/livekit.log
LimitNOFILE=65536
NoNewPrivileges=yes
PrivateTmp=yes
ProtectSystem=strict
ReadWritePaths=${INSTALL_DIR}/log
MemoryMax=512M

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now vchat livekit caddy
ok "Все сервисы запущены"

# ── Firewall ─────────────────────────────────────────────────
hdr "Firewall"
ufw_rule() {
  command -v ufw &>/dev/null && ufw status 2>/dev/null | grep -q active && ufw "$@" 2>/dev/null || true
}
ufw_rule allow 80/tcp  comment "HTTP (ACME)"
ufw_rule allow 443/tcp comment "HTTPS"
ufw_rule allow 7882/udp comment "WebRTC UDP"
if [[ $DEV_MODE -eq 0 ]]; then
  ufw_rule deny 3000 2>/dev/null || true
  ufw_rule deny 7880 2>/dev/null || true
  ufw_rule deny 7881 2>/dev/null || true
fi
ok "Порты настроены"

# ── Итог ─────────────────────────────────────────────────────
echo ""
echo -e "${WHT}══════════════════════════════════════${NC}"
echo -e "${GRN}  Готово!${NC}"
echo -e "${WHT}══════════════════════════════════════${NC}"
if [[ $DEV_MODE -eq 1 ]]; then
  echo -e "  http://${DOMAIN}"
else
  echo -e "  ${GRN}https://${DOMAIN}${NC}  (TLS автоматически)"
  echo ""
  echo -e "${YEL}  DNS: A-запись должна указывать на этот сервер:${NC}"
  echo -e "  ${DOMAIN}  →  A  →  ${PUBLIC_IP:-$(curl -s --max-time 3 https://ifconfig.me 2>/dev/null || echo '<IP>')}"
fi
echo ""
echo "  journalctl -u vchat   -f"
echo "  journalctl -u livekit -f"
echo "  journalctl -u caddy   -f"
echo ""
