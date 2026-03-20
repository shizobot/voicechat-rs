//! vchat backend
//!
//! GET  /                  → index.html
//! GET  /api/config        → {livekit_url, version}
//! GET  /api/rooms         → [{name, count}]
//! POST /api/rooms         → {name}
//! POST /api/token         → {room, username, avatar}
//! POST /api/check-nick    → {room, username} → {available}
//! GET  /api/health        → {ok}
//! GET  /favicon.ico       → 204

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

// ── Config ────────────────────────────────────────────────────
struct Config {
    api_key:        String,
    api_secret:     String,
    host:           String,
    port:           u16,
    static_dir:     PathBuf,
    livekit_url:    String,
    allowed_origin: String,
}

impl Config {
    fn from_env() -> Self {
        let api_secret = env::var("LIVEKIT_API_SECRET").unwrap_or_else(|_| {
            eprintln!("[WARN] LIVEKIT_API_SECRET не задан");
            "change_me_in_production".into()
        });
        Self {
            api_key:        env::var("LIVEKIT_API_KEY").unwrap_or_else(|_| "vchat_key".into()),
            api_secret,
            host:           env::var("HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            port:           env::var("PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(3000),
            static_dir:     env::var("STATIC_DIR").map(PathBuf::from)
                                .unwrap_or_else(|_| PathBuf::from("public")),
            livekit_url:    env::var("LIVEKIT_URL")
                                .unwrap_or_else(|_| "ws://localhost:7880".into()),
            allowed_origin: env::var("ALLOWED_ORIGIN").unwrap_or_else(|_| "*".into()),
        }
    }
}

// ── JWT ──────────────────────────────────────────────────────
fn b64(data: &[u8]) -> String { URL_SAFE_NO_PAD.encode(data) }

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
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
    format!("{msg}.{}", b64(&mac.finalize().into_bytes()))
}

fn build_token(cfg: &Config, room: &str, identity: &str, avatar: u8) -> String {
    let now = now_secs();
    make_jwt(&serde_json::json!({
        "iss": cfg.api_key, "sub": identity,
        "iat": now, "exp": now + 4*3600, "nbf": now, "jti": random_jti(),
        "metadata": serde_json::json!({"avatar": avatar}).to_string(),
        "video": {
            "room": room, "roomJoin": true,
            "canPublish": true, "canSubscribe": true, "canPublishData": true
        }
    }), &cfg.api_secret)
}

// ── Validation ────────────────────────────────────────────────
fn valid_room(s: &str) -> bool {
    !s.is_empty() && s.len() <= 60
        && s.chars().all(|c| c.is_alphanumeric() || "-_".contains(c))
}

fn valid_name(s: &str) -> bool {
    !s.is_empty() && s.len() <= 32
        && s.chars().all(|c| {
            c.is_alphanumeric() || "-_.".contains(c)
                || matches!(c, 'а'..='я'|'А'..='Я'|'ё'|'Ё')
        })
}

// ── HTTP helpers ──────────────────────────────────────────────
fn sec_headers(r: &mut Response<std::io::Cursor<Vec<u8>>>, origin: &str) {
    for (k, v) in [
        ("Access-Control-Allow-Origin",  origin),
        ("Access-Control-Allow-Methods", "GET, POST, OPTIONS"),
        ("Access-Control-Allow-Headers", "Content-Type"),
        ("X-Content-Type-Options",       "nosniff"),
        ("X-Frame-Options",              "DENY"),
        ("Referrer-Policy",              "strict-origin-when-cross-origin"),
    ] {
        if let Ok(h) = Header::from_bytes(k.as_bytes(), v.as_bytes()) {
            r.add_header(h);
        }
    }
}

fn json_resp(code: u16, body: serde_json::Value, origin: &str)
    -> Response<std::io::Cursor<Vec<u8>>>
{
    let mut r = Response::from_data(body.to_string().into_bytes()).with_status_code(code);
    r.add_header(Header::from_bytes("Content-Type","application/json; charset=utf-8").unwrap());
    sec_headers(&mut r, origin);
    r
}

fn html_resp(bytes: Vec<u8>) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_data(bytes).with_status_code(200);
    r.add_header(Header::from_bytes("Content-Type","text/html; charset=utf-8").unwrap());
    r.add_header(Header::from_bytes("Cache-Control","no-cache").unwrap());
    r
}

fn options_resp(origin: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    let mut r = Response::from_data(vec![]).with_status_code(204);
    sec_headers(&mut r, origin);
    r
}

fn empty(code: u16) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(vec![]).with_status_code(code)
}

const MAX_BODY: usize = 8 * 1024;

fn read_body(req: &mut tiny_http::Request) -> Option<String> {
    if req.body_length().unwrap_or(0) > MAX_BODY { return None; }
    let mut buf = String::new();
    if req.as_reader().take(MAX_BODY as u64).read_to_string(&mut buf).is_err() { return None; }
    Some(buf)
}

