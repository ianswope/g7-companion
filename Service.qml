import QtQuick
import Quickshell
import Quickshell.Io
import "Model.js" as Model

// Runs bin/g7-share on a cadence and holds what it last said.
//
// The shell never talks to Dexcom itself. The helper does the network call,
// the session cache and the credential file; this file only starts it with a
// fixed argv and a cleared environment, caps what it prints while it prints
// it, and validates the shape before anything is drawn.
Item {
  id: root

  property var settings: ({})

  function setting(name, fallback) {
    var value = settings ? settings[name] : undefined
    return value === undefined || value === null ? fallback : value
  }

  readonly property string unit: setting("unit", "mgdl") === "mmol" ? "mmol" : "mgdl"
  readonly property var thresholds: Model.thresholds({
    urgentLow: setting("urgentLow", 55),
    low: setting("low", 70),
    high: setting("high", 180)
  })
  readonly property int refreshSec: Model.clampInt(setting("refreshSec", 60), 60, 30, 600)
  readonly property int staleMinutes: Model.clampInt(setting("staleMinutes", 10), 10, 6, 60)
  readonly property int historyMinutes: Model.clampInt(setting("historyMinutes", 180), 180, 60, 720)

  // ── what the helper last said ─────────────────────────────────────────────
  property bool ready: false
  property bool signedIn: false
  property bool needsLogin: false
  property bool live: false
  property string account: ""
  property string errorCode: ""
  property string message: ""
  property int retryAfter: 0
  property int fetchedAt: 0
  property var readings: []
  readonly property var latest: readings.length > 0 ? readings[0] : null

  // Wall clock in seconds, ticking so ages and staleness move between polls.
  property int now: Math.floor(Date.now() / 1000)
  readonly property int ageSec: Model.ageSeconds(latest, now)
  readonly property bool stale: Model.isStale(ageSec, staleMinutes)
  readonly property string band: Model.classify(latest ? latest.v : NaN, thresholds)
  readonly property var change: Model.delta(readings)

  readonly property string barText: Model.barLabel(latest, now, {
    unit: unit,
    staleMinutes: staleMinutes,
    signedIn: signedIn && !needsLogin,
    showTrend: setting("showTrend", true) !== false,
    alwaysShowAge: setting("alwaysShowAge", false) === true
  })
  readonly property string headline: Model.headline({
    signedIn: signedIn, needsLogin: needsLogin, reading: latest,
    now: now, unit: unit, message: message
  })

  function localPath(relative) {
    return decodeURIComponent(String(Qt.resolvedUrl(relative)).replace(/^file:\/\//, ""))
  }
  readonly property string helperPath: localPath("bin/g7-share")

  // Fixed interpreter, isolated mode, no site directory: nothing on PATH or
  // in PYTHON* variables decides what runs, and the environment is cleared.
  function helperArgv(args) {
    return ["/usr/bin/python3", "-I", "-S", helperPath].concat(args)
  }

  // ── polling ───────────────────────────────────────────────────────────────
  readonly property int maxHelperBytes: 131072
  property string buf: ""
  property bool overflow: false
  property bool pendingRefresh: false

  function refresh() {
    if (readProc.running) { pendingRefresh = true; return }
    buf = ""
    overflow = false
    readProc.command = helperArgv(["read", "--minutes", String(historyMinutes), "--max-count", "288"])
    readProc.running = true
  }

  function apply(text) {
    var doc = Model.parseHelperOutput(text)
    if (!doc) {
      errorCode = "helper"
      message = "The G7 helper returned something unexpected."
      return
    }
    ready = true
    signedIn = doc.error !== "signed_out"
    needsLogin = doc.needsLogin
    live = doc.ok && doc.source === "live"
    account = doc.account
    errorCode = doc.ok ? "" : doc.error
    message = doc.ok ? "" : doc.message
    retryAfter = doc.retryAfter
    fetchedAt = doc.fetchedAt
    readings = doc.readings
    if (doc.error === "signed_out") readings = []
  }

  function schedule() {
    pollTimer.interval = Model.nextPollSeconds(
      { needsLogin: needsLogin || !signedIn, retryAfter: retryAfter }, refreshSec, now) * 1000
    pollTimer.restart()
  }

  Process {
    id: readProc
    running: false
    command: []
    clearEnvironment: true
    environment: ({ "LANG": "C.UTF-8" })
    stdout: SplitParser {
      splitMarker: ""
      onRead: function(chunk) {
        if (root.overflow) return
        root.buf += chunk
        if (root.buf.length > root.maxHelperBytes) {
          root.overflow = true
          root.buf = ""
          readProc.signal(15)
          readKill.restart()
        }
      }
    }
    onRunningChanged: if (running) readDeadline.restart(); else readDeadline.stop()
    onExited: function(exitCode) {
      readKill.stop()
      var text = root.buf
      root.buf = ""
      root.now = Math.floor(Date.now() / 1000)
      if (root.overflow) {
        root.errorCode = "helper"
        root.message = "The G7 helper printed more than expected."
      } else {
        root.apply(text)
      }
      root.schedule()
      if (root.pendingRefresh) {
        root.pendingRefresh = false
        root.refresh()
      }
    }
  }

  // The helper bounds its own network time; this is the backstop if it hangs
  // anyway. TERM, then KILL.
  Timer {
    id: readDeadline
    interval: 40000
    repeat: false
    onTriggered: if (readProc.running) { readProc.signal(15); readKill.restart() }
  }
  Timer {
    id: readKill
    interval: 2000
    repeat: false
    onTriggered: if (readProc.running) readProc.signal(9)
  }

  Timer {
    id: pollTimer
    interval: 1000
    repeat: false
    running: true
    onTriggered: root.refresh()
  }

  Timer {
    interval: 15000
    repeat: true
    running: true
    onTriggered: root.now = Math.floor(Date.now() / 1000)
  }

  // ── sign in and out ───────────────────────────────────────────────────────
  // Sign-in is interactive and happens in a terminal, so the password is typed
  // into getpass and never passes through the shell, a setting or an argv.
  function signIn(useAccountId) {
    if (signInProc.running) return
    var args = ["login"]
    if (useAccountId === true) args.push("--account-id")
    signInProc.command = ["/usr/bin/setsid", "-f", "/usr/bin/xdg-terminal-exec",
      "--app-id=org.omarchy.terminal", "--title=G7 Companion sign-in", "-e"].concat(helperArgv(args))
    signInProc.running = true
    pollTimer.interval = 5000
    pollTimer.restart()
  }

  function signOut() {
    if (logoutProc.running) return
    logoutProc.command = helperArgv(["logout"])
    logoutProc.running = true
  }

  Process {
    id: signInProc
    running: false
    command: []
  }

  Process {
    id: logoutProc
    running: false
    command: []
    clearEnvironment: true
    environment: ({ "LANG": "C.UTF-8" })
    onExited: function() { root.refresh() }
  }

  Component.onDestruction: {
    if (readProc.running) readProc.signal(15)
    if (logoutProc.running) logoutProc.signal(15)
  }
}
