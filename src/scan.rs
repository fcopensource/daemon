//! Malware scan: runs the system's real antivirus engine and reports the result.
//!
//! Windows uses Microsoft Defender (`MpCmdRun.exe`); macOS and Linux use ClamAV
//! (`clamscan`). DAEMON has no detection logic of its own and only reports what
//! the engine finds: scans run with remediation disabled, so nothing is deleted
//! or quarantined by DAEMON (Defender's real-time protection still acts as usual).

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScanKind {
    /// Places malware usually lives (Defender quick scan / ClamAV on the home folder).
    Quick,
    /// Every file on every drive. Can take hours.
    Full,
}

impl ScanKind {
    pub fn label(self) -> &'static str {
        match self {
            ScanKind::Quick => "QUICK SCAN",
            ScanKind::Full => "FULL SCAN",
        }
    }
}

/// One detection: the engine's threat name and where it was found.
#[derive(Clone, Debug, PartialEq)]
pub struct Threat {
    pub name: String,
    pub path: String,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub enum ScanStatus {
    #[default]
    Idle,
    Running,
    Clean,
    Threats(Vec<Threat>),
    Cancelled,
    Error(String),
}

#[derive(Clone, Default)]
pub struct ScanState {
    pub status: ScanStatus,
    pub kind: Option<ScanKind>,
    /// Engine name, e.g. "Microsoft Defender".
    pub engine: String,
    pub started: Option<Instant>,
    pub took: Option<Duration>,
    pub finished_at: Option<chrono::DateTime<chrono::Local>>,
    /// Raw engine output of the last scan.
    pub log: String,
    /// Protection status rows (label, value, healthy).
    pub protection: Vec<(String, String, bool)>,
}

pub type SharedScan = Arc<Mutex<ScanState>>;

pub struct Scanner {
    pub shared: SharedScan,
    child: Arc<Mutex<Option<Child>>>,
}

impl Default for Scanner {
    fn default() -> Self {
        Self { shared: SharedScan::default(), child: Arc::new(Mutex::new(None)) }
    }
}

/// Runs console tools without flashing a console window in the windowed build.
fn command(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

/// `MpCmdRun.exe` from the newest Defender platform folder, else the classic location.
#[cfg(windows)]
fn defender_exe() -> Option<PathBuf> {
    let platform = PathBuf::from(std::env::var_os("ProgramData")?).join("Microsoft\\Windows Defender\\Platform");
    let mut versions: Vec<PathBuf> = std::fs::read_dir(platform)
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path().join("MpCmdRun.exe"))
        .filter(|p| p.is_file())
        .collect();
    versions.sort();
    versions.pop().or_else(|| {
        let classic = PathBuf::from(std::env::var_os("ProgramFiles")?).join("Windows Defender\\MpCmdRun.exe");
        classic.is_file().then_some(classic)
    })
}

#[cfg(not(windows))]
fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

/// The scan command for this platform, and the engine's display name.
fn scan_command(kind: ScanKind) -> Result<(Command, &'static str), String> {
    #[cfg(windows)]
    {
        let exe = defender_exe().ok_or("Microsoft Defender (MpCmdRun.exe) was not found. Is it disabled or replaced by another antivirus?")?;
        let mut cmd = command(exe);
        let scan_type = match kind {
            ScanKind::Quick => "1",
            ScanKind::Full => "2",
        };
        cmd.args(["-Scan", "-ScanType", scan_type, "-DisableRemediation"]);
        Ok((cmd, "Microsoft Defender"))
    }
    #[cfg(not(windows))]
    {
        let mut cmd = command("clamscan");
        cmd.args(["-r", "-i", "--stdout"]);
        match kind {
            ScanKind::Quick => cmd.arg(home()),
            ScanKind::Full => cmd.args(["--exclude-dir=^/(proc|sys|dev|run)", "/"]),
        };
        Ok((cmd, "ClamAV"))
    }
}

/// Parses `MpCmdRun -Scan` output. Exit code 0 means no threats, 2 means threats found.
pub fn parse_defender(code: Option<i32>, out: &str) -> ScanStatus {
    let mut threats = Vec::new();
    let mut name: Option<String> = None;
    for line in out.lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let (key, value) = (key.trim(), value.trim());
        match key {
            "Threat" => name = Some(value.to_string()),
            // "file : C:\path" (the path itself contains ':' after the drive letter).
            "file" | "folder" | "regkey" | "process" | "service" | "startup" => {
                if let Some(n) = &name {
                    threats.push(Threat { name: n.clone(), path: value.to_string() });
                }
            }
            _ => {}
        }
    }
    // A threat with no listed resource still counts.
    if let Some(n) = name {
        if !threats.iter().any(|t| t.name == n) {
            threats.push(Threat { name: n, path: String::new() });
        }
    }
    let found_none = out.contains("found no threats");
    match code {
        _ if !threats.is_empty() => ScanStatus::Threats(threats),
        Some(0) if found_none || !out.trim().is_empty() => ScanStatus::Clean,
        Some(2) => ScanStatus::Threats(vec![Threat { name: "Threats found (details in Windows Security)".into(), path: String::new() }]),
        _ => ScanStatus::Error(format!(
            "Defender exited with code {}.\n{}",
            code.map_or("?".into(), |c| c.to_string()),
            out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("").trim()
        )),
    }
}

