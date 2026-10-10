//! Dexcom Share client. The protocol is written down in docs/share-api.md at
//! the repository root; bin/g7-share is the Python twin of this file.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

pub const APP_ID: &str = "d89443d2-327c-4a6f-89e5-496bbb0317db";
pub const NULL_UUID: &str = "00000000-0000-0000-0000-000000000000";
const AUTH_MAX_BYTES: u64 = 4096;
const READ_MAX_BYTES: u64 = 256 * 1024;
const USER_AGENT: &str = concat!("G7-Companion/", env!("CARGO_PKG_VERSION"), " (+https://github.com/ianswope/g7-companion)");

/// Seconds Dexcom's sign-in lockout is assumed to last.
pub const LOCKOUT_SECONDS: i64 = 30 * 60;
/// Seconds to wait after a network or server failure.
pub const TRANSIENT_BACKOFF_SECONDS: i64 = 90;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Region {
    #[default]
    Us,
    Ous,
    Jp,
}

impl Region {
    pub fn base(self) -> &'static str {
        match self {
            Region::Us => "https://share2.dexcom.com/ShareWebServices/Services",
            Region::Ous => "https://shareous1.dexcom.com/ShareWebServices/Services",
            Region::Jp => "https://share.dexcom.jp/ShareWebServices/Services",
        }
    }

    pub fn parse(s: &str) -> Option<Region> {
        match s.trim().to_ascii_lowercase().as_str() {
            "us" | "united states" | "usa" => Some(Region::Us),
            "ous" | "outside the us" | "outside" | "intl" => Some(Region::Ous),
            "jp" | "japan" => Some(Region::Jp),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Region::Us => "United States",
            Region::Ous => "Outside the US",
            Region::Jp => "Japan",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Trend {
    #[default]
    None,
    DoubleUp,
    SingleUp,
    FortyFiveUp,
    Flat,
    FortyFiveDown,
    SingleDown,
    DoubleDown,
    NotComputable,
    RateOutOfRange,
}

impl Trend {
    pub fn from_value(v: &Value) -> Trend {
        match v {
            Value::Number(n) => match n.as_i64() {
                Some(1) => Trend::DoubleUp,
                Some(2) => Trend::SingleUp,
                Some(3) => Trend::FortyFiveUp,
                Some(4) => Trend::Flat,
                Some(5) => Trend::FortyFiveDown,
                Some(6) => Trend::SingleDown,
                Some(7) => Trend::DoubleDown,
                Some(8) => Trend::NotComputable,
                Some(9) => Trend::RateOutOfRange,
                _ => Trend::None,
            },
            Value::String(s) => match s.as_str() {
                "DoubleUp" => Trend::DoubleUp,
                "SingleUp" => Trend::SingleUp,
                "FortyFiveUp" => Trend::FortyFiveUp,
                "Flat" => Trend::Flat,
                "FortyFiveDown" => Trend::FortyFiveDown,
                "SingleDown" => Trend::SingleDown,
                "DoubleDown" => Trend::DoubleDown,
                "NotComputable" => Trend::NotComputable,
                "RateOutOfRange" => Trend::RateOutOfRange,
                _ => Trend::None,
            },
            _ => Trend::None,
        }
    }

    pub fn arrow(self) -> &'static str {
        match self {
            Trend::None => "",
            Trend::DoubleUp => "⇈",
            Trend::SingleUp => "↑",
            Trend::FortyFiveUp => "↗",
            Trend::Flat => "→",
            Trend::FortyFiveDown => "↘",
            Trend::SingleDown => "↓",
            Trend::DoubleDown => "⇊",
            Trend::NotComputable | Trend::RateOutOfRange => "?",
        }
    }

    pub fn words(self) -> &'static str {
        match self {
            Trend::None => "no trend",
            Trend::DoubleUp => "rising fast",
            Trend::SingleUp => "rising",
            Trend::FortyFiveUp => "rising slowly",
            Trend::Flat => "steady",
            Trend::FortyFiveDown => "falling slowly",
            Trend::SingleDown => "falling",
            Trend::DoubleDown => "falling fast",
            Trend::NotComputable => "trend not computable",
            Trend::RateOutOfRange => "rate out of range",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reading {
    /// Seconds since the epoch, UTC.
    pub t: i64,
    /// mg/dL.
    pub v: i64,
    pub trend: Trend,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShareError {
    /// The session ID is no longer valid. Log in again once.
    SessionGone,
    /// Wrong account name or password. Stop until the user signs in again.
    BadCredentials(String),
    /// Dexcom locked sign-in. Do not try again for a while.
    Lockout,
    /// Network, timeout, oversize or malformed response. Try again later.
    Transient(String),
    /// Any other Dexcom error code.
    Dexcom(String),
}

impl ShareError {
    pub fn message(&self) -> String {
        match self {
            ShareError::SessionGone => "Dexcom session expired.".into(),
            ShareError::BadCredentials(m) => m.clone(),
            ShareError::Lockout => "Too many sign-in attempts. Dexcom has locked sign-in for a while.".into(),
            ShareError::Transient(m) => m.clone(),
            ShareError::Dexcom(code) => format!("Dexcom returned {code}."),
        }
    }

    pub fn needs_login(&self) -> bool {
        matches!(self, ShareError::BadCredentials(_))
    }
}

impl std::fmt::Display for ShareError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for ShareError {}

pub struct Client {
    agent: ureq::Agent,
    base: String,
}

impl Client {
    pub fn new(region: Region) -> Client {
        Client::with_base(region.base())
    }

    /// A client for an arbitrary base URL. Only the tests use this; the apps
    /// always go through `Client::new` and the fixed Dexcom origins.
    pub fn with_base(base: &str) -> Client {
        let agent = ureq::AgentBuilder::new()
            .redirects(0)
            .timeout_connect(Duration::from_secs(5))
            .timeout(Duration::from_secs(10))
            .user_agent(USER_AGENT)
            .build();
        Client { agent, base: base.trim_end_matches('/').to_string() }
    }

    fn post(&self, path: &str, body: Option<&Value>, max: u64) -> Result<Value, ShareError> {
        let url = format!("{}{}", self.base, path);
        let req = self.agent.post(&url).set("Accept", "application/json");
        let result = match body {
            Some(b) => req.set("Content-Type", "application/json").send_string(&b.to_string()),
            None => req.send_bytes(&[]),
        };
        match result {
            Ok(resp) => {
                let status = resp.status();
                if status != 200 {
                    return Err(ShareError::Transient(format!("Dexcom answered with HTTP {status}.")));
                }
                let raw = read_capped(resp, max)?;
                if raw.iter().all(|b| b.is_ascii_whitespace()) {
                    return Ok(Value::Null);
                }
                serde_json::from_slice(&raw)
                    .map_err(|_| ShareError::Transient("Dexcom sent something that was not JSON.".into()))
            }
            Err(ureq::Error::Status(status, resp)) => {
                let raw = read_capped(resp, AUTH_MAX_BYTES).unwrap_or_default();
                Err(dexcom_error(status, &raw))
            }
            Err(ureq::Error::Transport(t)) => {
                let mut why = t.to_string();
                why.retain(|c| !c.is_control());
                why.truncate(160);
                Err(ShareError::Transient(format!("Could not reach Dexcom: {why}")))
            }
        }
    }

    /// Account name (email, phone or legacy username) to account ID.
    pub fn authenticate(&self, account_name: &str, password: &str) -> Result<String, ShareError> {
        let body = serde_json::json!({"accountName": account_name, "password": password, "applicationId": APP_ID});
        let id = uuid_string(self.post("/General/AuthenticatePublisherAccount", Some(&body), AUTH_MAX_BYTES)?, "account ID")?;
        if id == NULL_UUID {
            return Err(ShareError::BadCredentials("Dexcom does not know that account name.".into()));
        }
        Ok(id)
    }

    /// Account ID to session ID.
    pub fn login(&self, account_id: &str, password: &str) -> Result<String, ShareError> {
        let body = serde_json::json!({"accountId": account_id, "password": password, "applicationId": APP_ID});
        let sid = uuid_string(self.post("/General/LoginPublisherAccountById", Some(&body), AUTH_MAX_BYTES)?, "session ID")?;
        if sid == NULL_UUID {
            return Err(ShareError::BadCredentials("Dexcom rejected the password.".into()));
        }
        Ok(sid)
    }

    /// Newest first, already validated.
    pub fn readings(&self, session_id: &str, minutes: u32, max_count: u32) -> Result<Vec<Reading>, ShareError> {
        if !is_uuid(session_id) {
            return Err(ShareError::SessionGone);
        }
        let path = format!(
            "/Publisher/ReadPublisherLatestGlucoseValues?sessionId={}&minutes={}&maxCount={}",
            session_id,
            minutes.clamp(1, 1440),
            max_count.clamp(1, 288)
        );
        let doc = self.post(&path, None, READ_MAX_BYTES)?;
        let items = match doc {
            Value::Null => return Ok(Vec::new()),
            Value::Array(a) => a,
            _ => return Err(ShareError::Transient("Dexcom did not return a list of readings.".into())),
        };
        let mut out: Vec<Reading> = items.iter().take(288).filter_map(normalize).collect();
        out.sort_by(|a, b| b.t.cmp(&a.t));
        Ok(out)
    }
}

fn read_capped(resp: ureq::Response, max: u64) -> Result<Vec<u8>, ShareError> {
    let mut raw = Vec::new();
    resp.into_reader()
        .take(max + 1)
        .read_to_end(&mut raw)
        .map_err(|e| ShareError::Transient(format!("Lost the connection to Dexcom: {}", e.kind())))?;
    if raw.len() as u64 > max {
        return Err(ShareError::Transient("Dexcom sent more data than expected.".into()));
    }
    Ok(raw)
}

fn dexcom_error(status: u16, raw: &[u8]) -> ShareError {
    let code = serde_json::from_slice::<Value>(raw)
        .ok()
        .and_then(|v| v.get("Code").and_then(|c| c.as_str()).map(|s| s.chars().take(64).collect::<String>()))
        .unwrap_or_default();
    match code.as_str() {
        "SessionIdNotFound" | "SessionNotValid" => ShareError::SessionGone,
        "SSO_AuthenticateAccountNotFound" => ShareError::BadCredentials("Dexcom does not know that account name.".into()),
        "SSO_AuthenticatePasswordInvalid" | "AccountPasswordInvalid" => {
            ShareError::BadCredentials("Dexcom rejected the password.".into())
        }
        "SSO_AuthenticateMaxAttemptsExceeed" => ShareError::Lockout,
        "" => ShareError::Transient(format!("Dexcom answered with HTTP {status}.")),
        other => ShareError::Dexcom(other.chars().filter(|c| c.is_ascii_alphanumeric() || *c == '_').collect()),
    }
}

pub fn is_uuid(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 36
        && b.iter().enumerate().all(|(i, c)| match i {
            8 | 13 | 18 | 23 => *c == b'-',
            _ => c.is_ascii_hexdigit(),
        })
}

fn uuid_string(v: Value, what: &str) -> Result<String, ShareError> {
    match v {
        Value::String(s) if is_uuid(&s) => Ok(s.to_ascii_lowercase()),
        _ => Err(ShareError::Transient(format!("Dexcom did not return a {what}."))),
    }
}

/// `Date(1760112345000)`, `/Date(1760112345000+0000)/` and friends to seconds.
pub fn parse_wt(s: &str) -> Option<i64> {
    let s = s.trim().trim_start_matches('/').trim_end_matches('/');
    let inner = s.strip_prefix("Date(")?.strip_suffix(')')?;
    let digits_end = inner
        .char_indices()
        .find(|&(i, c)| !(c.is_ascii_digit() || (i == 0 && c == '-')))
        .map(|(i, _)| i)
        .unwrap_or(inner.len());
    let (num, rest) = inner.split_at(digits_end);
    if !rest.is_empty() {
        let tz = rest.as_bytes();
        if tz.len() != 5 || !(tz[0] == b'+' || tz[0] == b'-') || !tz[1..].iter().all(u8::is_ascii_digit) {
            return None;
        }
    }
    if num.is_empty() || num.len() > 16 {
        return None;
    }
    let ms: i64 = num.parse().ok()?;
    let t = ms.div_euclid(1000);
    (1_400_000_000..=4_000_000_000).contains(&t).then_some(t)
}

pub fn normalize(item: &Value) -> Option<Reading> {
    let obj = item.as_object()?;
    let t = parse_wt(obj.get("WT")?.as_str()?)?;
    let v = match obj.get("Value")? {
        Value::Number(n) => n.as_f64()?,
        _ => return None,
    };
    if !v.is_finite() {
        return None;
    }
    let v = v as i64;
    if !(20..=600).contains(&v) {
        return None;
    }
    let trend = obj.get("Trend").map(Trend::from_value).unwrap_or_default();
    Some(Reading { t, v, trend })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wt_forms() {
        assert_eq!(parse_wt("Date(1760112345000)"), Some(1760112345));
        assert_eq!(parse_wt("/Date(1760112345000+0000)/"), Some(1760112345));
        assert_eq!(parse_wt("Date(1760112345000-0500)"), Some(1760112345));
        assert_eq!(parse_wt("Date(12)"), None);
        assert_eq!(parse_wt("Date(1760112345000x)"), None);
        assert_eq!(parse_wt("garbage"), None);
    }

    #[test]
    fn normalize_drops_bad_entries() {
        assert_eq!(
            normalize(&json!({"WT":"Date(1760112345000)","Value":142,"Trend":"Flat"})),
            Some(Reading { t: 1760112345, v: 142, trend: Trend::Flat })
        );
        assert_eq!(normalize(&json!({"WT":"Date(1760112345000)","Value":138,"Trend":3})).unwrap().trend, Trend::FortyFiveUp);
        assert_eq!(normalize(&json!({"WT":"Date(1760112345000)","Value":120,"Trend":"Sideways"})).unwrap().trend, Trend::None);
        assert_eq!(normalize(&json!({"WT":"Date(1760112345000)","Value":"142"})), None);
        assert_eq!(normalize(&json!({"WT":"Date(1760112345000)","Value":999})), None);
        assert_eq!(normalize(&json!([1, 2])), None);
    }

    #[test]
    fn error_codes() {
        let e = |code: &str| dexcom_error(500, format!("{{\"Code\":\"{code}\"}}").as_bytes());
        assert_eq!(e("SessionNotValid"), ShareError::SessionGone);
        assert_eq!(e("SessionIdNotFound"), ShareError::SessionGone);
        assert!(e("SSO_AuthenticatePasswordInvalid").needs_login());
        assert!(e("AccountPasswordInvalid").needs_login());
        assert_eq!(e("SSO_AuthenticateMaxAttemptsExceeed"), ShareError::Lockout);
        assert_eq!(e("SSO_InternalError"), ShareError::Dexcom("SSO_InternalError".into()));
        assert!(matches!(dexcom_error(502, b"<html>"), ShareError::Transient(_)));
    }

    #[test]
    fn uuids() {
        assert!(is_uuid("11111111-2222-3333-4444-555555555555"));
        assert!(!is_uuid("11111111-2222-3333-4444-55555555555"));
        assert!(!is_uuid("11111111x2222-3333-4444-555555555555"));
    }

    #[test]
    fn origins_are_fixed() {
        assert_eq!(Region::Us.base(), "https://share2.dexcom.com/ShareWebServices/Services");
        assert_eq!(Region::parse("Outside the US"), Some(Region::Ous));
    }
}
