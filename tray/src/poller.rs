//! One poll of Dexcom Share, with the session cache and the back-off rules
//! from docs/share-api.md. The tray app and the `g7` CLI both call this.

use crate::model::now;
use crate::share::{Client, Reading, Region, ShareError, LOCKOUT_SECONDS, TRANSIENT_BACKOFF_SECONDS};
use crate::store::{self, Settings, State, Store};

/// What the UI needs to draw. Built from a poll, or from the cache alone.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    pub signed_in: bool,
    pub needs_login: bool,
    pub live: bool,
    pub account: String,
    pub message: String,
    pub readings: Vec<Reading>,
    pub fetched_at: i64,
    pub backoff_until: i64,
}

impl Snapshot {
    pub fn latest(&self) -> Option<&Reading> {
        self.readings.first()
    }
}

fn cached(settings: &Settings, state: &State, message: String, needs_login: bool) -> Snapshot {
    Snapshot {
        signed_in: settings.has_account(),
        needs_login,
        live: false,
        account: settings.account_label(),
        message,
        readings: state.readings.clone(),
        fetched_at: state.fetched_at,
        backoff_until: state.backoff_until,
    }
}

/// Where the password lives. The apps use the system credential store; the
/// tests use memory.
pub struct Secrets<'a> {
    pub load: &'a dyn Fn() -> Result<Option<String>, String>,
    pub save: &'a dyn Fn(&str) -> Result<(), String>,
}

pub const KEYCHAIN: Secrets<'static> = Secrets { load: &store::load_password, save: &store::save_password };

/// Poll once. Never authenticates while a password failure or a Dexcom
/// back-off is in force, because every failed attempt counts towards a
/// lockout that also hits the wearer's phone.
pub fn poll(store: &Store, client_for: &dyn Fn(Region) -> Client, minutes: u32) -> Snapshot {
    poll_with(store, client_for, &KEYCHAIN, minutes)
}

pub fn poll_with(store: &Store, client_for: &dyn Fn(Region) -> Client, secrets: &Secrets, minutes: u32) -> Snapshot {
    let settings = store.load_settings();
    let mut state = store.load_state();
    if !settings.has_account() {
        return Snapshot { message: "Not signed in.".into(), ..Default::default() };
    }
    if state.auth_failed {
        let msg = if state.auth_message.is_empty() { "Sign in again.".to_string() } else { state.auth_message.clone() };
        return cached(&settings, &state, msg, true);
    }
    if state.backoff_until > now() {
        return cached(&settings, &state, state.backoff_message.clone(), false);
    }
    let password = match (secrets.load)() {
        Ok(Some(p)) => p,
        Ok(None) => return cached(&settings, &state, "The saved password is missing. Sign in again.".into(), true),
        Err(e) => return cached(&settings, &state, e, false),
    };

    let client = client_for(settings.region);
    match fetch(&client, &settings, &mut state, &password, minutes) {
        Ok(readings) => {
            state.readings = readings;
            state.fetched_at = now();
            state.backoff_until = 0;
            state.backoff_message.clear();
            state.backoff_lockout = false;
            state.last_error.clear();
            let _ = store.save_state(&state);
            Snapshot {
                signed_in: true,
                needs_login: false,
                live: true,
                account: settings.account_label(),
                message: String::new(),
                readings: state.readings.clone(),
                fetched_at: state.fetched_at,
                backoff_until: 0,
            }
        }
        Err(err) => {
            let msg = err.message();
            match &err {
                ShareError::BadCredentials(_) => {
                    state.auth_failed = true;
                    state.auth_message = msg.clone();
                    state.session_id = None;
                }
                ShareError::Lockout => {
                    state.backoff_until = now() + LOCKOUT_SECONDS;
                    state.backoff_message = msg.clone();
                    state.backoff_lockout = true;
                }
                ShareError::Transient(_) | ShareError::Dexcom(_) | ShareError::SessionGone => {
                    state.backoff_until = now() + TRANSIENT_BACKOFF_SECONDS;
                    state.backoff_message = msg.clone();
                }
            }
            state.last_error = msg.clone();
            let _ = store.save_state(&state);
            cached(&settings, &state, msg, err.needs_login())
        }
    }
}

