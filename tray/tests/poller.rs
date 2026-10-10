//! The poller against a stand-in for Dexcom Share on 127.0.0.1. Same
//! scenarios as tests/test_g7_share.py, so both clients keep the same rules.

use g7::poller::{self, Credentials, Secrets};
use g7::share::{Client, Region};
use g7::store::Store;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};
use std::thread;

const ACCOUNT_ID: &str = "11111111-2222-3333-4444-555555555555";

#[derive(Default)]
struct Fake {
    password: String,
    sessions: Vec<String>,
    calls: Vec<String>,
    lockout: bool,
    kill_sessions: bool,
    huge: bool,
}

fn serve(fake: Arc<Mutex<Fake>>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/Services", listener.local_addr().unwrap());
    thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { continue };
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            if reader.read_line(&mut line).is_err() {
                continue;
            }
            let target = line.split_whitespace().nth(1).unwrap_or("").to_string();
            let mut len = 0usize;
            loop {
                let mut h = String::new();
                reader.read_line(&mut h).unwrap();
                if h == "\r\n" || h.is_empty() {
                    break;
                }
                if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0; len];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap_or_default();
            let (status, payload) = route(&fake, &target, &body);
            let resp = format!(
                "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                payload.len()
            );
            let _ = stream.write_all(resp.as_bytes());
        }
    });
    base
}

fn fail(code: &str) -> (u16, String) {
    (500, format!("{{\"Code\":\"{code}\",\"Message\":\"nope\"}}"))
}

fn route(fake: &Arc<Mutex<Fake>>, target: &str, body: &serde_json::Value) -> (u16, String) {
    let mut f = fake.lock().unwrap();
    let (path, query) = target.split_once('?').unwrap_or((target, ""));
    f.calls.push(path.trim_start_matches("/Services").to_string());
    match path {
        "/Services/General/AuthenticatePublisherAccount" => {
            if f.lockout {
                return fail("SSO_AuthenticateMaxAttemptsExceeed");
            }
            if body["accountName"] != "kid@example.com" {
                return fail("SSO_AuthenticateAccountNotFound");
            }
            if body["password"] != f.password.as_str() {
                return fail("SSO_AuthenticatePasswordInvalid");
            }
            (200, format!("\"{ACCOUNT_ID}\""))
        }
        "/Services/General/LoginPublisherAccountById" => {
            if body["accountId"] != ACCOUNT_ID || body["password"] != f.password.as_str() {
                return fail("AccountPasswordInvalid");
            }
            let sid = format!("aaaaaaaa-bbbb-cccc-dddd-{:012}", f.sessions.len());
            f.sessions.push(sid.clone());
            (200, format!("\"{sid}\""))
        }
        "/Services/Publisher/ReadPublisherLatestGlucoseValues" => {
            if f.kill_sessions {
                f.sessions.clear();
                f.kill_sessions = false;
            }
            let sid = query.split('&').find_map(|kv| kv.strip_prefix("sessionId=")).unwrap_or("");
            if !f.sessions.iter().any(|s| s == sid) {
                return fail("SessionNotValid");
            }
            if f.huge {
                let one = r#"{"WT":"Date(1760112345000)","Value":100,"Trend":"Flat"}"#;
                return (200, format!("[{}]", vec![one; 9000].join(",")));
            }
            (
                200,
                r#"[{"WT":"Date(1760112345000)","Value":142,"Trend":"Flat"},{"WT":"Date(1760112045000)","Value":138,"Trend":3}]"#.into(),
            )
        }
        _ => fail("Unknown"),
    }
}

struct Env {
    fake: Arc<Mutex<Fake>>,
    base: String,
    store: Store,
    dir: std::path::PathBuf,
    secret: RefCell<Option<String>>,
}

impl Env {
    fn new() -> Env {
        let fake = Arc::new(Mutex::new(Fake { password: "correct horse".into(), ..Default::default() }));
        let base = serve(fake.clone());
        let dir = std::env::temp_dir().join(format!("g7-poller-{}-{:?}", std::process::id(), thread::current().id()));
        let _ = std::fs::remove_dir_all(&dir);
        let store = Store::at(dir.clone()).unwrap();
        Env { fake, base, store, dir, secret: RefCell::new(None) }
    }

    fn calls(&self) -> Vec<String> {
        std::mem::take(&mut self.fake.lock().unwrap().calls)
    }

