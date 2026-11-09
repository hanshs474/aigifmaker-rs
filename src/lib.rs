//! Turn text into animated GIFs with AI GIF Maker. Native GIF generation — no
//! video conversion. Type what you want and download your GIF in about a minute.
//!
//! This is the official Rust SDK for [AI GIF Maker](https://aigifmaker.ai/?utm_source=crates&utm_medium=package):
//! send a text prompt, get back the URL of a real animated GIF.
//!
//! # Quick start
//!
//! Create an API key at <https://aigifmaker.ai/settings/apikeys> and export it as
//! `AIGIFMAKER_API_KEY`. Then:
//!
//! ```no_run
//! let url = aigifmaker::generate("a cat dancing in the rain, pixel art")?;
//! println!("{url}");
//! # Ok::<(), aigifmaker::Error>(())
//! ```
//!
//! [`generate`] submits the prompt, polls every 3 seconds until the GIF is ready
//! (15 minutes at most) and returns its URL.
//!
//! # A client of your own
//!
//! ```no_run
//! use std::time::Duration;
//!
//! let mut client = aigifmaker::Client::new("sk-...");   // or Client::from_env()
//! client.timeout = Duration::from_secs(600);
//!
//! let left = client.credits()?;                         // free to call
//! let id = client.submit("a rocket launching, retro cartoon")?;
//! let status = client.status(&id)?;                     // pending | processing | success | failed
//! if let Some(url) = status.url() {
//!     println!("{url}");
//! }
//! let url = client.wait(&id)?;                          // or poll until it is done
//! # Ok::<(), aigifmaker::Error>(())
//! ```
//!
//! On an account that has not bought credits yet, a GIF can wait in a queue first:
//! [`Status::queued`] is `true` and [`Status::queue_eta`] says how many seconds are
//! left. [`Client::wait`] and [`Client::generate`] keep polling through it.
//!
//! # Credits
//!
//! Each GIF costs 1 credit on your AI GIF Maker account, the same as on the website.
//! New accounts get one free GIF (it carries a small aigifmaker.ai badge). Failed
//! generations are refunded. Credits are on the
//! [pricing](https://aigifmaker.ai/pricing?utm_source=crates&utm_medium=package) page.
//!
//! # Errors
//!
//! Every call returns [`Error`], split by what to do next: [`Error::Auth`] (no key,
//! or a wrong one), [`Error::Quota`] (out of credits), [`Error::Rejected`]
//! (moderation said no; reword the prompt), [`Error::Failed`] (the GIF failed and
//! the credit came back), [`Error::Timeout`] and [`Error::Api`] for anything else.
//!
//! ```no_run
//! match aigifmaker::generate("a cat waving") {
//!     Ok(url) => println!("{url}"),
//!     Err(aigifmaker::Error::Quota(_)) => eprintln!("top up at https://aigifmaker.ai/pricing"),
//!     Err(e) => eprintln!("{e}"),
//! }
//! ```
//!
//! Blocking HTTP via [`ureq`]; the only other dependency is `serde_json`.

use serde_json::{json, Value};
use std::fmt;
use std::thread::sleep;
use std::time::{Duration, Instant};

/// The service this crate talks to.
pub const BASE_URL: &str = "https://aigifmaker.ai";
/// Where an API key is created.
pub const KEYS_URL: &str = "https://aigifmaker.ai/settings/apikeys";
/// Where credits are bought.
pub const PRICING_URL: &str = "https://aigifmaker.ai/pricing";
/// The environment variable [`Client::from_env`] reads.
pub const ENV_KEY: &str = "AIGIFMAKER_API_KEY";

/// What went wrong, split by what to do next.
#[derive(Debug, Clone, PartialEq)]
pub enum Error {
    /// No API key, or the key is invalid or deleted. Create one at [`KEYS_URL`].
    Auth(String),
    /// The account is out of credits. Buy more at [`PRICING_URL`].
    Quota(String),
    /// Moderation refused the prompt, or the account is cooling down after a refusal. Reword it.
    Rejected(String),
    /// The GIF was accepted and then failed; carries the service's message. The credit is refunded.
    Failed(String),
    /// The wait ran out; the GIF may still finish in your history on the site. Carries the task id.
    Timeout(String),
    /// Anything else: transport failure or an unexpected answer, with the message verbatim.
    Api(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Auth(m) if m.contains("settings/apikeys") => write!(f, "aigifmaker: {m}"),
            Error::Auth(m) => write!(f, "aigifmaker: {m} (create a key at {KEYS_URL})"),
            Error::Quota(m) => write!(f, "aigifmaker: {m} (buy credits at {PRICING_URL})"),
            Error::Rejected(m) => write!(f, "aigifmaker: {m} (reword the prompt rather than retrying)"),
            Error::Failed(m) => write!(f, "aigifmaker: generation failed: {m}"),
            Error::Timeout(id) => write!(f, "aigifmaker: deadline passed for task {id}; it may still finish in your history"),
            Error::Api(m) => write!(f, "aigifmaker: {m}"),
        }
    }
}