fn fetch(client: &Client, settings: &Settings, state: &mut State, password: &str, minutes: u32) -> Result<Vec<Reading>, ShareError> {
    let (sid, fresh) = match &state.session_id {
        Some(sid) if crate::share::is_uuid(sid) => (sid.clone(), false),
        _ => (new_session(client, settings, state, password)?, true),
    };
    match client.readings(&sid, minutes, 288) {
        Err(ShareError::SessionGone) if !fresh => {
            state.session_id = None;
            let sid = new_session(client, settings, state, password)?;
            client.readings(&sid, minutes, 288)
        }
        other => other,
    }
}

fn new_session(client: &Client, settings: &Settings, state: &mut State, password: &str) -> Result<String, ShareError> {
    let account_id = match settings.account_id.as_deref().filter(|id| crate::share::is_uuid(id)) {
        Some(id) => id.to_string(),
        None => {
            let name = settings.account_name.as_deref().unwrap_or_default();
            match state.session_account_id.as_deref().filter(|id| crate::share::is_uuid(id)) {
                Some(id) => id.to_string(),
                None => client.authenticate(name, password)?,
            }
        }
    };
    let sid = client.login(&account_id, password)?;
    state.session_id = Some(sid.clone());
    state.session_account_id = Some(account_id);
    Ok(sid)
}

pub struct Credentials {
    pub region: Region,
    pub account_name: Option<String>,
    pub account_id: Option<String>,
    pub password: String,
}

/// Validate, try the credentials against Dexcom, and only then save them.
pub fn sign_in(store: &Store, client_for: &dyn Fn(Region) -> Client, creds: Credentials) -> Result<Snapshot, String> {
    sign_in_with(store, client_for, &KEYCHAIN, creds)
}

pub fn sign_in_with(store: &Store, client_for: &dyn Fn(Region) -> Client, secrets: &Secrets, creds: Credentials) -> Result<Snapshot, String> {
    let name = creds.account_name.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string);
    let id = creds.account_id.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_ascii_lowercase);
    if let Some(id) = &id {
        if !crate::share::is_uuid(id) {
            return Err("That is not an account ID. It looks like 8-4-4-4-12 letters and digits.".into());
        }
    }
    if name.is_none() && id.is_none() {
        return Err("An account name or account ID is required.".into());
    }
    if let Some(n) = &name {
        if n.chars().count() > 200 || n.chars().any(char::is_control) {
            return Err("That account name is not valid.".into());
        }
    }
    if creds.password.is_empty() || creds.password.len() > 200 || creds.password.chars().any(|c| c == '\r' || c == '\n' || c == '\0') {
        return Err("A password is required.".into());
    }
    let mut state = store.load_state();
    if state.backoff_lockout && state.backoff_until > now() {
        let minutes = ((state.backoff_until - now()) + 59) / 60;
        return Err(format!("Dexcom locked sign-in after too many attempts. Try again in {minutes} minutes."));
    }

    let mut settings = store.load_settings();
    settings.region = creds.region;
    settings.account_name = name;
    settings.account_id = id;
    let mut fresh = State::default();
    let client = client_for(settings.region);
    let result = new_session(&client, &settings, &mut fresh, &creds.password).and_then(|sid| client.readings(&sid, 1440, 1));
    let readings = match result {
        Ok(r) => r,
        Err(e) => {
            if e == ShareError::Lockout {
                state.backoff_until = now() + LOCKOUT_SECONDS;
                state.backoff_message = e.message();
                state.backoff_lockout = true;
                let _ = store.save_state(&state);
            }
            return Err(e.message());
        }
    };
    (secrets.save)(&creds.password)?;
    store.save_settings(&settings).map_err(|e| format!("Could not save settings: {e}"))?;
    fresh.readings = readings;
    fresh.fetched_at = now();
    store.save_state(&fresh).map_err(|e| format!("Could not save state: {e}"))?;
    Ok(Snapshot {
        signed_in: true,
        needs_login: false,
        live: true,
        account: settings.account_label(),
        message: String::new(),
        readings: fresh.readings.clone(),
        fetched_at: fresh.fetched_at,
        backoff_until: 0,
    })
}

/// When to poll next, in seconds.
pub fn next_poll_secs(snap: &Snapshot, refresh_sec: u64) -> u64 {
    if !snap.signed_in || snap.needs_login {
        return 30;
    }
    let wait = snap.backoff_until - now();
    if wait > 0 {
        return refresh_sec.max(wait as u64 + 1);
    }
    refresh_sec
}
