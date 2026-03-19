//! vchat — API токенов + статика + конфиг для фронтенда
//!
//! GET  /              → index.html
//! GET  /api/config    → { "livekit_url": "wss://...", "version": "1.0" }
//! GET  /api/rooms     → ["room1", ...]
//! POST /api/rooms     → { "name": "..." }
//! POST /api/token     → { "room": "...", "username": "..." }
//! GET  /api/health    → { "ok": true }
//! GET  /favicon.ico   → 204

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::HashMap;
use std::env;
use std::fs;
use std::io::Read;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tiny_http::{Header, Method, Response, Server};

type HmacSha256 = Hmac<Sha256>;

// ── Конфиг ───────────────────────────────────────────────────
struct Config {
    api_key:      String,
    api_secret:   String,
    host:         String,
    port:         u16,
    static_dir:   PathBuf,
    livekit_url:  String,  // wss://domain/ws  или  ws://localhost:7880
    allowed_origin: String, // CORS
}

impl Config {
    fn from_env() -> Self {
        let api_secret = env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| {
            eprintln!("[WARN] LIVEKIT_API_SECRET не задан");
            "change_me_in_production".into()
        });

        // По умолчанию ws:// для локальной разработки
        // В продакшне: LIVEKIT_URL=wss://chat.example.com/ws
        let livekit_url = env::var("LIVEKIT_URL")
            .unwrap_or_else(|_| "ws://localhost:7880".into());

        let allowed_origin = env::var("ALLOWED_ORIGIN")
            .unwrap_or_else(|_| "*".into());

        Self {
            api_key:    env::var("LIVEKIT_API_KEY").unwrap_or_else(|_| "vchat_key".into()),
            api_secret,
            host:       env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port:       env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000),
            static_dir: env::var("STATIC_DIR").map(PathBuf::from)
                            .unwrap_or_else(|_| PathBuf::from("public")),
            livekit_url,
            allowed_origin,
        }
    }
}

// ── JWT ──────────────────────────────────────────────────────
fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn random_jti() -> String {
    let mut buf = [0u8; 8];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = std::io::Read::read_exact(&mut f, &mut buf);
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

fn make_jwt(payload: &serde_json::Value, secret: &str) -> String {
    let header = b64(br#"{"alg":"HS256","typ":"JWT"}"#);
    let body   = b64(payload.to_string().as_bytes());
    let msg    = format!("{header}.{body}");
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).unwrap();
    mac.update(msg.as_bytes());
    let sig = b64(&mac.finalize().into_bytes());
    format!("{msg}.{sig}")
}

fn build_token(cfg: &Config, room: &str, identity: &str) -> String {
    let now = now_secs();
    make_jwt(&serde_json::json!({
        "iss": cfg.api_key,
        "sub": identity,
        "iat": now,
        "exp": now + 4 * 3600,
        "nbf": now,
        "jti": random_jti(),
        "video": {
            "room":           room,
            "roomJoin":       true,
            "canPublish":     true,
            "canSubscribe":   true,
            "canPublishData": true
        }
    }), &cfg.api_secret)
}

// ── Валидация ─────────────────────────────────────────────────
fn valid_room(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 60
        && s.chars().all(|c| c.is_alphanumeric() || "-_".contains(c))
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 40
        && s.chars().all(|c| {
            c.is_alphanumeric()
                || " -_".contains(c)
                || matches!(c, 'а'..='я' | 'А'..='Я' | 'ё' | 'Ё')
        })
}

// ── HTTP helpers ──────────────────────────────────────────────
fn security_headers(r: &mut Response<std::io::Cursor<Vec<u8>>>, origin: &str) {
    let headers = [
        ("Access-Control-Allow-Origin",  origin),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "Content-Type"),
        ("X-Content-Type-Options",       "nosniff"),
        ("X-Frame-Options",              "DENY"),
        ("Referrer-Policy",              "strict-origin-when-cross-origin"),
    ];
    for (k, v) in headers {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            r.add_header(h);
        }
    }
}

