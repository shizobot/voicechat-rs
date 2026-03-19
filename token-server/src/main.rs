//! vchat — единый бинарник: API токенов + статика
//!
//! Эндпоинты:
//!   GET  /               → index.html
//!   GET  /api/rooms      → JSON список комнат
//!   POST /api/rooms      → { "name": "..." }
//!   POST /api/token      → { "room": "...", "username": "..." }
//!   GET  /api/health     → { "ok": true }

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
    api_key:    String,
    api_secret: String,
    host:       String,
    port:       u16,
    static_dir: PathBuf,
}

impl Config {
    fn from_env() -> Self {
        let api_secret = env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| {
            eprintln!("[WARN] LIVEKIT_API_SECRET не задан — используется небезопасное значение");
            "change_me_in_production".into()
        });
        Self {
            api_key:    env::var("LIVEKIT_API_KEY").unwrap_or_else(|_| "vchat_key".into()),
            api_secret,
            host:       env::var("HOST").unwrap_or_else(|_| "0.0.0.0".into()),
            port:       env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000),
            static_dir: env::var("STATIC_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("public")),
        }
    }
}

// ── JWT ──────────────────────────────────────────────────────
fn b64(data: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(data)
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn random_jti() -> String {
    let mut buf = [0u8; 8];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        // читаем ровно 8 байт — read() может вернуть меньше
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
    let payload = serde_json::json!({
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
    });
    make_jwt(&payload, &cfg.api_secret)
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
fn cors() -> Vec<Header> {
    [
        ("Access-Control-Allow-Origin",  "*"),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "Content-Type"),
    ]
    .iter()
    .map(|(k, v)| Header::from_bytes(k.as_bytes(), v.as_bytes()).unwrap())
    .collect()
}

fn json_resp(code: u16, body: serde_json::Value) -> Response<std::io::Cursor<Vec<u8>>> {
    let bytes = body.to_string().into_bytes();
    let mut r = Response::from_data(bytes).with_status_code(code);
    r.add_header(Header::from_bytes("Content-Type", "application/json; charset=utf-8").unwrap());
    for h in cors() { r.add_header(h); }
    r
}

fn options_resp() -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_data(vec![]).with_status_code(204);
    for h in cors() { r.add_header(h); }
    r
}

fn html_resp(code: u16, bytes: Vec<u8>) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_data(bytes).with_status_code(code);
    r.add_header(Header::from_bytes("Content-Type", "text/html; charset=utf-8").unwrap());
    r
}

fn not_found() -> Response<std::io::Cursor<Vec<u8>>> {
    json_resp(404, serde_json::json!({ "error": "not found" }))
}

const MAX_BODY: usize = 8 * 1024; // 8 KB

fn read_body(req: &mut tiny_http::Request) -> Option<String> {
    let limit = req.body_length().unwrap_or(0);
    if limit > MAX_BODY {
        return None; // тело слишком большое
    }
    let mut buf = String::new();
    let mut reader = req.as_reader().take(MAX_BODY as u64);
    if reader.read_to_string(&mut buf).is_err() {
        return None;
    }
    Some(buf)
}

// ── Комнаты ───────────────────────────────────────────────────
struct Rooms {
    map: Mutex<HashMap<String, u64>>,  // name → created_at
}

impl Rooms {
    fn new() -> Self {
        Self { map: Mutex::new(HashMap::new()) }
    }

    fn list(&self) -> Vec<String> {
        // Удаляем комнаты старше 12 часов (давно никого нет)
        let cutoff = now_secs().saturating_sub(12 * 3600);
        let mut map = self.map.lock().unwrap();
        map.retain(|_, ts| *ts > cutoff);
        map.keys().cloned().collect()
    }

    fn add(&self, name: String) -> bool {
        let mut map = self.map.lock().unwrap();
        if map.len() >= 200 {
            return false; // максимум 200 комнат
        }
        map.insert(name, now_secs());
        true
    }
}