/// Parses `clamscan -i` output: "path: Name FOUND" lines; exit 0 clean, 1 infected, 2 error.
pub fn parse_clam(code: Option<i32>, out: &str) -> ScanStatus {
    let threats: Vec<Threat> = out
        .lines()
        .filter_map(|l| l.strip_suffix(" FOUND"))
        .filter_map(|l| l.rsplit_once(": "))
        .map(|(path, name)| Threat { name: name.to_string(), path: path.to_string() })
        .collect();
    match code {
        _ if !threats.is_empty() => ScanStatus::Threats(threats),
        Some(0) => ScanStatus::Clean,
        _ => ScanStatus::Error(format!("clamscan exited with code {}", code.map_or("?".into(), |c| c.to_string()))),
    }
}

/// Reads a PowerShell 5.1 JSON date ("/Date(1791179952000)/") as local time.
fn ps_date(v: &Value) -> Option<chrono::DateTime<chrono::Local>> {
    let s = v.as_str()?;
    let ms: i64 = s.split('(').nth(1)?.split(|c| c == ')' || c == '+' || c == '-').next()?.parse().ok()?;
    chrono::DateTime::from_timestamp_millis(ms).map(|d| d.with_timezone(&chrono::Local))
}

/// Days since a scan; Defender reports 4294967295 (u32::MAX) for "never".
fn scan_age(v: &Value) -> (String, bool) {
    match v.as_u64() {
        Some(d) if d >= u32::MAX as u64 => ("NEVER".into(), false),
        Some(0) => ("TODAY".into(), true),
        Some(d) => (format!("{d} DAYS AGO"), d <= 14),
        None => ("--".into(), true),
    }
}

/// Protection status rows from `Get-MpComputerStatus` JSON.
pub fn parse_protection(json: &Value) -> Vec<(String, String, bool)> {
    let on = |k: &str| json[k].as_bool();
    let flag = |label: &str, k: &str| {
        let v = on(k);
        (label.to_string(), v.map_or("--", |b| if b { "ON" } else { "OFF" }).to_string(), v != Some(false))
    };
    let mut rows = vec![
        flag("ANTIVIRUS", "AntivirusEnabled"),
        flag("REAL-TIME PROTECTION", "RealTimeProtectionEnabled"),
        flag("BEHAVIOR MONITOR", "BehaviorMonitorEnabled"),
    ];
    if let Some(mode) = json["AMRunningMode"].as_str() {
        rows.push(("ENGINE MODE".into(), mode.to_uppercase(), mode == "Normal"));
    }
    if let Some(d) = ps_date(&json["AntivirusSignatureLastUpdated"]) {
        let days = (chrono::Local::now() - d).num_days();
        rows.push(("SIGNATURES".into(), d.format("%Y-%m-%d %H:%M").to_string(), days <= 7));
    }
    let (q, q_ok) = scan_age(&json["QuickScanAge"]);
    rows.push(("LAST QUICK SCAN".into(), q, q_ok));
    let (f, f_ok) = scan_age(&json["FullScanAge"]);
    rows.push(("LAST FULL SCAN".into(), f, f_ok));
    rows
}

