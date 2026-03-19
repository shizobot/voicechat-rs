#!/usr/bin/env bash
# =============================================================
#  setup.sh — VoiceChat: Rust backend + LiveKit + systemd
#  Без Docker, без Node.js, без nginx — всё в одном бинарнике
#  sudo bash setup.sh
# =============================================================
set -euo pipefail

RED='\033[0;31m'; GRN='\033[0;32m'; YEL='\033[1;33m'
BLU='\033[0;34m'; WHT='\033[1;37m'; NC='\033[0m'
ok()   { echo -e "${GRN}✓${NC} $*"; }
fail() { echo -e "${RED}✗${NC} $*"; exit 1; }
info() { echo -e "${BLU}→${NC} $*"; }
warn() { echo -e "${YEL}⚠${NC} $*"; }
hdr()  { echo -e "\n${WHT}▶ $*${NC}"; }

[[ $EUID -eq 0 ]] || fail "Нужен root: sudo bash setup.sh"

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
INSTALL_DIR="/opt/vchat"
LIVEKIT_VER="1.8.2"
REAL_USER="${SUDO_USER:-$USER}"
REAL_HOME=$(getent passwd "$REAL_USER" | cut -d: -f6)

# ── Cargo ─────────────────────────────────────────────────────
hdr "Rust toolchain"
CARGO=""
for p in "$REAL_HOME/.cargo/bin/cargo" /usr/local/bin/cargo /usr/bin/cargo; do
  [[ -x "$p" ]] && { CARGO="$p"; break; }
done
[[ -n "$CARGO" ]] || fail "Cargo не найден. Установи Rust: https://rustup.rs"
ok "$($CARGO --version)"

# ── Сборка ────────────────────────────────────────────────────
hdr "Сборка vchat (Rust)"
pushd "$SCRIPT_DIR/token-server" > /dev/null
sudo -u "$REAL_USER" "$CARGO" build --release 2>&1 | grep -E "^error|Compiling vchat|Finished"
BIN="target/release/vchat"
[[ -f "$BIN" ]] || fail "Сборка не удалась"
ok "$(du -sh $BIN | cut -f1) → $BIN"
popd > /dev/null

# ── Установка ────────────────────────────────────────────────
hdr "Установка файлов"
mkdir -p "$INSTALL_DIR"/{bin,log,public}
install -m 755 "$SCRIPT_DIR/token-server/$BIN" "$INSTALL_DIR/bin/vchat"
cp -r "$SCRIPT_DIR/public/"* "$INSTALL_DIR/public/"
ok "Файлы в $INSTALL_DIR"

# ── LiveKit ───────────────────────────────────────────────────
hdr "LiveKit Server v${LIVEKIT_VER}"
if [[ -f "$INSTALL_DIR/bin/livekit-server" ]]; then
  ok "Уже установлен"
else
  ARCH=$(uname -m); case "$ARCH" in x86_64) LA="amd64";; aarch64) LA="arm64";; *) fail "Неизвестная архитектура: $ARCH";; esac
  LK_URL="https://github.com/livekit/livekit/releases/download/v${LIVEKIT_VER}/livekit_${LIVEKIT_VER}_linux_${LA}.tar.gz"
  info "Скачиваем..."
  TMP=$(mktemp -d)
  curl -L --progress-bar "$LK_URL" | tar xz -C "$TMP"
  install -m 755 "$TMP/livekit-server" "$INSTALL_DIR/bin/livekit-server"
  rm -rf "$TMP"
  ok "livekit-server установлен"
fi

# ── Секрет ───────────────────────────────────────────────────
hdr "Секретный ключ"
SECRET_FILE="$INSTALL_DIR/secret"
if [[ ! -f "$SECRET_FILE" ]]; then
  tr -dc 'a-f0-9' < /dev/urandom | head -c 64 > "$SECRET_FILE"
  chmod 600 "$SECRET_FILE"
  ok "Сгенерирован новый секрет"
else
  ok "Используем существующий"
fi
SECRET=$(cat "$SECRET_FILE")

# ── Конфиг LiveKit ────────────────────────────────────────────
hdr "Конфиг LiveKit"
PUBLIC_IP=$(curl -s --max-time 4 https://api4.my-ip.io/ip 2>/dev/null || curl -s --max-time 4 https://ifconfig.me 2>/dev/null || echo "")
LOCAL_IP=$(ip route get 8.8.8.8 2>/dev/null | awk '/src/{for(i=1;i<=NF;i++) if($i=="src") print $(i+1)}' | head -1)
info "IP: ${LOCAL_IP} → ${PUBLIC_IP:-не определён}"

cat > "$INSTALL_DIR/livekit.yaml" << YAML
port: 7880
rtc:
  tcp_port: 7881
  udp_port: 7882
  use_external_ip: true
$([ "${PUBLIC_IP:-}" != "${LOCAL_IP:-}" ] && [ -n "${PUBLIC_IP:-}" ] && echo "  external_ip: ${PUBLIC_IP}" || echo "")
keys:
  vchat_key: ${SECRET}
logging:
  level: warn
  json: false
room:
  max_participants: 20
  empty_timeout: 300s
YAML
ok "livekit.yaml создан"

# ── systemd: vchat (Rust) ────────────────────────────────────
hdr "systemd сервисы"
cat > /etc/systemd/system/vchat.service << UNIT
[Unit]
Description=VoiceChat API + Static (Rust)
After=network.target

[Service]
Type=simple
User=nobody
Group=nogroup
Environment=LIVEKIT_API_KEY=vchat_key
Environment=LIVEKIT_API_SECRET=${SECRET}
Environment=HOST=0.0.0.0
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

[Install]
WantedBy=multi-user.target
UNIT

# ── systemd: livekit ──────────────────────────────────────────
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

[Install]
WantedBy=multi-user.target
UNIT

systemctl daemon-reload
systemctl enable --now vchat livekit
ok "Сервисы запущены"

# ── Firewall ──────────────────────────────────────────────────
hdr "Порты"
if command -v ufw &>/dev/null && ufw status 2>/dev/null | grep -q active; then
  ufw allow 3000/tcp comment "VoiceChat web" 2>/dev/null || true
  ufw allow 7880/tcp comment "LiveKit WS"    2>/dev/null || true
  ufw allow 7881/tcp comment "WebRTC TCP"    2>/dev/null || true
  ufw allow 7882/udp comment "WebRTC UDP"    2>/dev/null || true
  ok "UFW: порты открыты"
elif command -v firewall-cmd &>/dev/null; then
  for p in 3000/tcp 7880/tcp 7881/tcp 7882/udp; do
    firewall-cmd --permanent --add-port="$p" 2>/dev/null || true
  done
  firewall-cmd --reload; ok "firewalld: порты открыты"
else
  warn "Открой вручную: 3000/tcp, 7880/tcp, 7881/tcp, 7882/udp"
fi

# ── Итог ─────────────────────────────────────────────────────
echo ""
echo -e "${WHT}══════════════════════════════════════${NC}"
echo -e "${GRN}  Готово!${NC}"
echo -e "${WHT}══════════════════════════════════════${NC}"
echo -e "  Веб-интерфейс : ${WHT}http://${PUBLIC_IP:-localhost}:3000${NC}"
echo ""
echo -e "  journalctl -u vchat    -f"
echo -e "  journalctl -u livekit  -f"
echo ""
