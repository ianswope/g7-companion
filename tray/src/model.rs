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

pub fn format_age(seconds: i64) -> String {
    if seconds < 60 {
        return "now".into();
    }
    let m = seconds / 60;
    if m < 60 {
        return format!("{m}m");
    }
    let h = m / 60;
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
    let h = m / 60;
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
    fn ages() {
        assert_eq!(format_age(30), "now");
        assert_eq!(format_age(7200), "2h");
        assert_eq!(format_age_long(60), "1 minute ago");
    }
}