impl Scanner {
    pub fn running(&self) -> bool {
        self.shared.lock().unwrap().status == ScanStatus::Running
    }

    /// Fetches the antivirus protection status in the background.
    pub fn refresh_protection(&self, ctx: egui::Context) {
        let shared = self.shared.clone();
        std::thread::spawn(move || {
            let rows = protection_status();
            shared.lock().unwrap().protection = rows;
            ctx.request_repaint();
        });
    }

    /// Starts a scan on a background thread unless one is already running.
    pub fn start(&self, kind: ScanKind, ctx: egui::Context) {
        if self.running() {
            return;
        }
        let (mut cmd, engine) = match scan_command(kind) {
            Ok(c) => c,
            Err(e) => {
                let mut st = self.shared.lock().unwrap();
                st.status = ScanStatus::Error(e);
                st.kind = Some(kind);
                return;
            }
        };
        let child = cmd.stdout(Stdio::piped()).stderr(Stdio::piped()).stdin(Stdio::null()).spawn();
        let child = match child {
            Ok(c) => c,
            Err(e) => {
                let hint = if cfg!(windows) { String::new() } else { "\nInstall ClamAV (brew install clamav / apt install clamav) and run freshclam.".into() };
                self.shared.lock().unwrap().status = ScanStatus::Error(format!("Could not start {engine}: {e}{hint}"));
                return;
            }
        };
        {
            let mut st = self.shared.lock().unwrap();
            st.status = ScanStatus::Running;
            st.kind = Some(kind);
            st.engine = engine.into();
            st.started = Some(Instant::now());
            st.took = None;
            st.log.clear();
        }
        *self.child.lock().unwrap() = Some(child);
        let (shared, child) = (self.shared.clone(), self.child.clone());
        std::thread::spawn(move || {
            // Take the pipes, then wait without holding the lock so `cancel` can kill the process.
            let (mut stdout, mut stderr) = {
                let mut g = child.lock().unwrap();
                let c = g.as_mut().unwrap();
                (c.stdout.take(), c.stderr.take())
            };
            let err_reader = std::thread::spawn(move || {
                let mut s = String::new();
                if let Some(e) = stderr.as_mut() {
                    let _ = std::io::Read::read_to_string(e, &mut s);
                }
                s
            });
            let mut out = String::new();
            if let Some(o) = stdout.as_mut() {
                let _ = std::io::Read::read_to_string(o, &mut out);
            }
            out += &err_reader.join().unwrap_or_default();
            let code = child.lock().unwrap().take().and_then(|mut c| c.wait().ok()).and_then(|s| s.code());
            // Scan ages changed; refresh them.
            let protection = protection_status();

            let mut st = shared.lock().unwrap();
            st.protection = protection;
            if st.status == ScanStatus::Running {
                st.status = if cfg!(windows) { parse_defender(code, &out) } else { parse_clam(code, &out) };
            }
            st.took = st.started.map(|s| s.elapsed());
            st.finished_at = Some(chrono::Local::now());
            st.log = out;
            drop(st);
            ctx.request_repaint();
        });
    }

    /// Stops a running scan.
    pub fn cancel(&self) {
        if !self.running() {
            return;
        }
        self.shared.lock().unwrap().status = ScanStatus::Cancelled;
        if let Some(c) = self.child.lock().unwrap().as_mut() {
            let _ = c.kill();
        }
        // Killing MpCmdRun does not stop the scan inside the Defender service.
        #[cfg(windows)]
        if let Some(exe) = defender_exe() {
            std::thread::spawn(move || {
                let _ = command(exe).arg("-Cancel").stdout(Stdio::null()).stderr(Stdio::null()).status();
            });
        }
    }
}

#[cfg(windows)]
fn protection_status() -> Vec<(String, String, bool)> {
    let script = "Get-MpComputerStatus | Select-Object AntivirusEnabled,RealTimeProtectionEnabled,BehaviorMonitorEnabled,\
                  AMRunningMode,AntivirusSignatureLastUpdated,QuickScanAge,FullScanAge | ConvertTo-Json -Compress";
    let out = command("powershell").args(["-NoProfile", "-NonInteractive", "-Command", script]).output();
    match out.ok().and_then(|o| serde_json::from_slice::<Value>(&o.stdout).ok()) {
        Some(json) => parse_protection(&json),
        None => vec![("DEFENDER STATUS".into(), "UNAVAILABLE".into(), false)],
    }
}

