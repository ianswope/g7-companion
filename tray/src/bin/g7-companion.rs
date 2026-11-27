//! G7 Companion tray app: the latest Dexcom G7 reading in the macOS menu bar
//! or the Windows notification area.
#![cfg_attr(windows, windows_subsystem = "windows")]

use g7::icon;
use g7::model::{self, now};
use g7::platform;
use g7::poller::{self, Snapshot};
use g7::share::Client;
use g7::store::Store;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};
use tao::event::{Event, StartCause};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tray_icon::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

#[cfg(target_os = "macos")]
#[path = "macos_chart/mod.rs"]
mod macos_chart;

enum UserEvent {
    Snapshot(Snapshot),
    Menu(MenuEvent),
    /// None when the person cancelled the dialog.
    SignInDone(Option<Result<Snapshot, String>>),
    /// True when the person confirmed sign-out.
    SignOutAnswer(bool),
}

enum Cmd {
    Refresh,
    /// Poll quickly until this unix time while a sign-in dialog may be open.
    WatchSignIn(i64),
}

struct Items {
    value: MenuItem,
    detail: MenuItem,
    change: MenuItem,
    status: MenuItem,
    refresh: MenuItem,
    sign_in: MenuItem,
    sign_in_id: MenuItem,
    sign_out: MenuItem,
    mmol: CheckMenuItem,
    autostart: CheckMenuItem,
    settings: MenuItem,
    quit: MenuItem,
}

