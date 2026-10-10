import QtQuick
import Quickshell
import Quickshell.Io
import qs.Commons
import qs.Ui
import "Model.js" as Model

// G7 Companion: the latest Dexcom G7 reading in the bar, with the trend and,
// whenever it is not fresh, its age. The panel adds the last few hours as a
// sparkline, the change since the previous reading, and sign-in.
//
// This is a glance, not an alarm. The Dexcom app on the phone and the Follow
// app keep the alerts; nothing here should be relied on for treatment.
Panel {
  id: root
  moduleName: "ianswope.g7-companion"
  ipcTarget: "ianswope.g7-companion"
  manageIpc: false

  readonly property color foreground: bar ? bar.foreground : Color.foreground
  readonly property string fontFamily: bar ? bar.fontFamily : Style.font.family
  readonly property color urgent: bar ? bar.urgent : Color.urgent
  readonly property color dim: Qt.darker(foreground, 1.55)
  readonly property color highColor: "#e8a33d"
  readonly property color staleColor: Qt.darker(barForeground, 1.8)

  function bandColor(band, base) {
    if (band === "urgentLow" || band === "low") return root.urgent
    if (band === "high") return root.highColor
    return base
  }

  // A stale reading goes grey whatever its value was. A coloured number that
  // stopped updating is the one failure this widget must never present.
  readonly property color barValueColor: !g7.latest || g7.stale
    ? root.staleColor
    : bandColor(g7.band, barForeground)

  implicitWidth: button.implicitWidth
  implicitHeight: button.implicitHeight

  onOpenedChanged: if (opened) { g7.refresh(); Qt.callLater(function() { keyCatcher.forceActiveFocus() }) }

  Service {
    id: g7
    settings: root.settings
  }

  IpcHandler {
    target: root.ipcTarget
    function open(): void { root.open() }
    function close(): void { root.close() }
    function show(): void { root.open() }
    function hide(): void { root.close() }
    function toggle(): void { root.toggle() }
    function refresh(): void { g7.refresh() }
  }

  TextMetrics {
    id: labelMetrics
    font.family: root.fontFamily
    font.pixelSize: Style.bar.iconFont
    font.bold: true
    text: g7.barText
  }

  BarIconButton {
    id: button
    anchors.fill: parent
    bar: root.bar
    slotSize: labelMetrics.width + Style.space(10)
    iconComponent: Component {
      Item {
        Text {
          anchors.centerIn: parent
          text: g7.barText
          textFormat: Text.PlainText
          color: root.barValueColor
          font.family: root.fontFamily
          font.pixelSize: Style.bar.iconFont
          font.bold: true
          renderType: Text.NativeRendering
        }
      }
    }
    onPressed: function(buttonCode) {
      if (buttonCode === Qt.MiddleButton) g7.refresh()
      else root.toggle()
    }
  }

  KeyboardPanel {
    id: panel
    anchorItem: button
    owner: root
    bar: root.bar
    open: root.opened
    focusTarget: keyCatcher
    contentWidth: panel.fittedContentWidth(Style.space(340))
    contentHeight: panel.fittedContentHeight(column.implicitHeight + Style.space(8), Style.space(560))

    PanelKeyCatcher {
      id: keyCatcher
      anchors.fill: parent
      onCloseRequested: root.close()
      onTabRequested: function(direction) { root.switchPanel(direction) }
      onTextKey: function(t) {
        if (t === "r") g7.refresh()
        else if (t === "s" && (!g7.signedIn || g7.needsLogin)) g7.signIn(false)
      }

      Column {
        id: column
        anchors.top: parent.top
        anchors.left: parent.left
        anchors.right: parent.right
        spacing: Style.space(12)

        PanelHero {
          width: parent.width
          title: "G7 Companion"
          meta: Model.sanitize(g7.headline, 120)
          foreground: root.foreground
          fontFamily: root.fontFamily
          iconComponent: Component {
            Text {
              text: "󰖌"
              textFormat: Text.PlainText
              color: root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.display
            }
          }
        }

        // ── the reading ──────────────────────────────────────────────────────
        Row {
          width: parent.width
          visible: g7.latest !== null
          spacing: Style.space(10)

          Text {
            id: bigValue
            text: g7.latest ? Model.formatValue(g7.latest.v, g7.unit) : "--"
            textFormat: Text.PlainText
            color: g7.stale ? root.dim : root.bandColor(g7.band, root.foreground)
            font.family: root.fontFamily
            font.pixelSize: Style.font.displayLarge * 1.6
            font.bold: true
          }

          Column {
            anchors.verticalCenter: bigValue.verticalCenter
            spacing: Style.space(2)

            Text {
              text: g7.latest ? (Model.trendArrow(g7.latest.trend) + "  " + Model.trendWords(g7.latest.trend)) : ""
              textFormat: Text.PlainText
              color: g7.stale ? root.dim : root.foreground
              font.family: root.fontFamily
              font.pixelSize: Style.font.title
            }

            Text {
              text: Model.unitLabel(g7.unit) + (g7.change ? " · " + Model.formatDelta(g7.change, g7.unit) : "")
              textFormat: Text.PlainText
              color: root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }

            Text {
              text: Model.formatAgeLong(g7.ageSec) + (g7.stale ? " · not updating" : "")
              textFormat: Text.PlainText
              color: g7.stale ? root.urgent : root.dim
              font.family: root.fontFamily
              font.pixelSize: Style.font.caption
            }
          }
        }

        // ── the last few hours ───────────────────────────────────────────────
        Item {
          width: parent.width
          height: Style.space(90)
          visible: g7.readings.length > 1

          Canvas {
            id: spark
            anchors.fill: parent
            readonly property real yMin: 40
            readonly property real yMax: 300
            property var points: Model.sparkPoints(g7.readings, g7.now, g7.historyMinutes, yMin, yMax)
            onPointsChanged: requestPaint()
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()

            function yFor(mgdl) {
              return height - (Math.min(yMax, Math.max(yMin, mgdl)) - yMin) / (yMax - yMin) * height
            }

            onPaint: {
              var ctx = getContext("2d")
              ctx.reset()
              var fg = root.foreground
              // Target range band.
              var top = yFor(g7.thresholds.high)
              var bottom = yFor(g7.thresholds.low)
              ctx.fillStyle = Qt.rgba(fg.r, fg.g, fg.b, 0.07)
              ctx.fillRect(0, top, width, bottom - top)
              ctx.strokeStyle = Qt.rgba(fg.r, fg.g, fg.b, 0.18)
              ctx.lineWidth = 1
              ctx.beginPath()
              ctx.moveTo(0, top); ctx.lineTo(width, top)
              ctx.moveTo(0, bottom); ctx.lineTo(width, bottom)
              ctx.stroke()
              // One dot per reading, coloured by band, like the Dexcom app.
              var r = Math.max(1.5, Style.space(2))
              for (var i = 0; i < points.length; i++) {
                var p = points[i]
                var band = Model.classify(p.v, g7.thresholds)
                ctx.fillStyle = root.bandColor(band, fg)
                ctx.beginPath()
                ctx.arc(p.x * (width - 2 * r) + r, height - p.y * height, r, 0, Math.PI * 2)
                ctx.fill()
              }
            }
          }

          Text {
            anchors.left: parent.left
            anchors.bottom: parent.bottom
            text: Math.round(g7.historyMinutes / 60) + "h ago"
            textFormat: Text.PlainText
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
          }
          Text {
            anchors.right: parent.right
            anchors.bottom: parent.bottom
            text: "now"
            textFormat: Text.PlainText
            color: root.dim
            font.family: root.fontFamily
            font.pixelSize: Style.font.caption
          }
        }

        // ── state that needs the reader ──────────────────────────────────────
        Text {
          width: parent.width
          visible: !g7.signedIn || g7.needsLogin
          text: !g7.signedIn
            ? "Sign in with the Dexcom account the G7 app on the wearer's phone uses: the person sharing, not a follower. Share has to be on in the G7 app, which it already is if anyone uses Dexcom Follow."
            : (g7.message !== "" ? g7.message + " " : "") + "Sign in again to resume. Nothing is sent to Dexcom until you do, so a changed password cannot lock the account."
          textFormat: Text.PlainText
          color: g7.needsLogin && g7.signedIn ? root.urgent : root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        Text {
          width: parent.width
          visible: g7.signedIn && !g7.needsLogin && g7.message !== ""
          text: g7.message + (g7.readings.length > 0 ? " Showing the last reading received." : "")
          textFormat: Text.PlainText
          color: root.dim
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          wrapMode: Text.WordWrap
        }

        PanelSeparator {
          width: parent.width
          foreground: root.foreground
        }

        Row {
          anchors.horizontalCenter: parent.horizontalCenter
          spacing: Style.space(10)

          PanelActionButton {
            iconText: "󰑐"
            tooltipText: "Refresh now"
            foreground: root.foreground
            fontFamily: root.fontFamily
            visible: g7.signedIn && !g7.needsLogin
            onClicked: g7.refresh()
          }

          PanelActionButton {
            iconText: "󰍂"
            tooltipText: "Sign in with username"
            foreground: root.foreground
            fontFamily: root.fontFamily
            bordered: !g7.signedIn || g7.needsLogin
            onClicked: g7.signIn(false)
          }

          PanelActionButton {
            iconText: "󰌆"
            tooltipText: "Sign in with account ID"
            foreground: root.dim
            fontFamily: root.fontFamily
            onClicked: g7.signIn(true)
          }

          PanelActionButton {
            iconText: "󰍃"
            tooltipText: "Sign out and delete stored credentials"
            foreground: root.dim
            fontFamily: root.fontFamily
            visible: g7.signedIn
            onClicked: g7.signOut()
          }
        }

        Text {
          width: parent.width
          visible: g7.signedIn && g7.account !== ""
          text: "Signed in as " + g7.account + (g7.live ? "" : " · offline")
          textFormat: Text.PlainText
          color: Qt.darker(root.dim, 1.15)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          horizontalAlignment: Text.AlignHCenter
          wrapMode: Text.WordWrap
        }

        Text {
          width: parent.width
          text: (!g7.signedIn || g7.needsLogin) ? "s sign in · esc close" : "r refresh · esc close"
          textFormat: Text.PlainText
          color: Qt.darker(root.dim, 1.15)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          horizontalAlignment: Text.AlignHCenter
        }

        Text {
          width: parent.width
          text: "Not a medical device. Alerts and treatment decisions stay with the Dexcom app."
          textFormat: Text.PlainText
          color: Qt.darker(root.dim, 1.15)
          font.family: root.fontFamily
          font.pixelSize: Style.font.caption
          horizontalAlignment: Text.AlignHCenter
          wrapMode: Text.WordWrap
        }
      }
    }
  }
}