#[cfg(not(windows))]
fn protection_status() -> Vec<(String, String, bool)> {
    let version = command("clamscan").arg("--version").output().ok().filter(|o| o.status.success());
    match version {
        Some(o) => vec![("CLAMAV".into(), String::from_utf8_lossy(&o.stdout).trim().to_string(), true)],
        None => vec![("CLAMAV".into(), "NOT INSTALLED".into(), false)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const CLEAN: &str = "Scan starting...\nScan finished.\nScanning C:\\Users\\x found no threats.\n";
    const INFECTED: &str = "Scan starting...\nScan finished.\nScanning C:\\Users\\x found 1 threats.\n\
<===========================LIST OF DETECTED THREATS==========================>\n\
----------------------------- Threat information ------------------------------\n\
Threat                  : Virus:DOS/EICAR_Test_File\n\
Resources               : 1 total\n\
    file                : C:\\Users\\x\\eicar.com\n\
-------------------------------------------------------------------------------\n";

    #[test]
    fn defender_clean_and_infected() {
        assert_eq!(parse_defender(Some(0), CLEAN), ScanStatus::Clean);
        let ScanStatus::Threats(t) = parse_defender(Some(2), INFECTED) else { panic!("expected threats") };
        assert_eq!(t, vec![Threat { name: "Virus:DOS/EICAR_Test_File".into(), path: "C:\\Users\\x\\eicar.com".into() }]);
        assert!(matches!(parse_defender(Some(2), "Scan finished.\n"), ScanStatus::Threats(_)));
        assert!(matches!(parse_defender(Some(5), "CmdTool: Failed with hr = 0x80508023"), ScanStatus::Error(e) if e.contains("0x80508023")));
    }

    #[test]
    fn clam_clean_and_infected() {
        let out = "/home/u/eicar.txt: Win.Test.EICAR_HDB-1 FOUND\n\n----------- SCAN SUMMARY -----------\nInfected files: 1\n";
        let ScanStatus::Threats(t) = parse_clam(Some(1), out) else { panic!("expected threats") };
        assert_eq!(t[0], Threat { name: "Win.Test.EICAR_HDB-1".into(), path: "/home/u/eicar.txt".into() });
        assert_eq!(parse_clam(Some(0), "Infected files: 0\n"), ScanStatus::Clean);
        assert!(matches!(parse_clam(Some(2), "error"), ScanStatus::Error(_)));
    }

    #[test]
    fn protection_rows() {
        let rows = parse_protection(&json!({
            "AntivirusEnabled": true,
            "RealTimeProtectionEnabled": false,
            "AMRunningMode": "Normal",
            "QuickScanAge": 3,
            "FullScanAge": 4294967295u64
        }));
        let get = |k: &str| rows.iter().find(|r| r.0 == k).cloned().unwrap();
        assert_eq!(get("ANTIVIRUS"), ("ANTIVIRUS".into(), "ON".into(), true));
        assert_eq!(get("REAL-TIME PROTECTION").2, false);
        assert_eq!(get("LAST QUICK SCAN").1, "3 DAYS AGO");
        assert_eq!(get("LAST FULL SCAN"), ("LAST FULL SCAN".into(), "NEVER".into(), false));
    }

    /// Runs a real quick scan (minutes). Run with `cargo test -- --ignored live_quick_scan --nocapture`.
    #[test]
    #[ignore]
    fn live_quick_scan() {
        for row in protection_status() {
            println!("{row:?}");
        }
        let scanner = Scanner::default();
        scanner.start(ScanKind::Quick, egui::Context::default());
        while scanner.running() {
            std::thread::sleep(Duration::from_secs(1));
        }
        let st = scanner.shared.lock().unwrap().clone();
        println!("{:?} in {:?}\n{}", st.status, st.took, st.log);
        assert!(matches!(st.status, ScanStatus::Clean | ScanStatus::Threats(_)), "{:?}", st.status);
    }

    #[test]
    fn powershell_dates() {
        let d = ps_date(&json!("/Date(1791179952000)/")).unwrap();
        assert_eq!(d.timestamp(), 1_791_179_952);
        assert!(ps_date(&json!("nope")).is_none());
    }
}
