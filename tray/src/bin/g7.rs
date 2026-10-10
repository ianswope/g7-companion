//! g7: the command-line side of G7 Companion.
//!
//!     g7 login [--account-id] [--region us|ous|jp]
//!     g7 read [--json]
//!     g7 status
//!     g7 logout

use g7::model::{self, now};
use g7::poller::{self, Credentials};
use g7::share::{Client, Region};
use g7::store::Store;
use std::io::Write;
use std::process::ExitCode;

const USAGE: &str = "usage: g7 login [--account-id] [--region us|ous|jp] | g7 read [--json] | g7 status | g7 logout";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let Some(cmd) = args.first().map(String::as_str) else {
        eprintln!("{USAGE}");
        return ExitCode::from(2);
    };
    let store = match Store::open() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("g7: {e}");
            return ExitCode::FAILURE;
        }
    };
    match cmd {
        "login" => login(&store, &args[1..]),
        "read" => read(&store, args.iter().any(|a| a == "--json")),
        "status" => status(&store),
        "logout" => match store.clear() {
            Ok(()) => {
                println!("Signed out. Credentials and cached readings removed.");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("g7: {e}");
                ExitCode::FAILURE
            }
        },
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            ExitCode::SUCCESS
        }
        _ => {
            eprintln!("{USAGE}");
            ExitCode::from(2)
        }
    }
}

fn prompt(label: &str) -> Option<String> {
    print!("{label}");
    std::io::stdout().flush().ok()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line).ok()?;
    Some(line.trim().to_string())
}

fn login(store: &Store, args: &[String]) -> ExitCode {
    let by_id = args.iter().any(|a| a == "--account-id");
    let region = match args.iter().position(|a| a == "--region").and_then(|i| args.get(i + 1)) {
        Some(r) => match Region::parse(r) {
            Some(r) => r,
            None => {
                eprintln!("g7: region must be us, ous or jp");
                return ExitCode::from(2);
            }
        },
        None => Region::Us,
    };
    eprintln!("Use the Dexcom account the G7 app on the wearer's phone is signed into:");
    eprintln!("the person sharing, not a follower. The password goes to the system credential store.");
    let account = if by_id {
        prompt("Dexcom account ID: ")
    } else {
        prompt("Dexcom username (email, or phone like +15551234567): ")
    };
    let Some(account) = account.filter(|a| !a.is_empty()) else {
        eprintln!("g7: cancelled");
        return ExitCode::FAILURE;
    };
    let Ok(password) = rpassword::prompt_password("Dexcom password: ") else {
        eprintln!("g7: cancelled");
        return ExitCode::FAILURE;
    };
    let creds = Credentials {
        region,
        account_name: (!by_id).then(|| account.clone()),
        account_id: by_id.then_some(account),
        password,
    };
    match poller::sign_in(store, &Client::new, creds) {
        Ok(snap) => {
            println!("Signed in as {}.", snap.account);
            match snap.latest() {
                Some(r) => println!(
                    "Latest: {} mg/dL {} ({}).",
                    r.v,
                    r.trend.arrow(),
                    model::format_age_long(model::age_seconds(r, now()))
                ),
                None => println!("No reading in the last 24 hours. Check that Share is on in the G7 app."),
            }
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("g7: {e}");
            ExitCode::FAILURE
        }
    }
}

fn read(store: &Store, json: bool) -> ExitCode {
    let snap = poller::poll(store, &Client::new, 180);
    let settings = store.load_settings();
    if json {
        let doc = serde_json::json!({
            "ok": snap.live,
            "signed_in": snap.signed_in,
            "needs_login": snap.needs_login,
            "message": snap.message,
            "fetched_at": snap.fetched_at,
            "readings": snap.readings,
        });
        println!("{doc}");
    } else if let Some(r) = snap.latest() {
        let age = model::age_seconds(r, now());
        println!(
            "{} {} {}, {}{}",
            model::format_value(r.v, settings.unit),
            settings.unit.label(),
            r.trend.arrow(),
            model::format_age_long(age),
            if snap.live { String::new() } else { format!(" ({})", snap.message) }
        );
    } else {
        println!("{}", if snap.message.is_empty() { "No reading." } else { &snap.message });
    }
    if snap.live || (!snap.needs_login && snap.signed_in && !snap.readings.is_empty()) {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn status(store: &Store) -> ExitCode {
    let s = store.load_settings();
    let st = store.load_state();
    println!("signed in:   {}", if s.has_account() { s.account_label() } else { "no".into() });
    println!("region:      {}", s.region.label());
    println!("units:       {}", s.unit.label());
    println!("needs login: {}", st.auth_failed);
    if st.backoff_until > now() {
        println!("waiting:     {} s ({})", st.backoff_until - now(), st.backoff_message);
    }
    println!("settings:    {}", store.settings_path().display());
    ExitCode::SUCCESS
}