fn json_resp(code: u16, body: serde_json::Value, origin: &str)
    -> Response<std::io::Cursor<Vec<u8>>>
{
    let bytes = body.to_string().into_bytes();
    let mut r = Response::from_data(bytes).with_status_code(code);
    r.add_header(
        Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap()
    );
    security_headers(&mut r, origin);
    r
}

fn html_resp(bytes: Vec<u8>) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_data(bytes).with_status_code(200);
    r.add_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
    r.add_header(Header::from_bytes("Cache-Control", "no-cache").unwrap());
    r
}

fn options_resp(origin: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_data(vec![]).with_status_code(204);
    security_headers(&mut r, origin);
    r
}

fn empty_resp(code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(vec![]).with_status_code(code)
}

const MAX_BODY: usize = 8 * 1024;

fn read_body(req: &mut tiny_http::Request) -> Option<String> {
    if req.body_length().unwrap_or(0) > MAX_BODY {
        return None;
    }
    let mut buf = String::new();
    if req.as_reader().take(MAX_BODY as u64).read_to_string(&mut buf).is_err() {
        return None;
    }
    Some(buf)
}

// ── Комнаты ───────────────────────────────────────────────────
struct Rooms {
    map: Mutex<HashMap<String, u64>>,
}

impl Rooms {
    fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }

    fn list(&self) -> Vec<String> {
        let cutoff = now_secs().saturating_sub(12 * 3600);
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, ts| *ts > cutoff);
        map.keys().cloned().collect()
    }

    fn add(&self, name: String) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= 200 {
            return false;
        }
        map.insert(name, now_secs());
        true
    }
}

// ── Rate limiting ──────────────────────────────────────────────
struct RateLimit {
    // ip → (count, window_start)
    map: Mutex<HashMap<String, (u32, u64)>>,
}

impl RateLimit {
    fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }

    // Возвращает true если запрос разрешён
    fn check(&self, ip: &str, max: u32, window_secs: u64) -> bool {
        let now = now_secs();
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());

        // Чистим старые записи раз в 1000 запросов
        if map.len() > 1000 {
            let cutoff = now.saturating_sub(window_secs * 2);
            map.retain(|_, (_, ts)| *ts > cutoff);
        }

        let entry = map.entry(ip.to_owned()).or_insert((0, now));
        if now - entry.1 > window_secs {
            // Новое окно
            *entry = (1, now);
            return true;
        }
        if entry.0 >= max {
            return false;
        }
        entry.0 += 1;
        true
    }
}