    fn with<R>(&self, f: impl FnOnce(&dyn Fn(Region) -> Client, &Secrets) -> R) -> R {
        let base = self.base.clone();
        let client_for = move |_r: Region| Client::with_base(&base);
        let load = || Ok(self.secret.borrow().clone());
        let save = |p: &str| {
            *self.secret.borrow_mut() = Some(p.to_string());
            Ok(())
        };
        let secrets = Secrets { load: &load, save: &save };
        f(&client_for, &secrets)
    }

    fn sign_in(&self, password: &str) -> Result<poller::Snapshot, String> {
        self.with(|c, s| {
            poller::sign_in_with(
                &self.store,
                c,
                s,
                Credentials {
                    region: Region::Us,
                    account_name: Some("kid@example.com".into()),
                    account_id: None,
                    password: password.into(),
                },
            )
        })
    }

    fn poll(&self) -> poller::Snapshot {
        self.with(|c, s| poller::poll_with(&self.store, c, s, 180))
    }
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn sign_in_then_poll_reuses_the_session() {
    let env = Env::new();
    let snap = env.sign_in("correct horse").unwrap();
    assert_eq!(snap.account, "k***@example.com");
    assert_eq!(snap.latest().unwrap().v, 142);
    assert_eq!(env.secret.borrow().as_deref(), Some("correct horse"));
    env.calls();
    let snap = env.poll();
    assert!(snap.live);
    assert_eq!(snap.readings.len(), 2);
    assert_eq!(env.calls(), ["/Publisher/ReadPublisherLatestGlucoseValues"]);
}

#[test]
fn wrong_password_is_not_saved() {
    let env = Env::new();
    let err = env.sign_in("nope").unwrap_err();
    assert!(err.contains("password"), "{err}");
    assert!(env.secret.borrow().is_none());
    assert!(!env.store.load_settings().has_account());
}

#[test]
fn expired_session_logs_in_once() {
    let env = Env::new();
    env.sign_in("correct horse").unwrap();
    env.fake.lock().unwrap().kill_sessions = true;
    env.calls();
    let snap = env.poll();
    assert!(snap.live, "{}", snap.message);
    assert_eq!(
        env.calls(),
        [
            "/Publisher/ReadPublisherLatestGlucoseValues",
            "/General/LoginPublisherAccountById",
            "/Publisher/ReadPublisherLatestGlucoseValues"
        ]
    );
}

#[test]
fn changed_password_stops_all_authentication() {
    let env = Env::new();
    env.sign_in("correct horse").unwrap();
    {
        let mut f = env.fake.lock().unwrap();
        f.password = "changed".into();
        f.kill_sessions = true;
    }
    let snap = env.poll();
    assert!(snap.needs_login);
    assert_eq!(snap.latest().unwrap().v, 142, "the cached reading stays visible");
    env.calls();
    let snap = env.poll();
    assert!(snap.needs_login);
    assert!(env.calls().is_empty(), "no request until the person signs in again");
    env.sign_in("changed").unwrap();
    assert!(env.poll().live);
}

#[test]
fn lockout_backs_off_and_blocks_sign_in() {
    let env = Env::new();
    env.sign_in("correct horse").unwrap();
    {
        let mut f = env.fake.lock().unwrap();
        f.lockout = true;
        f.kill_sessions = true;
    }
    // The cached account ID skips authenticate, so the lockout shows up when
    // the account has to be looked up again; clear it the way a fresh install would.
    let mut st = env.store.load_state();
    st.session_account_id = None;
    env.store.save_state(&st).unwrap();
    let snap = env.poll();
    assert!(!snap.live);
    assert!(snap.backoff_until > g7::model::now() + 29 * 60);
    env.calls();
    env.poll();
    assert!(env.calls().is_empty());
    let err = env.sign_in("correct horse").unwrap_err();
    assert!(err.contains("locked"), "{err}");
    assert!(env.calls().is_empty(), "sign-in must not extend a lockout");
}

#[test]
fn oversize_response_is_refused() {
    let env = Env::new();
    env.sign_in("correct horse").unwrap();
    env.fake.lock().unwrap().huge = true;
    let snap = env.poll();
    assert!(!snap.live);
    assert!(snap.message.contains("more data"), "{}", snap.message);
}
