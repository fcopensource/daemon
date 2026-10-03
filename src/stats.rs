//! Background thread that samples system statistics once per second.

use std::cmp::Ordering;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use eframe::egui;
use sysinfo::{Disks, Networks, System};

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

            *out.lock().unwrap() = snap.clone();
            ctx.request_repaint();
            tick += 1;
        }
    });

    shared
}
