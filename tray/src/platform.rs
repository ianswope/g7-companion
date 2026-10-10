//! The few things that differ per operating system: the sign-in dialog, a
//! message box, start-at-login, and opening the settings file.
//!
//! Dialogs use what ships with the OS (AppleScript on macOS, Windows Forms
//! through Windows PowerShell on Windows, zenity on Linux) so the app needs
//! no GUI toolkit. Nothing secret is ever passed as a command-line argument:
//! the scripts are fixed text, and the password comes back on a pipe.

use crate::poller::Credentials;
use crate::share::Region;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};

const APP_NAME: &str = "G7 Companion";
const MAX_DIALOG_OUTPUT: u64 = 4096;

/// Keep only printable text, without quotes or backslashes, for embedding in
/// a fixed dialog script.
fn plain(text: &str, limit: usize) -> String {
    text.chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '\\' | '\'' | '`' | '$' | '<' | '>' | '&'))
        .take(limit)
        .collect()
}

fn run_capture(mut cmd: Command, stdin: Option<&str>) -> Option<String> {
    cmd.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = cmd.spawn().ok()?;
    if let Some(input) = stdin {
        let mut pipe = child.stdin.take()?;
        pipe.write_all(input.as_bytes()).ok()?;
    }
    let mut out = Vec::new();
    child.stdout.take()?.take(MAX_DIALOG_OUTPUT + 1).read_to_end(&mut out).ok()?;
    let status = child.wait().ok()?;
    if !status.success() || out.len() as u64 > MAX_DIALOG_OUTPUT {
        return None;
    }
    String::from_utf8(out).ok()
}

fn parse_answer(text: &str, by_id: bool) -> Option<Credentials> {
    let mut lines = text.splitn(3, '\n');
    let region = Region::parse(lines.next()?)?;
    let account = lines.next()?.trim().to_string();
    let password = lines.next()?.trim_end_matches(['\r', '\n']).to_string();
    if account.is_empty() || password.is_empty() {
        return None;
    }
    Some(Credentials {
        region,
        account_name: (!by_id).then(|| account.clone()),
        account_id: by_id.then_some(account),
        password,
    })
}

const ACCOUNT_PROMPT: &str = "Dexcom username of the person wearing the G7: the one sharing, not a follower. Usually an email address, or a phone number like +15551234567.";
const ID_PROMPT: &str = "Dexcom account ID: sign in at uam1.dexcom.com (US) or uam2.dexcom.com (elsewhere) and copy the long ID from the address bar.";

