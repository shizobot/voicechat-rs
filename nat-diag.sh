#!/usr/bin/env bash
# ============================================================
#  nat-diag.sh — Диагностика NAT и выбор метода пробития
#  Запуск: bash nat-diag.sh
# ============================================================
set -euo pipefail

RED='\033[0;31m'; GRN='\033[0;32m'; YEL='\033[1;33m'
BLU='\033[0;34m'; CYN='\033[0;36m'; WHT='\033[1;37m'; NC='\033[0m'

ok()   { echo -e "  ${GRN}✓${NC} $*"; }
fail() { echo -e "  ${RED}✗${NC} $*"; }
warn() { echo -e "  ${YEL}⚠${NC} $*"; }
info() { echo -e "  ${BLU}→${NC} $*"; }
hdr()  { echo -e "\n${WHT}══ $* ══${NC}"; }

need() { command -v "$1" &>/dev/null || { warn "Не найден: $1 (apt install $1)"; return 1; }; }

# ── 1. БАЗОВЫЕ ПАРАМЕТРЫ ────────────────────────────────────
hdr "1. Сетевые параметры"

PUBLIC_IP=""
for svc in "https://api4.my-ip.io/ip" "https://ipv4.icanhazip.com" "https://ifconfig.me"; do
  IP=$(curl -s --max-time 4 "$svc" 2>/dev/null | tr -d '[:space:]') || continue
  if [[ "$IP" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
    PUBLIC_IP="$IP"; break
  fi
done

LOCAL_IP=$(ip route get 8.8.8.8 2>/dev/null | awk '{for(i=1;i<=NF;i++) if($i=="src") {print $(i+1); exit}}')
DEFAULT_GW=$(ip route | awk '/^default/{print $3; exit}')
DEFAULT_IF=$(ip route | awk '/^default/{print $5; exit}')

echo ""
info "Публичный IP : ${WHT}${PUBLIC_IP:-неизвестен}${NC}"
info "Локальный IP : ${WHT}${LOCAL_IP:-неизвестен}${NC}"
info "Шлюз         : ${WHT}${DEFAULT_GW:-неизвестен}${NC}"
info "Интерфейс    : ${WHT}${DEFAULT_IF:-неизвестен}${NC}"

if [[ "$PUBLIC_IP" == "$LOCAL_IP" ]]; then
  ok "Прямой публичный IP (NAT отсутствует)"
  BEHIND_NAT=0
else
  warn "За NAT (${LOCAL_IP} → ${PUBLIC_IP})"
  BEHIND_NAT=1
fi

# ── 2. STUN PROBE ───────────────────────────────────────────
hdr "2. STUN-зондирование (определение типа NAT)"

STUN_SERVERS=(
  "stun.l.google.com:19302"
  "stun1.l.google.com:19302"
  "stun.cloudflare.com:3478"
  "stun.nextcloud.com:3478"
  "stun.sipgate.net:3478"
)

stun_query() {
  local host="$1" port="$2"
  # RFC 3489 Binding Request (20 байт заголовок)
  local hex="000100002112A44200000000000000000000000000000000"
  local result
  result=$(printf '%b' "$(echo "$hex" | sed 's/../\\x&/g')" | \
    nc -u -w3 "$host" "$port" 2>/dev/null | \
    xxd -p 2>/dev/null | tr -d '\n' || true)
  
  if [[ ${#result} -ge 40 ]]; then
    # Извлекаем XOR-MAPPED-ADDRESS (тип 0020) или MAPPED-ADDRESS (тип 0001)
    local xor_pos=$(echo "$result" | grep -bo "0020" | head -1 | cut -d: -f1 2>/dev/null || echo "")
    local map_pos=$(echo "$result" | grep -bo "0001" | head -1 | cut -d: -f1 2>/dev/null || echo "")
    
    # Fallback: парсим по позиции (упрощённо)
    # XOR-MAPPED: смещение 28 байт = 56 hex chars
    if [[ ${#result} -ge 72 ]]; then
      local magic="2112a442"
      local p1=$(( 16#${result:56:2} ^ 16#21 ))
      local p2=$(( 16#${result:58:2} ^ 16#12 ))
      local mapped_port=$(( (p1 << 8) | p2 ))
      local o1=$(( 16#${result:60:2} ^ 16#21 ))
      local o2=$(( 16#${result:62:2} ^ 16#12 ))
      local o3=$(( 16#${result:64:2} ^ 16#a4 ))
      local o4=$(( 16#${result:66:2} ^ 16#42 ))
      if [[ $mapped_port -gt 0 && $mapped_port -lt 65535 && $o1 -gt 0 ]]; then
        echo "${o1}.${o2}.${o3}.${o4}:${mapped_port}"
        return 0
      fi
    fi
  fi
  echo ""
}

STUN_RESULTS=()
STUN_IPS=()
STUN_PORTS=()

echo ""
for srv in "${STUN_SERVERS[@]}"; do
  host="${srv%:*}"; port="${srv#*:}"
  printf "  %-40s" "$srv"
  
  # Простая проверка UDP доступности
  if ! nc -zu -w3 "$host" "$port" 2>/dev/null; then
    echo -e "${RED}недоступен${NC}"
    continue
  fi
  
  result=$(stun_query "$host" "$port")
  if [[ -n "$result" ]]; then
    echo -e "${GRN}${result}${NC}"
    STUN_RESULTS+=("$result")
    STUN_IPS+=("${result%:*}")
    STUN_PORTS+=("${result#*:}")
  else
    echo -e "${YEL}доступен (парсинг не удался)${NC}"
    STUN_RESULTS+=("ok")
  fi
done

# Анализ типа NAT по вариативности портов
UNIQ_PORTS=$(printf '%s\n' "${STUN_PORTS[@]:-}" | sort -u | grep -c . || echo 0)
echo ""
if [[ $BEHIND_NAT -eq 0 ]]; then
  info "NAT тип: ${GRN}Нет NAT (публичный IP)${NC}"
  NAT_TYPE="open"
elif [[ ${#STUN_PORTS[@]} -eq 0 ]]; then
  warn "NAT тип: ${RED}STUN заблокирован (возможно Symmetric + strict firewall)${NC}"
  NAT_TYPE="symmetric"
elif [[ $UNIQ_PORTS -le 1 ]]; then
  ok "NAT тип: ${GRN}Full Cone / Restricted Cone${NC} (порт стабилен)"
  NAT_TYPE="cone"
elif [[ $UNIQ_PORTS -le 2 ]]; then
  warn "NAT тип: ${YEL}Port-Restricted Cone${NC} (порт немного варьируется)"
  NAT_TYPE="port_restricted"
else
  fail "NAT тип: ${RED}Symmetric NAT${NC} (порт меняется для каждого назначения, уникальных: $UNIQ_PORTS)"
  NAT_TYPE="symmetric"
fi

# ── 3. UPnP ─────────────────────────────────────────────────
hdr "3. UPnP / NAT-PMP (автопроброс портов)"
echo ""

UPNP_OK=0
if need upnpc 2>/dev/null; then
  if upnpc -l 2>/dev/null | grep -q "ExternalIPAddress"; then
    ok "UPnP работает на роутере"
    UPNP_EXTERNAL=$(upnpc -l 2>/dev/null | grep "ExternalIPAddress" | awk '{print $3}')
    info "Внешний IP по UPnP: $UPNP_EXTERNAL"
    
    # Пробуем пробросить порт 7882/UDP
    if upnpc -r 7882 UDP 2>/dev/null | grep -q "success"; then
      ok "Порт 7882/UDP пробит через UPnP"
      UPNP_OK=1
    else
      warn "UPnP есть, но проброс 7882/UDP не удался"
    fi
  else
    fail "UPnP на роутере не найден или отключён"
  fi
else
  warn "miniupnpc не установлен: apt install miniupnpd-utils"
  # Пробуем через SSDP вручную
  SSDP_RESP=$(echo -ne 'M-SEARCH * HTTP/1.1\r\nHOST:239.255.255.250:1900\r\nMAN:"ssdp:discover"\r\nMX:3\r\nST:upnp:rootdevice\r\n\r\n' | \
    nc -u -w4 239.255.255.250 1900 2>/dev/null | head -5 || true)
  if [[ -n "$SSDP_RESP" ]]; then
    ok "SSDP/UPnP устройства обнаружены в сети"
    warn "Установи miniupnpc для автопроброса: apt install miniupnpd-utils"
  else
    fail "UPnP устройства не найдены"
  fi
fi

# NAT-PMP (более современный протокол, AirPort, FritzBox)
echo ""
if need natpmpc 2>/dev/null; then
  NAT_PMP_IP=$(natpmpc 2>/dev/null | grep "Mapped public address" | awk '{print $4}' || true)
  if [[ -n "$NAT_PMP_IP" ]]; then
    ok "NAT-PMP работает, публичный IP: $NAT_PMP_IP"
    natpmpc -a 7882 7882 udp 3600 2>/dev/null && ok "Порт 7882/UDP пробит через NAT-PMP" || warn "NAT-PMP: проброс не удался"
  else
    fail "NAT-PMP не поддерживается роутером"
  fi
fi

# ── 4. ПРОВЕРКА ПОРТОВ ──────────────────────────────────────
hdr "4. Доступность портов для LiveKit"
echo ""

TEST_PORTS=(
  "7880:TCP:LiveKit WebSocket"
  "7881:TCP:WebRTC TCP fallback"
  "7882:UDP:WebRTC основной трафик"
  "3000:TCP:Веб-интерфейс"
  "3478:UDP:STUN/TURN"
  "5349:TCP:TURN TLS"
)

check_port_local() {
  local port=$1 proto=$2
  if [[ "$proto" == "TCP" ]]; then
    ss -tlnp 2>/dev/null | grep -q ":${port} " && echo "listening" || echo "closed"
  else
    ss -ulnp 2>/dev/null | grep -q ":${port} " && echo "listening" || echo "closed"
  fi
}

check_port_external() {
  local port=$1
  # Используем внешний сервис для проверки (если есть интернет)
  result=$(curl -s --max-time 5 "https://portcheck.io/api/${PUBLIC_IP}/${port}" 2>/dev/null | \
    python3 -c "import sys,json; d=json.load(sys.stdin); print('open' if d.get('status')=='open' else 'closed')" 2>/dev/null || echo "unknown")
  echo "$result"
}

for entry in "${TEST_PORTS[@]}"; do
  IFS=: read -r port proto desc <<< "$entry"
  local_state=$(check_port_local "$port" "$proto")
  printf "  %-6s %-5s %-30s локально: " "$port" "$proto" "$desc"
  if [[ "$local_state" == "listening" ]]; then
    printf "${GRN}слушает${NC}"
  else
    printf "${YEL}не запущен${NC}"
  fi
  echo ""
done

# ── 5. FIREWALL ─────────────────────────────────────────────
hdr "5. Firewall (iptables / ufw / firewalld)"
echo ""

if command -v ufw &>/dev/null; then
  UFW_STATUS=$(ufw status 2>/dev/null | head -1)
  info "UFW: $UFW_STATUS"
  if echo "$UFW_STATUS" | grep -q "active"; then
    ufw status numbered 2>/dev/null | grep -E "7880|7881|7882|3000|3478" && \
      ok "Нужные порты открыты в UFW" || \
      warn "Порты LiveKit не найдены в UFW правилах"
    echo ""
    info "Команды для открытия портов UFW:"
    echo "    sudo ufw allow 7880/tcp"
    echo "    sudo ufw allow 7881/tcp"
    echo "    sudo ufw allow 7882/udp"
    echo "    sudo ufw allow 3000/tcp"
  fi
fi

if command -v firewall-cmd &>/dev/null; then
  info "Firewalld: $(firewall-cmd --state 2>/dev/null)"
  echo ""
  info "Команды для открытия портов firewalld:"
  echo "    sudo firewall-cmd --permanent --add-port=7880/tcp"
  echo "    sudo firewall-cmd --permanent --add-port=7882/udp"
  echo "    sudo firewall-cmd --reload"
fi

IPTABLES_RULES=$(iptables -L INPUT -n 2>/dev/null | grep -E "7880|7881|7882|ACCEPT" | head -5 || true)
if [[ -n "$IPTABLES_RULES" ]]; then
  info "Релевантные правила iptables:"
  echo "$IPTABLES_RULES" | sed 's/^/    /'
fi

# ── 6. TAILSCALE ────────────────────────────────────────────
hdr "6. Tailscale (альтернативный метод)"
echo ""

if command -v tailscale &>/dev/null; then
  TS_STATUS=$(tailscale status 2>/dev/null | head -3 || echo "не запущен")
  ok "Tailscale установлен"
  info "Статус:\n$(echo "$TS_STATUS" | sed 's/^/    /')"
  TS_IP=$(tailscale ip -4 2>/dev/null || echo "")
  [[ -n "$TS_IP" ]] && info "Tailscale IP: ${GRN}${TS_IP}${NC}"
else
  warn "Tailscale не установлен"
  info "Установка: curl -fsSL https://tailscale.com/install.sh | sh"
  info "Это самый простой способ для группы друзей!"
fi

# ── 7. COTURN (TURN сервер) ──────────────────────────────────
hdr "7. TURN сервер (coturn)"
echo ""

if command -v turnserver &>/dev/null || docker ps 2>/dev/null | grep -q coturn; then
  ok "coturn уже запущен"
else
  warn "coturn не установлен"
  info "Docker: docker run -d --network=host coturn/coturn"
fi

# ── 8. ИТОГ И РЕКОМЕНДАЦИИ ──────────────────────────────────
hdr "8. ИТОГ И РЕКОМЕНДАЦИИ"
echo ""

echo -e "  ${WHT}Ваш NAT тип:${NC}"
case "$NAT_TYPE" in
  open)
    echo -e "    ${GRN}Открытый хост — никаких проблем с NAT${NC}"
    echo ""
    echo -e "  ${WHT}Рекомендация:${NC}"
    echo "    1. Убедись, что файрвол открыт (порты 7880, 7881, 7882/UDP)"
    echo "    2. Запускай LiveKit self-hosted — всё будет работать"
    ;;
  cone)
    echo -e "    ${GRN}Full/Restricted Cone NAT — хорошо${NC}"
    echo ""
    echo -e "  ${WHT}Рекомендация:${NC}"
    echo "    1. Включи UPnP на роутере (обычно в настройках роутера)"
    echo "    2. Либо пробрось порт 7882/UDP вручную на IP сервера"
    echo "    3. LiveKit STUN будет работать, TURN не нужен"
    ;;
  port_restricted)
    echo -e "    ${YEL}Port-Restricted Cone NAT — требует настройки${NC}"
    echo ""
    echo -e "  ${WHT}Рекомендация (выбери одно):${NC}"
    echo "    A. Проброс порта вручную: роутер → 7882/UDP → IP_СЕРВЕРА"
    echo "    B. UPnP: включить в настройках роутера"
    echo "    C. Tailscale: самый простой вариант для друзей"
    echo "    D. TURN сервер на VPS как резерв"
    ;;
  symmetric)
    echo -e "    ${RED}Symmetric NAT — сложный случай${NC}"
    echo ""
    echo -e "  ${WHT}Рекомендация (в порядке простоты):${NC}"
    echo "    A. ${GRN}Tailscale${NC} — проще всего, работает всегда:"
    echo "       curl -fsSL https://tailscale.com/install.sh | sh"
    echo "       tailscale up"
    echo "       → Раздай друзьям ссылку на join, все в одной VPN сети"
    echo ""
    echo "    B. ${BLU}coturn на VPS${NC} — если нужен именно WebRTC:"
    cat << 'EOF'
       docker run -d --name coturn --network=host \
         -e DETECT_EXTERNAL_IP=yes \
         coturn/coturn \
         -n --log-file=stdout \
         --use-auth-secret \
         --static-auth-secret=MY_SECRET \
         --realm=voicechat \
         --fingerprint \
         --lt-cred-mech \
         --no-multicast-peers \
         --no-cli \
         --no-tlsv1 \
         --no-tlsv1_1
EOF
    echo ""
    echo "    C. ${YEL}Проброс порта${NC} — если есть доступ к роутеру:"
    echo "       Открой порт 7882/UDP на внешнем IP → IP сервера"
    ;;
esac

echo ""
echo -e "  ${WHT}Скрипт завершён.${NC}"
echo ""
