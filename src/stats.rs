//! Background thread that samples system statistics once per second.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use sysinfo::{Components, Disks, Networks, System, Users};

/// Number of samples kept for the graphs (one per second).
pub const HISTORY: usize = 60;

#[derive(Clone, Default)]
pub struct ProcInfo {
    pub pid: u32,
    pub name: String,
    pub cpu: f32,
    pub mem: u64,
}

#[derive(Clone, Default)]
pub struct DiskInfo {
    pub name: String,
    pub mount: String,
    pub used: u64,
    pub total: u64,
}

#[derive(Clone, Default)]
pub struct Snapshot {
    pub hostname: String,
    pub os: String,
    pub kernel: String,
    pub cpu_brand: String,
    pub uptime: u64,

    pub cpu_total: f32,
    pub cores: Vec<f32>,
    pub cpu_history: VecDeque<f32>,

    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,

    pub procs: Vec<ProcInfo>,
    pub proc_count: usize,

    pub net_iface: String,
    pub rx_rate: u64,
    pub tx_rate: u64,
    pub rx_total: u64,
    pub tx_total: u64,
    pub rx_history: VecDeque<f32>,
    pub tx_history: VecDeque<f32>,

    pub disks: Vec<DiskInfo>,

    /// Deep-scan report: (section title, [(key, value)]).
    pub intel: Vec<(String, Vec<(String, String)>)>,
}

pub type Shared = Arc<Mutex<Snapshot>>;

fn push(h: &mut VecDeque<f32>, v: f32) {
    if h.len() >= HISTORY {
        h.pop_front();
    }
    h.push_back(v);
}

fn is_loopback(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "lo" || n.contains("loopback")
}

pub fn spawn(ctx: egui::Context) -> Shared {
    let shared: Shared = Arc::new(Mutex::new(Snapshot::default()));
    let out = shared.clone();

    thread::spawn(move || {
        let mut sys = System::new_all();
        let mut nets = Networks::new_with_refreshed_list();
        let mut disks = Disks::new_with_refreshed_list();
        let mut components = Components::new_with_refreshed_list();
        let mut users = Users::new_with_refreshed_list();

        let mut snap = Snapshot {
            hostname: System::host_name().unwrap_or_default(),
            os: System::long_os_version().unwrap_or_default(),
            kernel: System::kernel_version().unwrap_or_default(),
            cpu_brand: sys.cpus().first().map(|c| c.brand().trim().to_string()).unwrap_or_default(),
            ..Default::default()
        };

        let mut last = Instant::now();
        let mut tick: u64 = 0;
        loop {
            thread::sleep(Duration::from_millis(1000));
            let elapsed = last.elapsed().as_secs_f64().max(0.001);
            last = Instant::now();

            // CPU
            sys.refresh_cpu();
            snap.cpu_total = sys.global_cpu_info().cpu_usage();
            snap.cores = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
            push(&mut snap.cpu_history, snap.cpu_total);

            // Memory
            sys.refresh_memory();
            snap.mem_used = sys.used_memory();
            snap.mem_total = sys.total_memory();
            snap.swap_used = sys.used_swap();
            snap.swap_total = sys.total_swap();
            snap.uptime = System::uptime();

            // Processes
            sys.refresh_processes();
            let ncpu = sys.cpus().len().max(1) as f32;
            let mut procs: Vec<ProcInfo> = sys
                .processes()
                .iter()
                .filter(|(pid, p)| pid.as_u32() != 0 && p.name() != "System Idle Process")
                .map(|(pid, p)| ProcInfo {
                    pid: pid.as_u32(),
                    name: p.name().to_string(),
                    cpu: p.cpu_usage() / ncpu,
                    mem: p.memory(),
                })
                .collect();
            snap.proc_count = procs.len();
            procs.sort_by(|a, b| b.cpu.partial_cmp(&a.cpu).unwrap_or(Ordering::Equal));
            procs.truncate(10);
            snap.procs = procs;

            // Network
            if tick % 30 == 0 {
                nets.refresh_list();
            } else {
                nets.refresh();
            }
            let (mut rx, mut tx, mut rx_total, mut tx_total) = (0u64, 0u64, 0u64, 0u64);
            let mut busiest = (String::from("--"), 0u64);
            for (name, data) in nets.iter() {
                if is_loopback(name) {
                    continue;
                }
                rx += data.received();
                tx += data.transmitted();
                rx_total += data.total_received();
                tx_total += data.total_transmitted();
                let total = data.total_received() + data.total_transmitted();
                if total > busiest.1 {
                    busiest = (name.clone(), total);
                }
            }
            snap.net_iface = busiest.0;
            snap.rx_rate = (rx as f64 / elapsed) as u64;
            snap.tx_rate = (tx as f64 / elapsed) as u64;
            snap.rx_total = rx_total;
            snap.tx_total = tx_total;
            push(&mut snap.rx_history, snap.rx_rate as f32);
            push(&mut snap.tx_history, snap.tx_rate as f32);

            // Disks (slow-changing, refresh every 10s)
            if tick % 10 == 0 {
                disks.refresh();
                snap.disks = disks
                    .iter()
                    .map(|d| DiskInfo {
                        name: d.name().to_string_lossy().into_owned(),
                        mount: d.mount_point().display().to_string(),
                        used: d.total_space().saturating_sub(d.available_space()),
                        total: d.total_space(),
                    })
                    .collect();
            }

            // Deep-scan report (every 3s; first one right away).
            if tick % 3 == 0 {
                components.refresh();
                if tick % 30 == 0 {
                    users.refresh_list();
                }
                snap.intel = build_intel(&sys, &nets, &disks, &components, &users);
            }

            *out.lock().unwrap() = snap.clone();
            ctx.request_repaint();
            tick += 1;
        }
    });

    shared
}

