// Runs Model.js (a QML .pragma library) under node and checks the logic the
// bar depends on. node --test tests/model.test.mjs
import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import vm from "node:vm";

const src = readFileSync(new URL("../Model.js", import.meta.url), "utf8").replace(/^\.pragma library\s*$/m, "");
const M = {};
vm.runInNewContext(src + "\n;Object.assign(__out, {" +
  "trendArrow, trendWords, sanitize, parseHelperOutput, formatValue, unitLabel, delta, formatDelta," +
  "classify, ageSeconds, formatAge, formatAgeLong, isStale, barLabel, headline, sparkPoints," +
  "clampInt, thresholds, nextPollSeconds });", { __out: M });

const T = { urgentLow: 55, low: 70, high: 180 };
// Objects built inside the vm have that realm's prototypes; compare as data.
const plain = (x) => JSON.parse(JSON.stringify(x));
const NOW = 1760112345;

test("trend arrows cover every Dexcom trend and nothing else", () => {
  assert.equal(M.trendArrow("Flat"), "→");
  assert.equal(M.trendArrow("DoubleDown"), "⇊");
  assert.equal(M.trendArrow("None"), "");
  assert.equal(M.trendArrow("__proto__"), "");
  assert.equal(M.trendWords("constructor"), "no trend");
});

test("helper output is validated, not trusted", () => {
  const doc = M.parseHelperOutput(JSON.stringify({
    ok: true, source: "live", fetched_at: NOW, account: "k***@example.com",
    readings: [
      { t: NOW - 300, v: 138, trend: "FortyFiveUp" },
      { t: NOW, v: 142, trend: "Flat" },
      { t: NOW - 600, v: "131", trend: "Flat" },
      { t: NOW - 900, v: 9000, trend: "Flat" },
      { t: NOW - 1200, v: 120, trend: "<img src=x>" },
      { t: 12.5, v: 120, trend: "Flat" },
    ],
  }));
  assert.equal(doc.ok, true);
  assert.equal(doc.source, "live");
  assert.deepEqual(plain(doc.readings.map(r => [r.v, r.trend])), [[142, "Flat"], [138, "FortyFiveUp"], [120, "None"]]);
  assert.equal(M.parseHelperOutput("not json"), null);
  assert.equal(M.parseHelperOutput("[1,2]"), null);
  const evil = M.parseHelperOutput(JSON.stringify({ ok: false, message: "<img src='http://x/'>‮bad", readings: [] }));
  assert.ok(!evil.message.includes("<"));
  assert.ok(!evil.message.includes("‮"));
});

test("units", () => {
  assert.equal(M.formatValue(142, "mgdl"), "142");
  assert.equal(M.formatValue(142, "mmol"), "7.9");
  assert.equal(M.formatValue(54, "mmol"), "3.0");
  assert.equal(M.formatValue(NaN, "mgdl"), "--");
});

test("delta only across a real five-minute step", () => {
  assert.deepEqual(plain(M.delta([{ t: NOW, v: 142 }, { t: NOW - 300, v: 138 }])), { mgdl: 4, minutes: 5 });
  assert.equal(M.delta([{ t: NOW, v: 142 }, { t: NOW - 3600, v: 100 }]), null);
  assert.equal(M.formatDelta({ mgdl: -6, minutes: 5 }, "mgdl"), "−6 mg/dL in 5 min");
  assert.equal(M.formatDelta({ mgdl: 0, minutes: 5 }, "mgdl"), "±0 mg/dL in 5 min");
});

test("bands", () => {
  assert.equal(M.classify(50, T), "urgentLow");
  assert.equal(M.classify(65, T), "low");
  assert.equal(M.classify(70, T), "inRange");
  assert.equal(M.classify(180, T), "inRange");
  assert.equal(M.classify(181, T), "high");
  assert.equal(M.classify(undefined, T), "");
});

test("thresholds keep their order whatever is typed", () => {
  assert.deepEqual(plain(M.thresholds({ urgentLow: 55, low: 70, high: 180 })), T);
  const t = M.thresholds({ urgentLow: 100, low: 50, high: 120 });
  assert.ok(t.urgentLow < t.low && t.low < t.high);
  assert.deepEqual(plain(M.thresholds({ urgentLow: "x", low: null, high: undefined })), T);
});

test("the bar never shows a stale number without its age", () => {
  const opts = { unit: "mgdl", staleMinutes: 10, signedIn: true };
  assert.equal(M.barLabel({ t: NOW - 120, v: 142, trend: "Flat" }, NOW, opts), "142 →");
  assert.equal(M.barLabel({ t: NOW - 11 * 60, v: 142, trend: "Flat" }, NOW, opts), "142 → 11m");
  assert.equal(M.barLabel({ t: NOW - 3 * 3600, v: 142, trend: "Flat" }, NOW, opts), "142 → 3h");
  assert.equal(M.barLabel({ t: NOW - 120, v: 142, trend: "Flat" }, NOW, { ...opts, alwaysShowAge: true }), "142 → 2m");
  assert.equal(M.barLabel({ t: NOW - 120, v: 142, trend: "Flat" }, NOW, { ...opts, showTrend: false }), "142");
  assert.equal(M.barLabel(null, NOW, opts), "G7 --");
  assert.equal(M.barLabel(null, NOW, { ...opts, signedIn: false }), "G7");
  assert.ok(M.isStale(-1, 10));
});

test("ages", () => {
  assert.equal(M.formatAge(30), "now");
  assert.equal(M.formatAge(5 * 60), "5m");
  assert.equal(M.formatAgeLong(60), "1 minute ago");
  assert.equal(M.formatAgeLong(7200), "2 hours ago");
});

test("headline", () => {
  assert.equal(M.headline({ signedIn: false }), "Not signed in");
  assert.equal(M.headline({ signedIn: true, needsLogin: true }), "Sign in again");
  assert.equal(M.headline({ signedIn: true, reading: { t: NOW - 180, v: 142, trend: "Flat" }, now: NOW, unit: "mgdl" }),
    "142 mg/dL, steady · 3 minutes ago");
});

test("sparkline points stay in the window and the unit square", () => {
  const readings = [{ t: NOW, v: 400 }, { t: NOW - 5400, v: 100 }, { t: NOW - 20000, v: 100 }];
  const pts = M.sparkPoints(readings, NOW, 180, 40, 300);
  assert.equal(pts.length, 2);
  assert.equal(pts[1].x, 1);
  assert.equal(pts[1].y, 1);
  assert.ok(pts.every(p => p.x >= 0 && p.x <= 1 && p.y >= 0 && p.y <= 1));
});

test("polling never runs ahead of a Dexcom back-off", () => {
  assert.equal(M.nextPollSeconds({ needsLogin: true, retryAfter: 0 }, 60, NOW), 5);
  assert.equal(M.nextPollSeconds({ needsLogin: false, retryAfter: 0 }, 60, NOW), 60);
  assert.equal(M.nextPollSeconds({ needsLogin: false, retryAfter: NOW + 1800 }, 60, NOW), 1801);
});