impl std::error::Error for Error {}

/// One task, as `POST /api/ai/query` reports it.
#[derive(Clone, Debug)]
pub struct Status {
    /// The task id.
    pub id: String,
    /// `pending`, `processing`, `success`, `failed` or `canceled`.
    pub status: String,
    /// The finished GIF URLs (one) once `status` is `success`.
    pub images: Vec<String>,
    /// `true` while the task waits in the queue for accounts that have not bought credits yet.
    pub queued: bool,
    /// Seconds left in the queue, when `queued`.
    pub queue_eta: Option<f64>,
    /// Why the task failed, when it did.
    pub error_message: Option<String>,
    /// Failure category, e.g. `moderation`.
    pub failure_kind: Option<String>,
    /// The whole `data` object as the API sent it.
    pub raw: Value,
}

impl Status {
    /// The finished GIF URL, or `None` while the task runs.
    pub fn url(&self) -> Option<&str> {
        self.images.first().map(String::as_str)
    }
}

/// A client bound to one API key.
#[derive(Clone, Debug)]
pub struct Client {
    key: String,
    base: String,
    /// Time between status checks. Default 3 seconds.
    pub poll_every: Duration,
    /// Total wait for one GIF. Default 15 minutes.
    pub timeout: Duration,
}

/// The exact JSON body sent to `POST /api/ai/generate`. Only the prompt varies.
pub fn request_body(prompt: &str) -> Value {
    json!({
        "provider": "openlux",
        "mediaType": "image",
        "model": "gif-maker-v1",
        "scene": "text-to-image",
        "prompt": prompt,
        "options": { "output_format": "gif" }
    })
}

impl Client {
    /// A client for the given API key (`sk-...`). An empty key sends no
    /// `Authorization` header, and the service answers with [`Error::Auth`].
    pub fn new(api_key: impl Into<String>) -> Self {
        Client { key: api_key.into().trim().to_string(), base: BASE_URL.into(), poll_every: Duration::from_secs(3), timeout: Duration::from_secs(900) }
    }

    /// A client reading its key from `AIGIFMAKER_API_KEY`.
    pub fn from_env() -> Self {
        Client::new(std::env::var(ENV_KEY).unwrap_or_default())
    }

    /// Point the client at another host (tests, proxies).
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into().trim_end_matches('/').into();
        self
    }

    /// Start a GIF and return its task id without waiting.
    pub fn submit(&self, prompt: &str) -> Result<String, Error> {
        if prompt.trim().is_empty() {
            return Err(Error::Api("prompt is required".into()));
        }
        parse_submit(&self.post("/api/ai/generate", Some(request_body(prompt)))?)
    }

    /// Look up a task by the id [`Client::submit`] returned.
    pub fn status(&self, id: &str) -> Result<Status, Error> {
        parse_status(&self.post("/api/ai/query", Some(json!({ "taskId": id })))?)
    }

    /// Credits left on the account. Free to call.
    pub fn credits(&self) -> Result<i64, Error> {
        parse_credits(&self.post("/api/user/get-user-credits", None)?)
    }

    /// Submit, wait, and return the GIF URL.
    pub fn generate(&self, prompt: &str) -> Result<String, Error> {
        let id = self.submit(prompt)?;
        self.wait(&id)
    }

    /// Poll a submitted task until it finishes and return the GIF URL.
    pub fn wait(&self, id: &str) -> Result<String, Error> {
        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            sleep(self.poll_every);
            let st = match self.status(id) {
                Ok(s) => s,
                Err(e @ Error::Auth(_)) => return Err(e),
                Err(_) => continue, // a dropped poll is not a failed run
            };
            if let Some(url) = finished(&st)? {
                return Ok(url);
            }
        }
        Err(Error::Timeout(id.into()))
    }

    fn post(&self, path: &str, body: Option<Value>) -> Result<Value, Error> {
        let mut req = ureq::post(&format!("{}{path}", self.base)).set("User-Agent", concat!("aigifmaker-rust/", env!("CARGO_PKG_VERSION")));
        if !self.key.is_empty() {
            req = req.set("Authorization", &format!("Bearer {}", self.key));
        }
        let resp = match body {
            Some(b) => req.send_json(b),
            None => req.send_bytes(&[]),
        };
        match resp {
            Ok(r) => r.into_json().map_err(|e| Error::Api(format!("unreadable answer: {e}"))),
            Err(ureq::Error::Status(code, r)) => Ok(r.into_json().unwrap_or_else(|_| json!({ "code": -1, "message": format!("HTTP {code}") }))),
            Err(e) => Err(Error::Api(e.to_string())),
        }
    }
}