/// Ask for the sharer's credentials. None if the person cancelled.
pub fn ask_credentials(by_id: bool) -> Option<Credentials> {
    let prompt = if by_id { ID_PROMPT } else { ACCOUNT_PROMPT };
    #[cfg(target_os = "macos")]
    {
        let script = format!(
            r#"set regionChoice to choose from list {{"United States", "Outside the US", "Japan"}} with title "{APP_NAME}" with prompt "Where is the Dexcom account registered?" default items {{"United States"}}
if regionChoice is false then error number -128
set acct to text returned of (display dialog "{prompt}" default answer "" with title "{APP_NAME}" buttons {{"Cancel", "Next"}} default button "Next" cancel button "Cancel")
set pw to text returned of (display dialog "Dexcom password:" default answer "" with hidden answer with title "{APP_NAME}" buttons {{"Cancel", "Sign In"}} default button "Sign In" cancel button "Cancel")
return (item 1 of regionChoice) & linefeed & acct & linefeed & pw"#
        );
        let mut cmd = Command::new("/usr/bin/osascript");
        cmd.arg("-");
        return run_capture(cmd, Some(&script)).and_then(|t| parse_answer(&t, by_id));
    }
    #[cfg(windows)]
    {
        let script = format!(
            r#"Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
[System.Windows.Forms.Application]::EnableVisualStyles()
$f = New-Object System.Windows.Forms.Form
$f.Text = '{APP_NAME} sign-in'
$f.FormBorderStyle = 'FixedDialog'; $f.MaximizeBox = $false; $f.MinimizeBox = $false
$f.StartPosition = 'CenterScreen'; $f.TopMost = $true; $f.AutoSize = $true; $f.AutoSizeMode = 'GrowAndShrink'
$f.Padding = New-Object System.Windows.Forms.Padding(12)
$t = New-Object System.Windows.Forms.TableLayoutPanel
$t.ColumnCount = 1; $t.AutoSize = $true; $t.Dock = 'Fill'
function L($s) {{ $l = New-Object System.Windows.Forms.Label; $l.Text = $s; $l.AutoSize = $true; $l.MaximumSize = New-Object System.Drawing.Size(360, 0); $l.Margin = New-Object System.Windows.Forms.Padding(0,8,0,2); $t.Controls.Add($l) }}
L 'Where is the Dexcom account registered?'
$r = New-Object System.Windows.Forms.ComboBox
$r.DropDownStyle = 'DropDownList'; $r.Width = 360
[void]$r.Items.AddRange(@('United States','Outside the US','Japan')); $r.SelectedIndex = 0
$t.Controls.Add($r)
L '{prompt}'
$a = New-Object System.Windows.Forms.TextBox; $a.Width = 360; $t.Controls.Add($a)
L 'Dexcom password:'
$p = New-Object System.Windows.Forms.TextBox; $p.Width = 360; $p.UseSystemPasswordChar = $true; $t.Controls.Add($p)
$b = New-Object System.Windows.Forms.FlowLayoutPanel; $b.FlowDirection = 'RightToLeft'; $b.AutoSize = $true; $b.Dock = 'Fill'
$ok = New-Object System.Windows.Forms.Button; $ok.Text = 'Sign In'; $ok.DialogResult = 'OK'
$cancel = New-Object System.Windows.Forms.Button; $cancel.Text = 'Cancel'; $cancel.DialogResult = 'Cancel'
$b.Controls.Add($ok); $b.Controls.Add($cancel); $t.Controls.Add($b)
$f.AcceptButton = $ok; $f.CancelButton = $cancel
$f.Controls.Add($t)
if ($f.ShowDialog() -ne 'OK') {{ exit 1 }}
[Console]::OutputEncoding = [System.Text.Encoding]::UTF8
[Console]::Out.Write($r.SelectedItem + "`n" + $a.Text + "`n" + $p.Text)
exit 0
"#
        );
        return run_capture(powershell(&script), None).and_then(|t| parse_answer(&t, by_id));
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let zenity = Path::new("/usr/bin/zenity");
        if !zenity.exists() {
            return None;
        }
        let mut cmd = Command::new(zenity);
        cmd.args([
            "--forms",
            &format!("--title={APP_NAME} sign-in"),
            "--text",
            prompt,
            "--add-combo=Region",
            "--combo-values=United States|Outside the US|Japan",
            "--add-entry=Account",
            "--add-password=Password",
            "--separator=\n",
        ]);
        run_capture(cmd, None).and_then(|t| {
            let t = t.trim_end_matches('\n');
            let fixed = if t.starts_with('\n') { format!("United States{t}") } else { t.to_string() };
            parse_answer(&fixed, by_id)
        })
    }
}