// ── Rooms ─────────────────────────────────────────────────────
struct RoomInfo {
    created_at: u64,
    count:      u32,   // активных участников (приблизительно)
}

struct Rooms {
    map: Mutex<HashMap<String, RoomInfo>>,
}

impl Rooms {
    fn new() -> Self { Self { map: Mutex::new(HashMap::new()) } }

    fn list(&self) -> Vec<serde_json::Value> {
        let cutoff = now_secs().saturating_sub(12 * 3600);
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, r| r.created_at > cutoff);
        let mut list: Vec<_> = map.iter()
            .map(|(k, v)| serde_json::json!({"name": k, "count": v.count}))
            .collect();
        list.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        list
    }

    fn add(&self, name: String) -> bool {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= 200 { return false; }
        map.entry(name).or_insert(RoomInfo { created_at: now_secs(), count: 0 });
        true
    }

    fn inc(&self, name: &str) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = map.get_mut(name) { r.count = r.count.saturating_add(1); }
    }

    fn dec(&self, name: &str) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(r) = map.get_mut(name) { r.count = r.count.saturating_sub(1); }
    }
}

// ── Nick registry ─────────────────────────────────────────────
// room → nick → (registered_at, avatar)
struct Nicks {
    map: Mutex<HashMap<String, HashMap<String, (u64, u8)>>>,
}

impl Nicks {
    fn new() -> Self { Self { map: Mutex::new(HashMap::new()) } }

    fn is_available(&self, room: &str, nick: &str) -> bool {
        let now = now_secs();
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        match map.get(room).and_then(|r| r.get(nick)) {
            None => true,
            Some((ts, _)) => now.saturating_sub(*ts) > 4 * 3600, // истёк
        }
    }

    fn register(&self, room: &str, nick: &str, avatar: u8) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        let room_map = map.entry(room.to_owned()).or_default();
        // Чистим истёкшие
        let now = now_secs();
        room_map.retain(|_, (ts, _)| now.saturating_sub(*ts) <= 4 * 3600);
        room_map.insert(nick.to_owned(), (now, avatar));
    }
}

// ── Rate limit ────────────────────────────────────────────────
struct RateLimit { map: Mutex<HashMap<String, (u32, u64)>> }

impl RateLimit {
    fn new() -> Self { Self { map: Mutex::new(HashMap::new()) } }

    fn check(&self, ip: &str, max: u32, window: u64) -> bool {
        let now = now_secs();
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > 1000 { map.retain(|_, (_, ts)| now.saturating_sub(*ts) <= window * 2); }
        let e = map.entry(ip.to_owned()).or_insert((0, now));
        if now - e.1 > window { *e = (1, now); return true; }
        if e.0 >= max { return false; }
        e.0 += 1; true
    }
}

// ── Handler ───────────────────────────────────────────────────
struct State {
    cfg:        Config,
    rooms:      Rooms,
    nicks:      Nicks,
    rate_tok:   RateLimit,
    rate_room:  RateLimit,
    rate_check: RateLimit,
}

