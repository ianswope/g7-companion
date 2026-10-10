# G7 Companion

The latest Dexcom G7 glucose reading in your menu bar, system tray or
Omarchy bar, with its trend arrow. It is for the people on the other end of
Dexcom Follow: parents, partners, anyone who wants to glance at a number on
their computer instead of picking up their phone.

It reads the same Dexcom Share service the Follow app uses. The number updates
about once a minute, a minute or two behind the wearer's phone.

**G7 Companion is not a medical device and is not affiliated with Dexcom.**
Keep the Dexcom app and its alerts as your source of truth. Never make a
treatment decision from this number. If Dexcom changes their service, this
may stop working without warning.

## What you need

- The Dexcom username and password of **the person wearing the G7**: the
  account their G7 app is signed into. A follower's own Dexcom account cannot
  read anyone's data. This is the one awkward part, and it is a Dexcom
  limitation. It does not use up one of the ten follower slots and it does not
  sign the wearer out of anything.
- Share turned on in the wearer's G7 app with at least one follower. If you
  already use Dexcom Follow, it is.

A G7 account usually signs in with an email address, or with a phone number
written like `+15551234567`. If sign-in keeps failing with the right password,
use the account ID instead: sign in at
[uam1.dexcom.com](https://uam1.dexcom.com) (US) or
[uam2.dexcom.com](https://uam2.dexcom.com) (elsewhere) and copy the long ID
from the address bar. Every app has a "Sign in with account ID" option.

> Too many failed sign-ins make Dexcom lock the account for a while, and the
> lock also affects the wearer's phone. G7 Companion stops trying the moment a
> password is rejected and waits for you to sign in again. Stop after two or
> three failed attempts yourself, too.

## Install

### macOS (11 or later, Apple Silicon or Intel)

1. Download `G7-Companion-…-macOS.zip` from the
   [latest release](https://github.com/ianswope/g7-companion/releases/latest)
   and double-click it to unzip.
2. Drag **G7 Companion** into Applications and open it.
3. macOS will refuse the first time, because the app is not signed with an
   Apple Developer ID yet. Open **System Settings → Privacy & Security**,
   scroll down, and click **Open Anyway** next to G7 Companion. You only do
   this once.
4. A grey **G7** appears in the menu bar. Click it, choose **Sign in…**, and
   enter the wearer's Dexcom account.

The password goes into your login Keychain. After an update, macOS may ask
once whether G7 Companion can use it; choose **Always Allow**.

### Windows 10 and 11

1. Download `G7-Companion-…-Windows-x64.zip` from the
   [latest release](https://github.com/ianswope/g7-companion/releases/latest),
   right-click it, choose **Extract All**, and put the folder somewhere that
   will stay put, such as your Documents folder.
2. Open `g7-companion.exe`. Windows SmartScreen will warn that the app is
   unrecognized, because it is not code-signed yet. Click **More info**, then
   **Run anyway**.
3. A small grey tile appears in the notification area. If you do not see it,
   click the **^** arrow next to the clock and drag the tile onto the taskbar.
4. Right-click it, choose **Sign in…**, and enter the wearer's Dexcom account.

The password goes into Windows Credential Manager.

### Omarchy

```bash
omarchy plugin add https://github.com/ianswope/g7-companion --enable
```

Click **G7** in the bar, then the sign-in button. A terminal opens and asks for
the region, username and password; the password is typed there and never
passes through the bar. Settings (units, thresholds, how often to check) are
in the bar's widget settings like any other plugin.

The plugin needs nothing beyond the Python 3 that Omarchy already ships.

## Reading it

| You see | It means |
|---|---|
| `142 →` | 142 mg/dL, steady, measured in the last few minutes |
| `142 ↗` `142 ↑` `142 ⇈` | rising slowly, rising, rising fast |
| `142 ↘` `142 ↓` `142 ⇊` | falling slowly, falling, falling fast |
| red | below your low threshold (70 by default) |
| amber | above your high threshold (180 by default) |
| grey with an age, `142 → 25m` | **the number is old**. Nothing new has arrived for that long |

The grey state is the important one. If the wearer's phone loses signal, runs
out of battery or is out of range of the sensor, the last number stays on
screen, so G7 Companion greys it out and shows how old it is after ten minutes
instead of letting it look current.

Click the reading for the last three hours as a chart (Omarchy) or the trend
and change since the previous reading (macOS and Windows).

Set the low and high thresholds to match the ones in your Follow app. On
macOS and Windows, **Edit thresholds…** opens the settings file; change
`low`, `high` or `urgent_low` and save. On Omarchy they are in the widget
settings.

## Privacy

- G7 Companion talks to one place: Dexcom's Share servers
  (`share2.dexcom.com` in the US, `shareous1.dexcom.com` elsewhere,
  `share.dexcom.jp` in Japan). There are no analytics, no update checks and no
  server of mine anywhere in the path.
- The password is stored by your operating system: Keychain on macOS,
  Credential Manager on Windows. On Omarchy it is in
  `~/.config/g7-companion/credentials.json`, readable only by your user.
- The latest readings and the Dexcom session are cached on your computer so
  the app does not have to sign in to Dexcom every minute, which is what
  causes lockouts.
- **Sign out** deletes the password, the session and the cached readings.

## Removing it

**macOS:** choose **Sign out** from the menu, turn off **Start at login**,
quit, and drag the app to the Trash. The settings folder is
`~/Library/Application Support/G7 Companion`.

**Windows:** right-click the tile, choose **Sign out**, turn off **Start at
login**, quit, and delete the folder. The settings folder is
`%APPDATA%\G7 Companion`.

**Omarchy:** click **Sign out** in the panel, then

```bash
omarchy plugin remove ianswope.g7-companion
```

Signing out removes `~/.config/g7-companion/credentials.json` and the cache in
`~/.local/state/g7-companion/`. If you remove the plugin without signing out
first, delete those two directories by hand; nothing else is written.

## Command line

The macOS and Windows downloads include a `g7` command-line tool that shares
the app's settings and credentials:

```
g7 login [--account-id] [--region us|ous|jp]
g7 read [--json]
g7 status
g7 logout
```

On macOS it is inside the app: `"/Applications/G7 Companion.app/Contents/MacOS/g7" read`.

## How it works

[docs/share-api.md](docs/share-api.md) documents the three Dexcom Share calls
and the error codes, including the lockout rules both clients follow. The
same request shape has been used by [Nightscout](https://nightscout.github.io),
[xDrip](https://github.com/NightscoutFoundation/xDrip) and
[pydexcom](https://github.com/gagebenne/pydexcom) for years; this project owes
them its starting point.

| Part | Where | Built with |
|---|---|---|
| Omarchy bar widget | `Panel.qml`, `Service.qml`, `Model.js` | Quickshell QML |
| Its Share client | `bin/g7-share` | Python standard library |
| macOS and Windows app | `tray/` | Rust, [tray-icon](https://crates.io/crates/tray-icon), [keyring](https://crates.io/crates/keyring) |

Tests:

```bash
python3 -m unittest discover -s tests   # Share client, against a local fake
node --test tests/model.test.mjs        # widget logic
cd tray && cargo test                   # Rust client and poller, same scenarios
```

Releases are built by GitHub Actions from a `v*` tag; see
[.github/workflows/release.yml](.github/workflows/release.yml).

## License

MIT. Dexcom, Dexcom G7, Dexcom Share and Dexcom Follow are trademarks of
Dexcom, Inc., used here only to say what this works with.