/// A message box. Blocks until dismissed, so call it off the UI thread.
pub fn alert(message: &str) {
    let msg = plain(message, 400);
    #[cfg(target_os = "macos")]
    {
        let script = format!(r#"display alert "{APP_NAME}" message "{msg}""#);
        let mut cmd = Command::new("/usr/bin/osascript");
        cmd.arg("-");
        let _ = run_capture(cmd, Some(&script));
    }
    #[cfg(windows)]
    {
        let script = format!(
            "Add-Type -AssemblyName System.Windows.Forms\n[void][System.Windows.Forms.MessageBox]::Show('{msg}', '{APP_NAME}')\n"
        );
        let _ = run_capture(powershell(&script), None);
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let mut cmd = Command::new("/usr/bin/zenity");
        cmd.args(["--info", &format!("--title={APP_NAME}"), "--no-markup", "--text", &msg]);
        let _ = run_capture(cmd, None);
    }
}

#[cfg(windows)]
fn powershell(script: &str) -> Command {
    // -EncodedCommand takes UTF-16LE base64. The script is fixed text with no
    // secrets in it; it is encoded only so quoting cannot change its meaning.
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
    let exe = Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe");
    let mut cmd = Command::new(exe);
    cmd.args(["-NoProfile", "-NonInteractive", "-STA", "-ExecutionPolicy", "Bypass", "-EncodedCommand"]);
    cmd.arg(base64(&utf16));
    cmd
}

#[cfg(any(windows, test))]
fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Open the settings file in the default text editor.
pub fn open_in_editor(path: &Path) {
    #[cfg(target_os = "macos")]
    let _ = Command::new("/usr/bin/open").arg("-t").arg(path).spawn();
    #[cfg(windows)]
    {
        let root = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into());
        let _ = Command::new(Path::new(&root).join("System32\\notepad.exe")).arg(path).spawn();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    let _ = Command::new("/usr/bin/xdg-open").arg(path).spawn();
}

// ── start at login ──────────────────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn launch_agent_path() -> Option<std::path::PathBuf> {
    Some(dirs::home_dir()?.join("Library/LaunchAgents/io.github.ianswope.g7-companion.plist"))
}

pub fn autostart_enabled() -> bool {
    #[cfg(target_os = "macos")]
    return launch_agent_path().is_some_and(|p| p.exists());
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_CURRENT_USER;
        let run = winreg::RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
        return run.and_then(|k| k.get_value::<String, _>(APP_NAME)).is_ok();
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    return dirs::config_dir().is_some_and(|d| d.join("autostart/g7-companion.desktop").exists());
}

pub fn set_autostart(enable: bool) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    {
        let path = launch_agent_path().ok_or("no home directory")?;
        if !enable {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        let exe = exe.to_string_lossy();
        let escaped = exe.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>io.github.ianswope.g7-companion</string>
  <key>ProgramArguments</key><array><string>{escaped}</string></array>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
"#
        );
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(&path, plist).map_err(|e| e.to_string())
    }
    #[cfg(windows)]
    {
        use winreg::enums::{HKEY_CURRENT_USER, KEY_SET_VALUE};
        let run = winreg::RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags("Software\\Microsoft\\Windows\\CurrentVersion\\Run", KEY_SET_VALUE)
            .map_err(|e| e.to_string())?;
        if enable {
            run.set_value(APP_NAME, &format!("\"{}\"", exe.display())).map_err(|e| e.to_string())
        } else {
            match run.delete_value(APP_NAME) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            }
        }
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        let dir = dirs::config_dir().ok_or("no config directory")?.join("autostart");
        let path = dir.join("g7-companion.desktop");
        if !enable {
            return match std::fs::remove_file(&path) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
                _ => Ok(()),
            };
        }
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=G7 Companion\nExec=\"{}\"\nX-GNOME-Autostart-enabled=true\n",
            exe.display()
        );
        std::fs::write(&path, entry).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn answers() {
        let c = parse_answer("United States\nkid@example.com\ncorrect horse", false).unwrap();
        assert_eq!(c.region, Region::Us);
        assert_eq!(c.account_name.as_deref(), Some("kid@example.com"));
        assert_eq!(c.password, "correct horse");
        let c = parse_answer("Japan\n11111111-2222-3333-4444-555555555555\npw\n", true).unwrap();
        assert_eq!(c.region, Region::Jp);
        assert!(c.account_name.is_none());
        assert!(parse_answer("Mars\na\nb", false).is_none());
        assert!(parse_answer("United States\n\nb", false).is_none());
    }

    #[test]
    fn plain_strips_script_syntax() {
        assert_eq!(plain("He said \"hi\" & left\\'`$x\n<b>", 100), "He said hi  leftxb");
    }

    #[test]
    fn base64_matches_reference() {
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
    }
}