fn handle(req: &mut tiny_http::Request, method: &Method, path: &str, ip: &str, s: &State)
    -> Response<std::io::Cursor<Vec<u8>>>
{
    let o = s.cfg.allowed_origin.as_str();

    if *method == Method::Options { return options_resp(o); }

    match (method, path) {

        (Method::Get, "/" | "/index.html") =>
            match fs::read(s.cfg.static_dir.join("index.html")) {
                Ok(b) => html_resp(b),
                Err(_) => json_resp(500, serde_json::json!({"error":"not found"}), o),
            },

        (Method::Get, "/favicon.ico") => empty(204),

        (Method::Get, "/api/health") =>
            json_resp(200, serde_json::json!({"ok": true}), o),

        (Method::Get, "/api/config") =>
            json_resp(200, serde_json::json!({
                "livekit_url": s.cfg.livekit_url,
                "version":     env!("CARGO_PKG_VERSION")
            }), o),

        (Method::Get, "/api/rooms") =>
            json_resp(200, serde_json::json!(s.rooms.list()), o),

        (Method::Post, "/api/rooms") => {
            if !s.rate_room.check(ip, 5, 60) {
                return json_resp(429, serde_json::json!({"error":"rate limit"}), o);
            }
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({"error":"too large"}), o);
            };
            let Ok(j) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({"error":"invalid JSON"}), o);
            };
            let name = j["name"].as_str().unwrap_or("").trim().to_owned();
            if !valid_room(&name) {
                return json_resp(400, serde_json::json!({"error":"некорректное название"}), o);
            }
            if !s.rooms.add(name) {
                return json_resp(429, serde_json::json!({"error":"слишком много комнат"}), o);
            }
            json_resp(200, serde_json::json!({"ok": true}), o)
        },

        (Method::Post, "/api/check-nick") => {
            if !s.rate_check.check(ip, 30, 60) {
                return json_resp(429, serde_json::json!({"error":"rate limit"}), o);
            }
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({"error":"too large"}), o);
            };
            let Ok(j) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({"error":"invalid JSON"}), o);
            };
            let room = j["room"].as_str().unwrap_or("").trim().to_owned();
            let nick = j["username"].as_str().unwrap_or("").trim().to_owned();
            if !valid_room(&room) || !valid_name(&nick) {
                return json_resp(400, serde_json::json!({"error":"invalid"}), o);
            }
            let available = s.nicks.is_available(&room, &nick);
            json_resp(200, serde_json::json!({"available": available}), o)
        },

        (Method::Post, "/api/token") => {
            if !s.rate_tok.check(ip, 10, 60) {
                return json_resp(429, serde_json::json!({"error":"rate limit"}), o);
            }
            let Some(body) = read_body(req) else {
                return json_resp(413, serde_json::json!({"error":"too large"}), o);
            };
            let Ok(j) = serde_json::from_str::<serde_json::Value>(&body) else {
                return json_resp(400, serde_json::json!({"error":"invalid JSON"}), o);
            };
            let room     = j["room"].as_str().unwrap_or("").trim().to_owned();
            let username = j["username"].as_str().unwrap_or("").trim().to_owned();
            let avatar   = j["avatar"].as_u64().unwrap_or(0).min(15) as u8;

            if !valid_room(&room)   { return json_resp(400, serde_json::json!({"error":"некорректная комната"}), o); }
            if !valid_name(&username) { return json_resp(400, serde_json::json!({"error":"некорректное имя"}), o); }

            // Double-check availability
            if !s.nicks.is_available(&room, &username) {
                return json_resp(409, serde_json::json!({"error":"ник занят"}), o);
            }

            s.nicks.register(&room, &username, avatar);
            s.rooms.add(room.clone());
            s.rooms.inc(&room);

            let token = build_token(&s.cfg, &room, &username, avatar);
            json_resp(200, serde_json::json!({"token": token}), o)
        },

        (Method::Post, "/api/leave") => {
            // Клиент сообщает что вышел → освобождаем ник и декрементируем счётчик
            let Some(body) = read_body(req) else { return empty(204); };
            if let Ok(j) = serde_json::from_str::<serde_json::Value>(&body) {
                let room = j["room"].as_str().unwrap_or("").trim().to_owned();
                let nick = j["username"].as_str().unwrap_or("").trim().to_owned();
                if valid_room(&room) && valid_name(&nick) {
                    // Инвалидируем ник: сдвигаем ts далеко в прошлое
                    let mut map = s.nicks.map.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(rm) = map.get_mut(&room) {
                        if let Some(entry) = rm.get_mut(&nick) {
                            entry.0 = 0; // expired immediately
                        }
                    }
                    drop(map);
                    s.rooms.dec(&room);
                }
            }
            empty(204)
        },

        _ => json_resp(404, serde_json::json!({"error":"not found"}), o),
    }
}

// ── Main ──────────────────────────────────────────────────────
fn main() {
    let state = Arc::new(State {
        cfg:        Config::from_env(),
        rooms:      Rooms::new(),
        nicks:      Nicks::new(),
        rate_tok:   RateLimit::new(),
        rate_room:  RateLimit::new(),
        rate_check: RateLimit::new(),
    });

    let addr = format!("{}:{}", state.cfg.host, state.cfg.port);
    if !state.cfg.static_dir.exists() {
        eprintln!("[WARN] static dir not found: {:?}", state.cfg.static_dir);
    }

    let server = Arc::new(
        Server::http(&addr).unwrap_or_else(|e| { eprintln!("[ERROR] {e}"); std::process::exit(1) })
    );

    let key_p = &state.cfg.api_key[..state.cfg.api_key.len().min(8)];
    eprintln!("[vchat] http://{}  key={}***  lk={}", addr, key_p, state.cfg.livekit_url);

    let n = std::thread::available_parallelism().map(|n| n.get() * 2).unwrap_or(4).min(32);

    let handles: Vec<_> = (0..n).map(|_| {
        let server = Arc::clone(&server);
        let state  = Arc::clone(&state);
        std::thread::spawn(move || loop {
            let Ok(mut req) = server.recv() else { break };
            let url    = req.url().to_owned();
            let method = req.method().clone();
            let path   = url.split('?').next().unwrap_or("/");
            let ip     = req.remote_addr().map(|a| a.ip().to_string()).unwrap_or_default();
            let resp   = handle(&mut req, &method, path, &ip, &state);
            eprintln!("[{}] {} {}  {}", resp.status_code().0, method, path, ip);
            let _ = req.respond(resp);
        })
    }).collect();

    for h in handles { let _ = h.join(); }
}