fn pct(used: u64, total: u64) -> String {
    if total == 0 { "--".into() } else { format!("{:.1}%", used as f64 / total as f64 * 100.0) }
}

/// The machine's LAN address. Connecting a UDP socket only picks a route; no packet is sent.
fn local_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| {
            s.connect("8.8.8.8:80")?;
            s.local_addr()
        })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "OFFLINE".into())
}

/// Everything sysinfo and std can tell us about this machine, grouped into sections.
fn build_intel(sys: &System, nets: &Networks, disks: &Disks, comps: &Components, users: &Users) -> Vec<(String, Vec<(String, String)>)> {
    let kv = |k: &str, v: String| (k.to_string(), v);
    let mut out = Vec::new();

    let boot = chrono::DateTime::from_timestamp(System::boot_time() as i64, 0)
        .map(|d| d.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_default();
    let user = std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default();
    out.push((
        "IDENTITY".to_string(),
        vec![
            kv("HOSTNAME", System::host_name().unwrap_or_default()),
            kv("OPERATOR", user),
            kv("OS", System::name().unwrap_or_default()),
            kv("VERSION", System::long_os_version().unwrap_or_default()),
            kv("KERNEL", System::kernel_version().unwrap_or_default()),
            kv("DISTRIBUTION", System::distribution_id()),
            kv("ARCHITECTURE", std::env::consts::ARCH.to_string()),
            kv("FAMILY", std::env::consts::FAMILY.to_string()),
            kv("BOOTED", boot),
            kv("UPTIME", crate::widgets::fmt_duration(System::uptime())),
            kv("USER ACCOUNTS", users.iter().map(|u| u.name().to_string()).collect::<Vec<_>>().join(", ")),
        ],
    ));

    let cpus = sys.cpus();
    let first = cpus.first();
    let load = System::load_average();
    let mut cpu = vec![
        kv("MODEL", first.map(|c| c.brand().trim().to_string()).unwrap_or_default()),
        kv("VENDOR", first.map(|c| c.vendor_id().to_string()).unwrap_or_default()),
        kv("PHYSICAL CORES", sys.physical_core_count().map(|n| n.to_string()).unwrap_or("--".into())),
        kv("LOGICAL CORES", cpus.len().to_string()),
        kv("FREQUENCY", first.map(|c| format!("{} MHz", c.frequency())).unwrap_or_default()),
        kv("TOTAL LOAD", format!("{:.1}%", sys.global_cpu_info().cpu_usage())),
        kv("LOAD AVG 1/5/15", format!("{:.2} / {:.2} / {:.2}", load.one, load.five, load.fifteen)),
    ];
    for (i, group) in cpus.chunks(4).enumerate() {
        let line = group.iter().map(|c| format!("{:>3.0}%", c.cpu_usage())).collect::<Vec<_>>().join(" ");
        cpu.push(kv(&format!("CORES {}-{}", i * 4, i * 4 + group.len() - 1), line));
    }
    out.push(("PROCESSOR".to_string(), cpu));

    use crate::widgets::fmt_bytes;
    out.push((
        "MEMORY".to_string(),
        vec![
            kv("TOTAL", fmt_bytes(sys.total_memory())),
            kv("USED", format!("{}  ({})", fmt_bytes(sys.used_memory()), pct(sys.used_memory(), sys.total_memory()))),
            kv("AVAILABLE", fmt_bytes(sys.available_memory())),
            kv("FREE", fmt_bytes(sys.free_memory())),
            kv("SWAP TOTAL", fmt_bytes(sys.total_swap())),
            kv("SWAP USED", format!("{}  ({})", fmt_bytes(sys.used_swap()), pct(sys.used_swap(), sys.total_swap()))),
        ],
    ));

    let mut storage = Vec::new();
    for d in disks.iter() {
        let used = d.total_space().saturating_sub(d.available_space());
        storage.push(kv(
            &d.mount_point().display().to_string(),
            format!("{} / {}  ({})", fmt_bytes(used), fmt_bytes(d.total_space()), pct(used, d.total_space())),
        ));
        storage.push(kv(
            "  TYPE",
            format!(
                "{} · {:?}{}",
                d.file_system().to_string_lossy(),
                d.kind(),
                if d.is_removable() { " · REMOVABLE" } else { "" }
            ),
        ));
    }
    out.push(("STORAGE".to_string(), storage));

    let mut net = vec![kv("LOCAL IP", local_ip())];
    let mut ifaces: Vec<_> = nets.iter().collect();
    ifaces.sort_by_key(|(_, d)| std::cmp::Reverse(d.total_received() + d.total_transmitted()));
    for (name, d) in ifaces {
        net.push(kv(&crate::widgets::truncate(name, 18), d.mac_address().to_string()));
        net.push(kv(
            "  TRAFFIC",
            format!("↓ {}  ↑ {}", fmt_bytes(d.total_received()), fmt_bytes(d.total_transmitted())),
        ));
        net.push(kv(
            "  PACKETS",
            format!("↓ {}  ↑ {}", d.total_packets_received(), d.total_packets_transmitted()),
        ));
    }
    out.push(("NETWORK".to_string(), net));

    let mut thermal: Vec<(String, String)> = comps
        .iter()
        .map(|c| (crate::widgets::truncate(&c.label().to_uppercase(), 20), format!("{:.0}°C  (max {:.0}°C)", c.temperature(), c.max())))
        .collect();
    if thermal.is_empty() {
        thermal.push(kv("SENSORS", "NOT EXPOSED BY THIS OS".into()));
    }
    out.push(("THERMAL".to_string(), thermal));

    let mut by_mem: Vec<_> = sys.processes().values().collect();
    by_mem.sort_by_key(|p| std::cmp::Reverse(p.memory()));
    let mut procs = vec![
        kv("RUNNING", sys.processes().len().to_string()),
        kv("DAEMON PID", std::process::id().to_string()),
    ];
    for p in by_mem.iter().take(8) {
        procs.push(kv(&crate::widgets::truncate(&p.name().to_uppercase(), 18), format!("{}  pid {}", fmt_bytes(p.memory()), p.pid())));
    }
    out.push(("MEMORY HOGS".to_string(), procs));

    out
}

/// Internet reachability, measured by a separate probe thread.
#[derive(Clone, Default)]
pub struct NetStatus {
    /// `None` until the first probe finishes.
    pub online: Option<bool>,
    /// Round-trip time of the last successful TCP handshake.
    pub latency_ms: Option<u32>,
    pub latency_history: VecDeque<f32>,
    /// When the connection last went up or down.
    pub since: Option<chrono::DateTime<chrono::Local>>,
}

pub type SharedNet = Arc<Mutex<NetStatus>>;

/// Hosts tried in order; a TCP handshake is enough, no data is sent.
const PROBE_HOSTS: [&str; 2] = ["1.1.1.1:443", "8.8.8.8:443"];

/// Latency of the first host that accepts a TCP connection, or `None` if all fail.
fn probe_internet() -> Option<u32> {
    PROBE_HOSTS.iter().find_map(|host| {
        let addr: std::net::SocketAddr = host.parse().ok()?;
        let start = Instant::now();
        std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
            .ok()
            .map(|_| start.elapsed().as_millis().min(u32::MAX as u128) as u32)
    })
}

/// Checks internet connectivity every 5 seconds on its own thread.
pub fn spawn_net_probe(ctx: egui::Context) -> SharedNet {
    let shared: SharedNet = Arc::new(Mutex::new(NetStatus::default()));
    let out = shared.clone();
    thread::spawn(move || loop {
        let latency = probe_internet();
        {
            let mut s = out.lock().unwrap();
            let online = latency.is_some();
            if s.online != Some(online) {
                s.since = Some(chrono::Local::now());
            }
            s.online = Some(online);
            s.latency_ms = latency;
            push(&mut s.latency_history, latency.map_or(0.0, |l| l as f32));
        }
        ctx.request_repaint();
        thread::sleep(Duration::from_secs(5));
    });
    shared
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pct_handles_zero_total() {
        assert_eq!(pct(5, 0), "--");
        assert_eq!(pct(1, 4), "25.0%");
    }

    #[test]
    fn history_is_capped() {
        let mut h = VecDeque::new();
        for i in 0..HISTORY + 10 {
            push(&mut h, i as f32);
        }
        assert_eq!(h.len(), HISTORY);
        assert_eq!(*h.back().unwrap(), (HISTORY + 9) as f32);
    }

    #[test]
    fn loopback_names() {
        assert!(is_loopback("lo"));
        assert!(is_loopback("Loopback Pseudo-Interface 1"));
        assert!(!is_loopback("Wi-Fi"));
    }

    #[test]
    fn probe_hosts_parse() {
        for h in PROBE_HOSTS {
            assert!(h.parse::<std::net::SocketAddr>().is_ok(), "{h}");
        }
    }
}
