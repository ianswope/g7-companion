//! Formatting and classification. Mirrors Model.js in the Omarchy plugin so
//! both clients describe a reading the same way.

use crate::share::Reading;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    #[default]
    Mgdl,
    Mmol,
}

impl Unit {
    pub fn label(self) -> &'static str {
        match self {
            Unit::Mgdl => "mg/dL",
            Unit::Mmol => "mmol/L",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Thresholds {
    pub urgent_low: i64,
    pub low: i64,
    pub high: i64,
}

impl Thresholds {
    /// Clamped and kept in order whatever the settings file says.
    pub fn sane(urgent_low: i64, low: i64, high: i64) -> Thresholds {
        let urgent_low = urgent_low.clamp(40, 100);
        let low = low.clamp(50, 150).max(urgent_low + 1);
        let high = high.clamp(120, 400).max(low + 10);
        Thresholds { urgent_low, low, high }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    UrgentLow,
    Low,
    InRange,
    High,
}

pub fn classify(mgdl: i64, t: &Thresholds) -> Band {
    if mgdl < t.urgent_low {
        Band::UrgentLow
    } else if mgdl < t.low {
        Band::Low
    } else if mgdl > t.high {
        Band::High
    } else {
        Band::InRange
    }
}

pub fn format_value(mgdl: i64, unit: Unit) -> String {
    match unit {
        Unit::Mgdl => mgdl.to_string(),
        Unit::Mmol => format!("{:.1}", (mgdl as f64 / 1.8).round() / 10.0),
    }
}

pub fn age_seconds(r: &Reading, now: i64) -> i64 {
    (now - r.t).max(0)
}

/// Hours during which a leftover number of minutes is still shown. Past this,
/// whole hours are enough; before it, "1h" would hide almost an hour.
const AGE_MINUTE_HOURS: i64 = 6;

pub fn format_age(seconds: i64) -> String {
    if seconds < 60 {
        return "now".into();
    }
    let m = seconds / 60;
    if m < 60 {
        return format!("{m}m");
    }
    let (h, rem) = (m / 60, m % 60);
    if h < AGE_MINUTE_HOURS && rem > 0 {
        return format!("{h}h {rem}m");
    }
    if h < 48 {
        return format!("{h}h");
    }
    format!("{}d", h / 24)
}

pub fn format_age_long(seconds: i64) -> String {
    if seconds < 60 {
        return "just now".into();
    }
    let m = seconds / 60;
    if m < 60 {
        return if m == 1 { "1 minute ago".into() } else { format!("{m} minutes ago") };
    }
    let (h, rem) = (m / 60, m % 60);
    if h < AGE_MINUTE_HOURS && rem > 0 {
        let hours = if h == 1 { "1 hour".to_string() } else { format!("{h} hours") };
        let minutes = if rem == 1 { "1 minute".to_string() } else { format!("{rem} minutes") };
        return format!("{hours} {minutes} ago");
    }
    if h < 48 {
        return if h == 1 { "1 hour ago".into() } else { format!("{h} hours ago") };
    }
    format!("{} days ago", h / 24)
}

pub fn is_stale(r: Option<&Reading>, now: i64, stale_minutes: i64) -> bool {
    match r {
        None => true,
        Some(r) => age_seconds(r, now) > stale_minutes * 60,
    }
}

/// Change since the previous reading, if the two are a normal step apart.
pub fn delta(readings: &[Reading]) -> Option<(i64, i64)> {
    let (a, b) = (readings.first()?, readings.get(1)?);
    let gap = a.t - b.t;
    if gap <= 0 || gap > 15 * 60 {
        return None;
    }
    Some((a.v - b.v, ((gap as f64 / 60.0).round() as i64).max(1)))
}

pub fn format_delta(d: (i64, i64), unit: Unit) -> String {
    let (mgdl, minutes) = d;
    let sign = match mgdl.signum() {
        1 => "+",
        -1 => "−",
        _ => "±",
    };
    let mag = match unit {
        Unit::Mgdl => mgdl.abs().to_string(),
        Unit::Mmol => format!("{:.1}", (mgdl.abs() as f64 / 1.8).round() / 10.0),
    };
    format!("{sign}{mag} {} in {minutes} min", unit.label())
}

/// How far back the chart and the Share fetch look. The G7 posts a reading
/// about every five minutes, so three hours is a few dozen points.
pub const HISTORY_MINUTES: i64 = 180;

/// The chart's vertical range, in mg/dL. Readings outside it are clamped so
/// one spike cannot flatten the rest of the line.
pub const SPARK_Y_MIN: f64 = 40.0;
pub const SPARK_Y_MAX: f64 = 400.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SparkPoint {
    /// 0 at the start of the window, 1 at now.
    pub x: f64,
    /// 0 at the bottom of the plotted range, 1 at the top.
    pub y: f64,
    pub v: i64,
}

/// Points for the chart, oldest first. Readings outside the window are dropped.
/// Same rule as `sparkPoints` in Model.js.
pub fn spark_points(readings: &[Reading], now: i64, window_minutes: i64, y_min: f64, y_max: f64) -> Vec<SparkPoint> {
    let span = (window_minutes.max(0) as f64) * 60.0;
    if span <= 0.0 || y_max <= y_min {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in readings.iter().rev() {
        let dx = (now - r.t) as f64;
        if dx < 0.0 || dx > span {
            continue;
        }
        let v = (r.v as f64).clamp(y_min, y_max);
        out.push(SparkPoint { x: 1.0 - dx / span, y: (v - y_min) / (y_max - y_min), v: r.v });
    }
    out
}

/// One dot on the menu chart, already coloured by the thresholds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartDot {
    pub x: f64,
    pub y: f64,
    pub band: Band,
}

/// What the dropdown draws: the reading, and the last three hours.
#[derive(Clone, Debug, PartialEq)]
pub struct ChartContent {
    pub headline: String,
    /// Set for a fresh reading outside the target range. In range and stale
    /// both draw in the menu's own text colour.
    pub headline_band: Option<Band>,
    pub stale: bool,
    pub trend: String,
    pub detail: String,
    pub age: String,
    pub age_warn: bool,
    pub status: String,
    pub status_warn: bool,
    pub points: Vec<ChartDot>,
    /// Threshold lines, in the same 0-at-bottom space as the dots.
    pub low_y: f64,
    pub high_y: f64,
    pub window_minutes: i64,
}

impl ChartContent {
    pub fn show_chart(&self) -> bool {
        self.points.len() > 1
    }
}

fn y_unit(mgdl: i64) -> f64 {
    let v = (mgdl as f64).clamp(SPARK_Y_MIN, SPARK_Y_MAX);
    (v - SPARK_Y_MIN) / (SPARK_Y_MAX - SPARK_Y_MIN)
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// The dropdown's contents. Mirrors the Omarchy panel: value, trend, change,
/// age, and one dot per reading in the window.
pub fn chart_content(
    readings: &[Reading],
    now: i64,
    unit: Unit,
    thresholds: &Thresholds,
    stale_minutes: i64,
    signed_in: bool,
    needs_login: bool,
    account: &str,
    message: &str,
) -> ChartContent {
    let latest = readings.first();
    let stale = latest.is_some_and(|r| is_stale(Some(r), now, stale_minutes));
    let points = spark_points(readings, now, HISTORY_MINUTES, SPARK_Y_MIN, SPARK_Y_MAX)
        .into_iter()
        .map(|p| ChartDot { x: p.x, y: p.y, band: classify(p.v, thresholds) })
        .collect();
    let headline_band = match (latest, stale) {
        (Some(r), false) => match classify(r.v, thresholds) {
            Band::InRange => None,
            band => Some(band),
        },
        _ => None,
    };
    let headline = match latest {
        Some(r) => format_value(r.v, unit),
        None if signed_in => "No reading yet".into(),
        None => "Not signed in".into(),
    };
    let trend = match latest {
        Some(r) => {
            let arrow = r.trend.arrow();
            if arrow.is_empty() { r.trend.words().into() } else { format!("{arrow}  {}", r.trend.words()) }
        }
        None => String::new(),
    };
    let detail = match latest {
        Some(_) => match delta(readings) {
            Some(d) => format!("{} · {}", unit.label(), format_delta(d, unit)),
            None => unit.label().into(),
        },
        None if !signed_in => "Use the Dexcom account of the person wearing the G7.".into(),
        None => String::new(),
    };
    let age = match latest {
        Some(r) => {
            let mut text = format_age_long(age_seconds(r, now));
            if stale {
                text.push_str(" · not updating");
            }
            text
        }
        None => String::new(),
    };
    let (status, status_warn) = if needs_login {
        let text = if message.is_empty() { "Sign in again.".into() } else { format!("{message} Sign in again.") };
        (shorten(&text, 160), true)
    } else if signed_in && !message.is_empty() {
        let mut text = message.to_string();
        if !readings.is_empty() {
            text.push_str(" Showing the last reading received.");
        }
        (shorten(&text, 160), false)
    } else if signed_in && !account.is_empty() {
        (format!("Signed in as {account}"), false)
    } else {
        (String::new(), false)
    };
    ChartContent {
        headline,
        headline_band,
        stale,
        trend,
        detail,
        age,
        age_warn: stale,
        status,
        status_warn,
        points,
        low_y: y_unit(thresholds.low),
        high_y: y_unit(thresholds.high),
        window_minutes: HISTORY_MINUTES,
    }
}

/// What the menu bar shows. The age is appended whenever the reading is
/// stale, so a number that stopped updating never passes for a live one.
pub fn bar_label(r: Option<&Reading>, now: i64, unit: Unit, stale_minutes: i64, signed_in: bool) -> String {
    let Some(r) = r else {
        return if signed_in { "G7 --".into() } else { "G7".into() };
    };
    let mut s = format_value(r.v, unit);
    let arrow = r.trend.arrow();
    if !arrow.is_empty() {
        s.push(' ');
        s.push_str(arrow);
    }
    if is_stale(Some(r), now, stale_minutes) {
        s.push(' ');
        s.push_str(&format_age(age_seconds(r, now)));
    }
    s
}

pub fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::share::Trend;

    const NOW: i64 = 1_760_112_345;

    fn r(age: i64, v: i64, trend: Trend) -> Reading {
        Reading { t: NOW - age, v, trend }
    }

    #[test]
    fn labels() {
        assert_eq!(bar_label(Some(&r(120, 142, Trend::Flat)), NOW, Unit::Mgdl, 10, true), "142 →");
        assert_eq!(bar_label(Some(&r(660, 142, Trend::Flat)), NOW, Unit::Mgdl, 10, true), "142 → 11m");
        assert_eq!(bar_label(Some(&r(83 * 60, 142, Trend::Flat)), NOW, Unit::Mgdl, 10, true), "142 → 1h 23m");
        assert_eq!(bar_label(Some(&r(60, 54, Trend::DoubleDown)), NOW, Unit::Mmol, 10, true), "3.0 ⇊");
        assert_eq!(bar_label(None, NOW, Unit::Mgdl, 10, true), "G7 --");
        assert_eq!(bar_label(None, NOW, Unit::Mgdl, 10, false), "G7");
    }

    #[test]
    fn bands_and_thresholds() {
        let t = Thresholds::sane(55, 70, 180);
        assert_eq!(classify(54, &t), Band::UrgentLow);
        assert_eq!(classify(69, &t), Band::Low);
        assert_eq!(classify(180, &t), Band::InRange);
        assert_eq!(classify(181, &t), Band::High);
        let odd = Thresholds::sane(100, 50, 120);
        assert!(odd.urgent_low < odd.low && odd.low < odd.high);
    }

    #[test]
    fn deltas() {
        let rs = [r(0, 142, Trend::Flat), r(300, 138, Trend::Flat)];
        assert_eq!(delta(&rs), Some((4, 5)));
        assert_eq!(format_delta((-6, 5), Unit::Mgdl), "−6 mg/dL in 5 min");
        assert_eq!(delta(&[r(0, 142, Trend::Flat), r(3600, 100, Trend::Flat)]), None);
    }

    #[test]
    fn sparkline_stays_in_the_window_and_the_unit_square() {
        let readings = [r(0, 400, Trend::Flat), r(5400, 100, Trend::Flat), r(20000, 100, Trend::Flat)];
        let pts = spark_points(&readings, NOW, 180, SPARK_Y_MIN, SPARK_Y_MAX);
        assert_eq!(pts.len(), 2);
        assert!((pts[1].x - 1.0).abs() < 1e-9);
        assert!((pts[1].y - 1.0).abs() < 1e-9);
        assert!(pts.iter().all(|p| (0.0..=1.0).contains(&p.x) && (0.0..=1.0).contains(&p.y)));
        let below = spark_points(&[r(0, 300, Trend::Flat)], NOW, 180, SPARK_Y_MIN, SPARK_Y_MAX);
        assert!(below[0].y < 1.0);
        assert!(spark_points(&[r(-60, 120, Trend::Flat)], NOW, 180, SPARK_Y_MIN, SPARK_Y_MAX).is_empty());
    }

    #[test]
    fn chart_matches_the_panel() {
        let thresholds = Thresholds::sane(55, 70, 180);
        let readings = [r(60, 142, Trend::FortyFiveUp), r(360, 136, Trend::Flat)];
        let chart = chart_content(&readings, NOW, Unit::Mgdl, &thresholds, 10, true, false, "s***@example.com", "");
        assert_eq!(chart.headline, "142");
        assert_eq!(chart.headline_band, None);
        assert_eq!(chart.trend, "↗  rising slowly");
        assert_eq!(chart.detail, "mg/dL · +6 mg/dL in 5 min");
        assert_eq!(chart.age, "1 minute ago");
        assert!(chart.show_chart());
        assert_eq!(chart.status, "Signed in as s***@example.com");
        assert!(!chart.age_warn);

        let low = chart_content(&[r(60, 54, Trend::DoubleDown)], NOW, Unit::Mgdl, &thresholds, 10, true, false, "", "");
        assert_eq!(low.headline_band, Some(Band::UrgentLow));
        assert!(!low.show_chart());

        let stale = chart_content(&[r(20 * 60, 142, Trend::Flat), r(25 * 60, 140, Trend::Flat)], NOW, Unit::Mgdl, &thresholds, 10, true, false, "a", "");
        assert!(stale.stale && stale.age_warn && stale.headline_band.is_none());
        assert!(stale.age.ends_with("not updating"));

        let out = chart_content(&[], NOW, Unit::Mgdl, &thresholds, 10, false, false, "", "");
        assert_eq!(out.headline, "Not signed in");
        assert!(out.detail.contains("wearing the G7"));
    }

    #[test]
    fn ages() {
        assert_eq!(format_age(30), "now");
        assert_eq!(format_age(119 * 60), "1h 59m");
        assert_eq!(format_age(7200), "2h");
        assert_eq!(format_age(6 * 3600 + 20 * 60), "6h");
        assert_eq!(format_age_long(60), "1 minute ago");
        assert_eq!(format_age_long(61 * 60), "1 hour 1 minute ago");
        assert_eq!(format_age_long(119 * 60), "1 hour 59 minutes ago");
        assert_eq!(format_age_long(7200), "2 hours ago");
    }
}
