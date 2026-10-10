.pragma library

// Pure functions for the G7 Companion widget. No QML types, no I/O, so the
// same file runs under node in tests/model.test.mjs.

var TRENDS = {
  "None":           { arrow: "",   words: "no trend" },
  "DoubleUp":       { arrow: "⇈",  words: "rising fast" },
  "SingleUp":       { arrow: "↑",  words: "rising" },
  "FortyFiveUp":    { arrow: "↗",  words: "rising slowly" },
  "Flat":           { arrow: "→",  words: "steady" },
  "FortyFiveDown":  { arrow: "↘",  words: "falling slowly" },
  "SingleDown":     { arrow: "↓",  words: "falling" },
  "DoubleDown":     { arrow: "⇊",  words: "falling fast" },
  "NotComputable":  { arrow: "?",  words: "trend not computable" },
  "RateOutOfRange": { arrow: "?",  words: "rate out of range" }
}

var MAX_READINGS = 288
var MAX_MESSAGE = 200

function trendArrow(trend) {
  var t = Object.prototype.hasOwnProperty.call(TRENDS, trend) ? TRENDS[trend] : null
  return t ? t.arrow : ""
}

function trendWords(trend) {
  var t = Object.prototype.hasOwnProperty.call(TRENDS, trend) ? TRENDS[trend] : null
  return t ? t.words : "no trend"
}

// Strip markup characters, control characters and bidi overrides, then cap.
// Everything shown is rendered as plain text anyway; this is for the host
// components (tooltips, hero titles) that cannot be pinned to plain text.
function sanitize(text, limit) {
  var s = String(text === undefined || text === null ? "" : text)
  s = s.replace(/[<>&]/g, "")
  s = s.replace(/[\u0000-\u001f\u007f-\u009f‎‏‪-‮⁦-⁩]/g, " ")
  s = s.replace(/\s+/g, " ").trim()
  var cap = limit || MAX_MESSAGE
  return s.length > cap ? s.slice(0, cap - 1) + "…" : s
}

function isInt(n) {
  return typeof n === "number" && isFinite(n) && Math.floor(n) === n
}

// The helper's output is input like any other. Only the shape this widget
// draws survives; anything else is dropped rather than repaired.
function parseHelperOutput(text) {
  var doc
  try {
    doc = JSON.parse(String(text || ""))
  } catch (e) {
    return null
  }
  if (!doc || typeof doc !== "object" || Array.isArray(doc)) return null
  var readings = []
  var raw = Array.isArray(doc.readings) ? doc.readings.slice(0, MAX_READINGS) : []
  for (var i = 0; i < raw.length; i++) {
    var r = raw[i]
    if (!r || typeof r !== "object") continue
    if (!isInt(r.t) || !isInt(r.v)) continue
    if (r.v < 20 || r.v > 600) continue
    if (r.t < 1400000000 || r.t > 4000000000) continue
    var trend = Object.prototype.hasOwnProperty.call(TRENDS, r.trend) ? r.trend : "None"
    readings.push({ t: r.t, v: r.v, trend: trend })
  }
  readings.sort(function(a, b) { return b.t - a.t })
  return {
    ok: doc.ok === true,
    error: sanitize(doc.error, 64),
    message: sanitize(doc.message, MAX_MESSAGE),
    needsLogin: doc.needs_login === true,
    retryAfter: isInt(doc.retry_after) ? doc.retry_after : 0,
    fetchedAt: isInt(doc.fetched_at) ? doc.fetched_at : 0,
    source: doc.source === "live" ? "live" : (doc.source === "cache" ? "cache" : "none"),
    account: sanitize(doc.account, 80),
    readings: readings
  }
}

function toUnit(mgdl, unit) {
  return unit === "mmol" ? mgdl / 18.0 : mgdl
}

function formatValue(mgdl, unit) {
  if (!isInt(mgdl)) return "--"
  if (unit === "mmol") return (Math.round(mgdl / 1.8) / 10).toFixed(1)
  return String(mgdl)
}

function unitLabel(unit) {
  return unit === "mmol" ? "mmol/L" : "mg/dL"
}

// Change from the reading about five minutes earlier. Readings further apart
// than 15 minutes have a gap between them and no honest delta.
function delta(readings) {
  if (!readings || readings.length < 2) return null
  var a = readings[0], b = readings[1]
  var gap = a.t - b.t
  if (gap <= 0 || gap > 15 * 60) return null
  return { mgdl: a.v - b.v, minutes: Math.max(1, Math.round(gap / 60)) }
}