// ── Обработчик ────────────────────────────────────────────────
fn handle(
    req:       &mut tiny_http::Request,
    method:    &Method,
    path:      &str,
    ip:        &str,
    cfg:       &Config,
    rooms:     &Rooms,
    rate_token: &RateLimit,
    rate_room:  &RateLimit,
) -> Response<std::io::Cursor<Vec<u8>>> {
    let origin = cfg.allowed_origin.as_str();

    if *method == Method::Options {
        return options_resp(origin);
    }

    match (method, path) {

        // ── Статика ───────────────────────────────────────────
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            match fs::read(cfg.static_dir.join("index.html")) {
                Ok(b) => html_resp(b),
                Err(_) => json_resp(500, serde_json::json!({"error":"not found"}), origin),
            }
        }

        // ── favicon — молчаливый 204 ──────────────────────────
        (Method::Get, "/favicon.ico") => empty_resp(204),

        // ── /api/config — отдаём URL LiveKit фронтенду ────────
        (Method::Get, "/api/config") => {
            json_resp(200, serde_json::json!({
                "livekit_url": cfg.livekit_url,
                "version":     env!("CARGO_PKG_VERSION")
            }), origin)
        }

        // ── /api/health ───────────────────────────────────────
        (Method::Get, "/api/health") => {
            json_resp(200, serde_json::json!({"ok": true}), origin)
        }

        // ── GET /api/rooms ────────────────────────────────────
        (Method::Get, "/api/rooms") => {
            json_resp(200, serde_json::json!(rooms.list()), origin)
        }

        // ── POST /api/rooms ───────────────────────────────────
        (Method::Post, "/api/rooms") => {
            if !rate_room.check(ip, 5, 60) {
                return json_resp(429, serde_json::json!({"error":"слишком много запросов"}), origin);
            }
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({"error":"тело слишком большое"}), origin);
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({"error":"invalid JSON"}), origin);
            };
            let name = json["name"].as_str().unwrap_or("").trim().to_owned();
            if !valid_room(&name) {
                return json_resp(400, serde_json::json!({"error":"некорректное название"}), origin);
            }
            if !rooms.add(name) {
                return json_resp(429, serde_json::json!({"error":"слишком много комнат"}), origin);
            }
            json_resp(200, serde_json::json!({"ok": true}), origin)
        }

        // ── POST /api/token ───────────────────────────────────
        (Method::Post, "/api/token") => {
            if !rate_token.check(ip, 10, 60) {
                return json_resp(429, serde_json::json!({"error":"слишком много запросов"}), origin);
            }
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({"error":"тело слишком большое"}), origin);
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({"error":"invalid JSON"}), origin);
            };
            let room     = json["room"].as_str().unwrap_or("").trim().to_owned();
            let username = json["username"].as_str().unwrap_or("").trim().to_owned();

            if !valid_room(&room) {
                return json_resp(400, serde_json::json!({"error":"некорректная комната"}), origin);
            }
            if !valid_name(&username) {
                return json_resp(400, serde_json::json!({"error":"некорректное имя"}), origin);
            }
            let token = build_token(cfg, &room, &username);
            json_resp(200, serde_json::json!({"token": token}), origin)
        }

        _ => json_resp(404, serde_json::json!({"error":"not found"}), origin),
    }
}

// ── Main ──────────────────────────────────────────────────────
fn main() {
    let cfg        = Arc::new(Config::from_env());
    let rooms      = Arc::new(Rooms::new());
    let rate_token = Arc::new(RateLimit::new());
    let rate_room  = Arc::new(RateLimit::new());
    let addr       = format!("{}:{}", cfg.host, cfg.port);

    if !cfg.static_dir.exists() {
        eprintln!("[WARN] Папка статики не найдена: {:?}", cfg.static_dir);
    }

    let server = Server::http(&addr)
        .unwrap_or_else(|e| { eprintln!("[ERROR] {e}"); std::process::exit(1) });

    // Маскируем secret в выводе
    let key_preview = format!(
        "{}***",
        &cfg.api_key[..cfg.api_key.len().min(6)]
    );
    eprintln!("[vchat] http://{}  key={}  livekit={}", addr, key_preview, cfg.livekit_url);

    // Фиксированный пул потоков
    let thread_count = std::thread::available_parallelism()
        .map(|n| n.get() * 2)
        .unwrap_or(4)
        .min(32);

    let server = Arc::new(server);

    let handles: Vec<_> = (0..thread_count).map(|_| {
        let server     = Arc::clone(&server);
        let cfg        = Arc::clone(&cfg);
        let rooms      = Arc::clone(&rooms);
        let rate_token = Arc::clone(&rate_token);
        let rate_room  = Arc::clone(&rate_room);

        std::thread::spawn(move || {
            loop {
                let Ok(mut req) = server.recv() else { break };
                let url    = req.url().to_owned();
                let method = req.method().clone();
                let path   = url.split('?').next().unwrap_or("/");
                let ip     = req.remote_addr()
                    .map(|a| a.ip().to_string())
                    .unwrap_or_default();

                let resp = handle(
                    &mut req, &method, path, &ip,
                    &cfg, &rooms, &rate_token, &rate_room
                );
                let code = resp.status_code().0;
                eprintln!("[{code}] {method} {path}  {ip}");
                let _ = req.respond(resp);
            }
        })
    }).collect();

    for h in handles { let _ = h.join(); }
}