fn main() {
    let store = match Store::open() {
        Ok(s) => s,
        Err(e) => {
            platform::alert(&format!("G7 Companion cannot open its settings folder: {e}"));
            return;
        }
    };

    #[allow(unused_mut)]
    let mut event_loop = EventLoopBuilder::<UserEvent>::with_user_event().build();
    #[cfg(target_os = "macos")]
    {
        use tao::platform::macos::{ActivationPolicy, EventLoopExtMacOS};
        // A menu bar item, not an app: no Dock icon, no app menu.
        event_loop.set_activation_policy(ActivationPolicy::Accessory);
    }

    let proxy = event_loop.create_proxy();
    {
        let proxy = proxy.clone();
        MenuEvent::set_event_handler(Some(move |e| {
            let _ = proxy.send_event(UserEvent::Menu(e));
        }));
    }

    let (cmd_tx, cmd_rx) = mpsc::channel::<Cmd>();
    {
        let proxy = proxy.clone();
        thread::spawn(move || worker(cmd_rx, proxy));
    }

    let items = Items {
        value: MenuItem::new("G7 Companion", false, None),
        detail: MenuItem::new("", false, None),
        change: MenuItem::new("", false, None),
        status: MenuItem::new("", false, None),
        refresh: MenuItem::new("Refresh now", true, None),
        sign_in: MenuItem::new("Sign in…", true, None),
        sign_in_id: MenuItem::new("Sign in with account ID…", true, None),
        sign_out: MenuItem::new("Sign out", true, None),
        mmol: CheckMenuItem::new("Show mmol/L", true, store.load_settings().unit == model::Unit::Mmol, None),
        autostart: CheckMenuItem::new("Start at login", true, platform::autostart_enabled(), None),
        settings: MenuItem::new("Edit thresholds…", true, None),
        quit: MenuItem::new("Quit G7 Companion", true, None),
    };
    let menu = Menu::new();
    // macOS draws the reading and the chart itself, as the first row, once the
    // menu exists. The text rows would only repeat it.
    #[cfg(target_os = "macos")]
    let _ = menu.append(&PredefinedMenuItem::separator());
    #[cfg(not(target_os = "macos"))]
    let _ = menu.append_items(&[
        &items.value,
        &items.detail,
        &items.change,
        &items.status,
        &PredefinedMenuItem::separator(),
    ]);
    let _ = menu.append_items(&[
        &items.refresh,
        &items.sign_in,
        &items.sign_in_id,
        &items.sign_out,
        &PredefinedMenuItem::separator(),
        &items.mmol,
        &items.autostart,
        &items.settings,
        &PredefinedMenuItem::separator(),
        &MenuItem::new("Not a medical device. Alerts stay with the Dexcom app.", false, None),
        &items.quit,
    ]);

    let mut tray: Option<TrayIcon> = None;
    #[cfg(target_os = "macos")]
    let mut chart: Option<macos_chart::Slot> = None;
    let mut menu = Some(menu);
    let mut last = Snapshot::default();
    let mut signing_in = false;
    let mut confirming_sign_out = false;

    event_loop.run(move |event, _, control_flow| {
        // Wake every 15 seconds so ages and staleness move between polls.
        *control_flow = ControlFlow::WaitUntil(Instant::now() + Duration::from_secs(15));
        match event {
            Event::NewEvents(StartCause::Init) => {
                // macOS needs the status item created once the loop runs.
                #[cfg(target_os = "macos")]
                {
                    chart = Some(macos_chart::attach(menu.as_ref().expect("menu")));
                }
                let built = TrayIconBuilder::new()
                    .with_menu(Box::new(menu.take().expect("menu")))
                    .with_tooltip("G7 Companion")
                    .with_title("G7")
                    .with_icon(make_icon(&store, &last).0)
                    .build();
                match built {
                    Ok(t) => {
                        #[cfg(target_os = "macos")]
                        render(&t, &items, &store, &last, chart.as_ref());
                        #[cfg(not(target_os = "macos"))]
                        render(&t, &items, &store, &last);
                        tray = Some(t);
                    }
                    Err(e) => {
                        platform::alert(&format!("G7 Companion could not add its icon: {e}"));
                        *control_flow = ControlFlow::Exit;
                    }
                }
            }
            Event::NewEvents(StartCause::ResumeTimeReached { .. }) => {
                if let Some(t) = &tray {
                    #[cfg(target_os = "macos")]
                    render(t, &items, &store, &last, chart.as_ref());
                    #[cfg(not(target_os = "macos"))]
                    render(t, &items, &store, &last);
                }
            }
            Event::UserEvent(UserEvent::Snapshot(s)) => {
                last = s;
                if let Some(t) = &tray {
                    #[cfg(target_os = "macos")]
                    render(t, &items, &store, &last, chart.as_ref());
                    #[cfg(not(target_os = "macos"))]
                    render(t, &items, &store, &last);
                }
            }
            Event::UserEvent(UserEvent::SignOutAnswer(yes)) => {
                confirming_sign_out = false;
                if !yes {
                    return;
                }
                let _ = store.clear();
                last = Snapshot { message: "Not signed in.".into(), ..Default::default() };
                if let Some(t) = &tray {
                    #[cfg(target_os = "macos")]
                    render(t, &items, &store, &last, chart.as_ref());
                    #[cfg(not(target_os = "macos"))]
                    render(t, &items, &store, &last);
                }
            }
            Event::UserEvent(UserEvent::SignInDone(result)) => {
                signing_in = false;
                let Some(result) = result else { return };
                match result {
                    Ok(s) => {
                        last = s;
                        if let Some(t) = &tray {
                            #[cfg(target_os = "macos")]
                            render(t, &items, &store, &last, chart.as_ref());
                            #[cfg(not(target_os = "macos"))]
                            render(t, &items, &store, &last);
                        }
                        let _ = cmd_tx.send(Cmd::Refresh);
                    }
                    Err(e) => {
                        thread::spawn(move || platform::alert(&format!("Sign-in failed. {e}")));
                    }
                }
            }
            Event::UserEvent(UserEvent::Menu(e)) => {
                let id = e.id();
                if id == items.quit.id() {
                    tray.take();
                    *control_flow = ControlFlow::Exit;
                } else if id == items.refresh.id() {
                    let _ = cmd_tx.send(Cmd::Refresh);
                } else if (id == items.sign_in.id() || id == items.sign_in_id.id()) && !signing_in {
                    signing_in = true;
                    let _ = cmd_tx.send(Cmd::WatchSignIn(now() + poller::SIGN_IN_WATCH_SECONDS));
                    let by_id = id == items.sign_in_id.id();
                    let proxy = proxy.clone();
                    thread::spawn(move || {
                        let result = platform::ask_credentials(by_id).map(|creds| match Store::open() {
                            Ok(store) => poller::sign_in(&store, &Client::new, creds),
                            Err(e) => Err(e.to_string()),
                        });
                        let _ = proxy.send_event(UserEvent::SignInDone(result));
                    });
                } else if id == items.sign_out.id() && last.signed_in && !confirming_sign_out {
                    confirming_sign_out = true;
                    let proxy = proxy.clone();
                    thread::spawn(move || {
                        let yes = platform::confirm(
                            "Sign out and delete the stored password and cached readings? Signing in again means typing that password again.",
                            "Sign out",
                        );
                        let _ = proxy.send_event(UserEvent::SignOutAnswer(yes));
                    });
                } else if id == items.mmol.id() {
                    let mut s = store.load_settings();
                    s.unit = if items.mmol.is_checked() { model::Unit::Mmol } else { model::Unit::Mgdl };
                    let _ = store.save_settings(&s);
                    if let Some(t) = &tray {
                        #[cfg(target_os = "macos")]
                        render(t, &items, &store, &last, chart.as_ref());
                        #[cfg(not(target_os = "macos"))]
                        render(t, &items, &store, &last);
                    }
                } else if id == items.autostart.id() {
                    let want = items.autostart.is_checked();
                    if let Err(e) = platform::set_autostart(want) {
                        items.autostart.set_checked(!want);
                        thread::spawn(move || platform::alert(&format!("Could not change start at login: {e}")));
                    }
                } else if id == items.settings.id() {
                    let s = store.load_settings();
                    let _ = store.save_settings(&s);
                    platform::open_in_editor(&store.settings_path());
                }
            }
            _ => {}
        }
    });
}