function formatDelta(d, unit) {
  if (!d) return ""
  var sign = d.mgdl > 0 ? "+" : (d.mgdl < 0 ? "−" : "±")
  var mag = Math.abs(d.mgdl)
  var text = unit === "mmol" ? (Math.round(mag / 1.8) / 10).toFixed(1) : String(mag)
  return sign + text + " " + unitLabel(unit) + " in " + d.minutes + " min"
}

// "urgentLow", "low", "inRange", "high" or "" with no reading.
function classify(mgdl, thresholds) {
  if (!isInt(mgdl)) return ""
  if (mgdl < thresholds.urgentLow) return "urgentLow"
  if (mgdl < thresholds.low) return "low"
  if (mgdl > thresholds.high) return "high"
  return "inRange"
}

function ageSeconds(reading, nowSeconds) {
  if (!reading) return -1
  return Math.max(0, nowSeconds - reading.t)
}

function formatAge(seconds) {
  if (seconds < 0) return ""
  if (seconds < 60) return "now"
  var m = Math.floor(seconds / 60)
  if (m < 60) return m + "m"
  var h = Math.floor(m / 60)
  if (h < 48) return h + "h"
  return Math.floor(h / 24) + "d"
}

function formatAgeLong(seconds) {
  if (seconds < 0) return ""
  if (seconds < 60) return "just now"
  var m = Math.floor(seconds / 60)
  if (m < 60) return m === 1 ? "1 minute ago" : m + " minutes ago"
  var h = Math.floor(m / 60)
  if (h < 48) return h === 1 ? "1 hour ago" : h + " hours ago"
  var d = Math.floor(h / 24)
  return d + " days ago"
}

function isStale(seconds, staleMinutes) {
  return seconds < 0 || seconds > staleMinutes * 60
}

// What the bar shows. The age is part of the label whenever the reading is
// not fresh, so a number that stopped updating can never pass for a live one.
function barLabel(reading, nowSeconds, opts) {
  if (!reading) return opts.signedIn ? "G7 --" : "G7"
  var age = ageSeconds(reading, nowSeconds)
  var parts = [formatValue(reading.v, opts.unit)]
  if (opts.showTrend !== false) {
    var arrow = trendArrow(reading.trend)
    if (arrow !== "") parts.push(arrow)
  }
  if (isStale(age, opts.staleMinutes) || opts.alwaysShowAge === true) parts.push(formatAge(age))
  return parts.join(" ")
}

function headline(state) {
  if (!state.signedIn) return "Not signed in"
  if (state.needsLogin) return "Sign in again"
  if (!state.reading) return state.message || "Waiting for the first reading"
  var age = ageSeconds(state.reading, state.now)
  var base = formatValue(state.reading.v, state.unit) + " " + unitLabel(state.unit) + ", " + trendWords(state.reading.trend)
  return base + " · " + formatAgeLong(age)
}

// Points for the sparkline: x in [0,1] across the window, y in [0,1] from the
// bottom of the plotted range. Readings outside the window are dropped.
function sparkPoints(readings, nowSeconds, windowMinutes, yMin, yMax) {
  var span = windowMinutes * 60
  var out = []
  if (!readings) return out
  for (var i = readings.length - 1; i >= 0; i--) {
    var r = readings[i]
    var dx = nowSeconds - r.t
    if (dx < 0 || dx > span) continue
    var y = (Math.min(yMax, Math.max(yMin, r.v)) - yMin) / (yMax - yMin)
    out.push({ x: 1 - dx / span, y: y, v: r.v })
  }
  return out
}

function clampInt(value, fallback, low, high) {
  var n = parseInt(String(value), 10)
  if (!isFinite(n)) n = fallback
  return Math.max(low, Math.min(high, n))
}

// Thresholds are entered in mg/dL. They keep their order no matter what is
// typed into the settings, so "low" can never sit above "high".
function thresholds(settings) {
  var urgentLow = clampInt(settings.urgentLow, 55, 40, 100)
  var low = Math.max(urgentLow + 1, clampInt(settings.low, 70, 50, 150))
  var high = Math.max(low + 10, clampInt(settings.high, 180, 120, 400))
  return { urgentLow: urgentLow, low: low, high: high }
}

// Seconds until the next poll. Fast while signed out (the check is local and
// costs nothing), the configured cadence while healthy, and never before a
// back-off Dexcom asked for.
function nextPollSeconds(state, refreshSec, nowSeconds) {
  if (state.needsLogin) return 5
  if (state.retryAfter > nowSeconds) return Math.max(refreshSec, state.retryAfter - nowSeconds + 1)
  return refreshSec
}