/// `Some(url)` when the task is done, an error when it failed, `None` while it runs.
pub fn finished(st: &Status) -> Result<Option<String>, Error> {
    match st.status.as_str() {
        "success" => match st.url() {
            Some(u) => Ok(Some(u.to_string())),
            None => Err(Error::Api("finished without a GIF URL".into())),
        },
        "failed" | "canceled" => {
            let msg = st.error_message.clone().unwrap_or_else(|| st.status.clone());
            if st.failure_kind.as_deref() == Some("moderation") {
                Err(Error::Rejected(msg))
            } else {
                Err(Error::Failed(msg))
            }
        }
        _ => Ok(None),
    }
}

/// Map a non-zero JSON `code` and its `message` to an [`Error`].
pub fn classify(code: i64, message: &str) -> Error {
    let low = message.to_lowercase();
    let msg = if message.is_empty() { "request failed".to_string() } else { message.to_string() };
    if code != -1 {
        Error::Rejected(msg) // content cooldown and similar
    } else if low.contains("invalid api key") || low.contains("no auth") {
        Error::Auth(msg)
    } else if low.contains("insufficient credits") {
        Error::Quota(msg)
    } else {
        Error::Api(msg)
    }
}

/// Unwrap `{"code":0,"message":"ok","data":{…}}` into `data`, or the [`Error`] the answer stands for.
pub fn envelope(body: &Value) -> Result<Value, Error> {
    let code = body.get("code").and_then(Value::as_i64).ok_or_else(|| Error::Api("unexpected answer".into()))?;
    if code != 0 {
        return Err(classify(code, body.get("message").and_then(Value::as_str).unwrap_or("")));
    }
    let data = body.get("data").cloned().unwrap_or_else(|| json!({}));
    if data.get("wall").and_then(Value::as_bool) == Some(true) {
        return Err(Error::Auth(format!("an API key is required; create one at {KEYS_URL}")));
    }
    Ok(data)
}

/// Read the answer of `POST /api/ai/generate` and return the task id.
pub fn parse_submit(body: &Value) -> Result<String, Error> {
    let d = envelope(body)?;
    d.get("id").and_then(Value::as_str).filter(|s| !s.is_empty()).map(String::from).ok_or_else(|| Error::Api("the service returned no task id".into()))
}

/// Read the answer of `POST /api/ai/query`.
pub fn parse_status(body: &Value) -> Result<Status, Error> {
    let d = envelope(body)?;
    let s = |k: &str| d.get(k).and_then(Value::as_str).map(String::from);
    Ok(Status {
        id: s("id").unwrap_or_default(),
        status: s("status").unwrap_or_default(),
        images: d.get("images").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect()).unwrap_or_default(),
        queued: d.get("queued").and_then(Value::as_bool).unwrap_or(false),
        queue_eta: d.get("queueEta").and_then(Value::as_f64),
        error_message: s("errorMessage"),
        failure_kind: s("failureKind"),
        raw: d,
    })
}

/// Read the answer of `POST /api/user/get-user-credits`.
pub fn parse_credits(body: &Value) -> Result<i64, Error> {
    envelope(body)?.get("remainingCredits").and_then(Value::as_i64).ok_or_else(|| Error::Api("no remainingCredits in the answer".into()))
}