fn worker(rx: mpsc::Receiver<Cmd>, proxy: tao::event_loop::EventLoopProxy<UserEvent>) {
    let store = match Store::open() {
        Ok(s) => s,
        Err(_) => return,
    };
    let mut sign_in_until = 0i64;
    loop {
        let snap = poller::poll(&store, &Client::new, model::HISTORY_MINUTES as u32);
        let wait = poller::next_poll_secs(
            store.load_settings().refresh_sec(),
            sign_in_until,
            snap.backoff_until,
            now(),
        );
        if proxy.send_event(UserEvent::Snapshot(snap)).is_err() {
            return;
        }
        let cmd = match rx.recv_timeout(Duration::from_secs(wait)) {
            Ok(cmd) => Some(cmd),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        // Drain extra clicks so a burst becomes one poll. A sign-in watch
        // that arrives in the burst still has to be remembered.
        let mut pending = cmd.into_iter().chain(std::iter::from_fn(|| rx.try_recv().ok()));
        for cmd in pending.by_ref() {
            if let Cmd::WatchSignIn(until) = cmd {
                sign_in_until = until;
            }
        }
    }
}

fn make_icon(store: &Store, snap: &Snapshot) -> (Icon, String) {
    let settings = store.load_settings();
    let t = now();
    let latest = snap.latest();
    let stale = model::is_stale(latest, t, settings.stale_minutes());
    let band = latest.map(|r| model::classify(r.v, &settings.thresholds()));
    let color = icon::band_color(band, stale);
    let label = model::bar_label(latest, t, settings.unit, settings.stale_minutes(), snap.signed_in && !snap.needs_login);
    #[cfg(target_os = "macos")]
    let rgba = icon::dot_icon(color);
    #[cfg(not(target_os = "macos"))]
    let rgba = match latest {
        Some(r) => icon::reading_icon(&model::format_value(r.v, settings.unit), r.trend, color),
        None => icon::reading_icon("--", g7::share::Trend::None, color),
    };
    let icon = Icon::from_rgba(rgba, icon::SIZE, icon::SIZE).expect("icon size is fixed");
    (icon, label)
}

fn render(
    tray: &TrayIcon,
    items: &Items,
    store: &Store,
    snap: &Snapshot,
    #[cfg(target_os = "macos")] chart: Option<&macos_chart::Slot>,
) {
    let settings = store.load_settings();
    let t = now();
    let (icon, label) = make_icon(store, snap);
    let _ = tray.set_icon(Some(icon));
    tray.set_title(Some(&label));

    let latest = snap.latest();
    let stale = model::is_stale(latest, t, settings.stale_minutes());
    let tooltip = match latest {
        Some(r) => format!(
            "{} {} {}, {}{}",
            model::format_value(r.v, settings.unit),
            settings.unit.label(),
            r.trend.words(),
            model::format_age_long(model::age_seconds(r, t)),
            if stale { " (not updating)" } else { "" }
        ),
        None if !snap.signed_in => "G7 Companion: not signed in".into(),
        None => "G7 Companion: no reading yet".into(),
    };
    let _ = tray.set_tooltip(Some(&tooltip));

    match latest {
        Some(r) => {
            items.value.set_text(format!(
                "{} {}  {}  {}",
                model::format_value(r.v, settings.unit),
                settings.unit.label(),
                r.trend.arrow(),
                r.trend.words()
            ));
            items.detail.set_text(format!(
                "{}{}",
                model::format_age_long(model::age_seconds(r, t)),
                if stale { ", not updating" } else { "" }
            ));
        }
        None => {
            items.value.set_text(if snap.signed_in { "No reading yet" } else { "Not signed in" });
            items.detail.set_text("Use the Dexcom account of the person wearing the G7.");
        }
    }
    items.change.set_text(match model::delta(&snap.readings) {
        Some(d) => model::format_delta(d, settings.unit),
        None => String::new(),
    });
    let status = if snap.needs_login {
        format!("{} Sign in again.", snap.message)
    } else if !snap.message.is_empty() && snap.signed_in {
        snap.message.clone()
    } else if snap.signed_in {
        format!("Signed in as {}", snap.account)
    } else {
        String::new()
    };
    items.status.set_text(status.chars().take(90).collect::<String>());
    items.refresh.set_enabled(snap.signed_in && !snap.needs_login);
    items.sign_out.set_enabled(snap.signed_in);
    items.mmol.set_checked(settings.unit == model::Unit::Mmol);
    #[cfg(target_os = "macos")]
    if let Some(chart) = chart {
        chart.sync(store, snap);
    }
}