// ── Обработчик запроса ────────────────────────────────────────
fn handle(
    req:    &mut tiny_http::Request,
    method: &Method,
    path:   &str,
    cfg:    &Config,
    rooms:  &Rooms,
) -> Response<std::io::Cursor<Vec<u8>>> {

    if *method == Method::Options {
        return options_resp();
    }

    match (method, path) {

        // ── Главная страница ──────────────────────────────────
        (Method::Get, "/") | (Method::Get, "/index.html") => {
            let file = cfg.static_dir.join("index.html");
            match fs::read(&file) {
                Ok(bytes) => html_resp(200, bytes),
                Err(e) => {
                    eprintln!("[ERROR] Не удалось прочитать {:?}: {e}", file);
                    json_resp(500, serde_json::json!({ "error": "index.html не найден" }))
                }
            }
        }

        // ── GET /api/health ───────────────────────────────────
        (Method::Get, "/api/health") => {
            json_resp(200, serde_json::json!({ "ok": true }))
        }

        // ── GET /api/rooms ────────────────────────────────────
        (Method::Get, "/api/rooms") => {
            let list = rooms.list();
            json_resp(200, serde_json::json!(list))
        }

        // ── POST /api/rooms ───────────────────────────────────
        (Method::Post, "/api/rooms") => {
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({ "error": "тело запроса слишком большое" }));
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({ "error": "invalid JSON" }));
            };
            let name = json["name"].as_str().unwrap_or("").trim().to_owned();
            if !valid_room(&name) {
                return json_resp(400, serde_json::json!({ "error": "Некорректное название комнаты" }));
            }
            if !rooms.add(name) {
                return json_resp(429, serde_json::json!({ "error": "слишком много комнат" }));
            }
            json_resp(200, serde_json::json!({ "ok": true }))
        }

        // ── POST /api/token ───────────────────────────────────
        (Method::Post, "/api/token") => {
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({ "error": "тело запроса слишком большое" }));
            };
            let Ok(json) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({ "error": "invalid JSON" }));
            };
            let room     = json["room"].as_str().unwrap_or("").trim().to_owned();
            let username = json["username"].as_str().unwrap_or("").trim().to_owned();

            if !valid_room(&room) {
                return json_resp(400, serde_json::json!({ "error": "Некорректное название комнаты" }));
            }
            if !valid_name(&username) {
                return json_resp(400, serde_json::json!({ "error": "Некорректное имя пользователя" }));
            }
            let token = build_token(cfg, &room, &username);
            json_resp(200, serde_json::json!({ "token": token }))
        }

        _ => not_found(),
    }
}

// ── Main ──────────────────────────────────────────────────────
fn main() {
    let cfg   = Arc::new(Config::from_env());
    let rooms = Arc::new(Rooms::new());
    let addr  = format!("{}:{}", cfg.host, cfg.port);

    // Проверяем наличие static_dir
    if !cfg.static_dir.exists() {
        eprintln!("[WARN] Папка статики не найдена: {:?}", cfg.static_dir);
        eprintln!("[WARN] Укажи STATIC_DIR=/path/to/public или создай папку public/");
    }

    let server = Server::http(&addr)
        .unwrap_or_else(|e| { eprintln!("[ERROR] Не удалось запустить: {e}"); std::process::exit(1) });

    println!("╔══════════════════════════════════╗");
    println!("║  VoiceChat backend               ║");
    println!("╠══════════════════════════════════╣");
    println!("║  http://{}   ║", addr);
    println!("║  API key: {:<23}║", cfg.api_key);
    println!("║  Static:  {:<23}║", cfg.static_dir.display());
    println!("╚══════════════════════════════════╝");

    for mut req in server.incoming_requests() {
        let cfg   = Arc::clone(&cfg);
        let rooms = Arc::clone(&rooms);

        std::thread::spawn(move || {
            let url    = req.url().to_owned();
            let method = req.method().clone();
            let path   = url.split('?').next().unwrap_or("/");
            let ip     = req.remote_addr().map(|a| a.to_string()).unwrap_or_default();

            let resp = handle(&mut req, &method, path, &cfg, &rooms);
            let code = resp.status_code().0;

            println!("[{code}] {method} {path}  {ip}");
            let _ = req.respond(resp);
        });
    }
}