/// Turn a prompt into an animated GIF with the key in `AIGIFMAKER_API_KEY` and return its URL.
pub fn generate(prompt: &str) -> Result<String, Error> {
    Client::from_env().generate(prompt)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    /// A one-thread HTTP server that answers each request with the next canned body
    /// (all HTTP 200, like the real API) and records the request bodies.
    fn mock(replies: Vec<&'static str>) -> (Client, Arc<Mutex<Vec<(String, String)>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        std::thread::spawn(move || {
            for (reply, stream) in replies.into_iter().zip(l.incoming()) {
                let mut s = stream.unwrap();
                let mut r = BufReader::new(s.try_clone().unwrap());
                let (mut line, mut path, mut len) = (String::new(), String::new(), 0usize);
                while r.read_line(&mut line).unwrap() > 0 {
                    if path.is_empty() {
                        path = line.split_whitespace().nth(1).unwrap_or("").to_string();
                    } else if let Some(v) = line.to_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    } else if line == "\r\n" {
                        break;
                    }
                    line.clear();
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                log.lock().unwrap().push((path, String::from_utf8(body).unwrap()));
                write!(s, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", reply.len(), reply).unwrap();
            }
        });
        let mut c = Client::new("sk-test").with_base(base);
        c.poll_every = Duration::from_millis(1);
        (c, seen)
    }

    #[test]
    fn success_flow_sends_the_fixed_body() {
        let (c, seen) = mock(vec![
            r#"{"code":0,"message":"ok","data":{"id":"t1","status":"pending","taskId":"up-9","provider":"openlux"}}"#,
            r#"{"code":0,"message":"ok","data":{"id":"t1","status":"processing","images":[]}}"#,
            r#"{"code":0,"message":"ok","data":{"id":"t1","status":"success","images":["https://cdn.aigifmaker.ai/t1.gif"],"watermarked":[false]}}"#,
            r#"{"code":0,"message":"ok","data":{"remainingCredits":7,"made":3}}"#,
        ]);
        assert_eq!(c.generate("a cat dancing").unwrap(), "https://cdn.aigifmaker.ai/t1.gif");
        assert_eq!(c.credits().unwrap(), 7);
        let seen = seen.lock().unwrap();
        assert_eq!(seen[0].0, "/api/ai/generate");
        let sent: Value = serde_json::from_str(&seen[0].1).unwrap();
        assert_eq!(sent, request_body("a cat dancing"));
        assert_eq!(sent["model"], "gif-maker-v1");
        assert_eq!(sent["options"]["output_format"], "gif");
        assert_eq!(seen[1].0, "/api/ai/query");
        assert_eq!(serde_json::from_str::<Value>(&seen[1].1).unwrap(), json!({"taskId": "t1"}));
        assert_eq!(seen[3].0, "/api/user/get-user-credits");
    }

    #[test]
    fn queued_keeps_polling() {
        let (c, _) = mock(vec![
            r#"{"code":0,"message":"ok","data":{"id":"t1","status":"processing","images":[],"queued":true,"queueEta":40}}"#,
            r#"{"code":0,"message":"ok","data":{"id":"t1","status":"success","images":["https://cdn.aigifmaker.ai/q.gif"],"watermarked":[true]}}"#,
        ]);
        assert_eq!(c.wait("t1").unwrap(), "https://cdn.aigifmaker.ai/q.gif");
        let st = parse_status(&json!({"code":0,"message":"ok","data":{"id":"t1","status":"processing","images":[],"queued":true,"queueEta":40}})).unwrap();
        assert!(st.queued && st.queue_eta == Some(40.0) && finished(&st).unwrap().is_none());
    }

    #[test]
    fn failed_and_moderation() {
        let st = parse_status(&json!({"code":0,"message":"ok","data":{"id":"t1","status":"failed","images":[],"errorMessage":"upstream error","refundNote":"credit refunded"}})).unwrap();
        assert_eq!(finished(&st), Err(Error::Failed("upstream error".into())));
        let st = parse_status(&json!({"code":0,"data":{"id":"t1","status":"failed","errorMessage":"blocked","failureKind":"moderation"}})).unwrap();
        assert!(matches!(finished(&st), Err(Error::Rejected(_))));
    }

    #[test]
    fn invalid_key() {
        let body = json!({"code":-1,"message":"invalid API key — create one at https://aigifmaker.ai/settings/apikeys"});
        assert!(matches!(parse_credits(&body), Err(Error::Auth(_))));
        assert!(matches!(parse_credits(&json!({"code":-1,"message":"no auth, please sign in"})), Err(Error::Auth(_))));
    }

    #[test]
    fn wall_is_auth() {
        let body = json!({"code":0,"data":{"wall":true,"reason":"anon_credits","cost":1,"remaining":0,"grant":0}});
        assert!(matches!(parse_submit(&body), Err(Error::Auth(m)) if m.contains("settings/apikeys")));
    }

    #[test]
    fn insufficient_credits() {
        assert!(matches!(parse_submit(&json!({"code":-1,"message":"insufficient credits"})), Err(Error::Quota(_))));
    }

    #[test]
    fn cooldown_generic_and_empty_prompt() {
        assert!(matches!(parse_submit(&json!({"code":429,"message":"please wait"})), Err(Error::Rejected(_))));
        assert_eq!(parse_submit(&json!({"code":-1,"message":"something else broke"})), Err(Error::Api("something else broke".into())));
        assert!(matches!(Client::new("sk-x").submit("  "), Err(Error::Api(_))));
    }

    /// Live and free (no generation): `cargo test -- --ignored`
    #[test]
    #[ignore]
    fn live_smoke() {
        assert!(matches!(Client::new("sk-invalidtest000").credits(), Err(Error::Auth(_))));
        assert!(matches!(Client::new("").submit("a cat waving"), Err(Error::Auth(_))));
    }
}
