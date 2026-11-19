//! `quota-tray --bench-popup <out.txt> [--tab NAME]`
//!
//! Opens the real Slint popup in this process (no tray icon, no network)
//! from inside the Slint event loop and writes:
//!
//! * `popup_open_ms`: "click" until the first frame is rendered (cold).
//! * `reopen_ms`: the same for a second open (warm).
//! * `*_private_ws`: Task Manager's "Memory (active private working set)",
//!   measured by counting this process's unshared resident pages.
//! * `idle_cpu_ms_30s`: CPU time used during 30 s with the popup closed.
//!
//! `quota-tray --slint-probe <out.txt>`: the Slint runtime on its own.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::ProcessStatus::{
    K32GetProcessMemoryInfo, K32QueryWorkingSet, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

#[derive(Clone, Copy)]
struct Mem {
    ws: usize,
    commit: usize,
    private_ws: usize,
}

fn memory() -> Mem {
    unsafe {
        let mut c: PROCESS_MEMORY_COUNTERS_EX = std::mem::zeroed();
        c.cb = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
        let ok = K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c as *mut _ as *mut PROCESS_MEMORY_COUNTERS, c.cb);
        let (ws, commit) = if ok == 0 { (0, 0) } else { (c.WorkingSetSize, c.PrivateUsage) };
        Mem { ws, commit, private_ws: private_ws() }
    }
}

/// Resident pages not shared with any other process, in bytes.
fn private_ws() -> usize {
    const SHARED: usize = 0x100; // PSAPI_WORKING_SET_BLOCK.Shared
    let mut buf: Vec<usize> = vec![0; 16 * 1024];
    unsafe {
        for _ in 0..4 {
            let bytes = (buf.len() * std::mem::size_of::<usize>()) as u32;
            if K32QueryWorkingSet(GetCurrentProcess(), buf.as_mut_ptr() as *mut _, bytes) != 0 {
                let n = buf[0].min(buf.len() - 1);
                return buf[1..=n].iter().filter(|f| *f & SHARED == 0).count() * 4096;
            }
            let need = buf[0].max(buf.len()) + 1024;
            buf = vec![0; need + 1];
        }
    }
    0
}

/// Total CPU time (user + kernel) of this process, in milliseconds.
fn cpu_ms() -> f64 {
    unsafe {
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        if GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u) == 0 {
            return 0.0;
        }
        let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32 | f.dwLowDateTime as u64) as f64 / 10_000.0;
        t(k) + t(u)
    }
}

fn mb(b: usize) -> String {
    format!("{:.2} MB", b as f64 / 1_048_576.0)
}

type Log = Rc<RefCell<Vec<String>>>;

fn after(ms: u64, f: impl FnOnce() + 'static) {
    slint::Timer::single_shot(Duration::from_millis(ms), f);
}

/// Opens the popup, waits for its first frame (polling a frame counter on
/// a 1 ms timer), then calls `done(elapsed_ms)`.
fn open_and_time(done: impl FnOnce(f64) + 'static) {
    let frames = crate::popup::frame_count();
    let t = Instant::now();
    crate::popup::open_now();
    let done = Rc::new(RefCell::new(Some(done)));
    let timer = Rc::new(slint::Timer::default());
    let tm = timer.clone();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(1), move || {
        if crate::popup::frame_count() > frames || t.elapsed() > Duration::from_secs(3) {
            tm.stop();
            if let Some(d) = done.borrow_mut().take() {
                d(t.elapsed().as_secs_f64() * 1000.0);
            }
        }
    });
}

pub fn run(out: &std::path::Path, tab: Option<&str>) {
    crate::with_app(|a| a.ensure_timer());
    crate::popup::set_bench(true);
    if let Some(t) = tab {
        crate::popup::set_start_tab(t);
    }
    let out = out.to_path_buf();
    let log: Log = Rc::new(RefCell::new(Vec::new()));
    after(1000, move || {
        crate::trim_working_set();
        let before = memory();
        log.borrow_mut().push(format!("before_private_ws={}", mb(before.private_ws)));
        let l = log.clone();
        open_and_time(move |open_ms| {
            l.borrow_mut().push(format!("popup_open_ms={open_ms:.1}"));
            let l = l.clone();
            after(400, move || {
                let m = memory();
                l.borrow_mut().push(format!("open_private_ws={}", mb(m.private_ws)));
                l.borrow_mut().push(format!("open_ws={}", mb(m.ws)));
                crate::popup::close();
                let l = l.clone();
                after(500, move || {
                    let l2 = l.clone();
                    open_and_time(move |re_ms| {
                        l2.borrow_mut().push(format!("reopen_ms={re_ms:.1}"));
                        crate::popup::close();
                        let l = l2.clone();
                        after(800, move || {
                            crate::trim_working_set();
                            let m = memory();
                            l.borrow_mut().push(format!("closed_private_ws={}", mb(m.private_ws)));
                            l.borrow_mut().push(format!("closed_commit={}", mb(m.commit)));
                            let cpu0 = cpu_ms();
                            let l = l.clone();
                            after(30_000, move || {
                                l.borrow_mut().push(format!("idle_cpu_ms_30s={:.1}", cpu_ms() - cpu0));
                                let _ = std::fs::write(&out, l.borrow().join("\n") + "\n");
                                let _ = slint::quit_event_loop();
                            });
                        });
                    });
                });
            });
        });
    });
    let _ = slint::run_event_loop_until_quit();
}

/// The Slint runtime with no window, then one popup open and closed.
pub fn slint_probe(out: &std::path::Path) {
    crate::trim_working_set();
    std::thread::sleep(Duration::from_millis(300));
    let base = memory();
    let log: Log = Rc::new(RefCell::new(vec![format!("process_only_private_ws={}", mb(base.private_ws))]));
    let out = out.to_path_buf();
    crate::popup::set_bench(true);
    after(1500, move || {
        crate::trim_working_set();
        log.borrow_mut().push(format!("event_loop_no_window_private_ws={}", mb(memory().private_ws)));
        crate::popup::open_now();
        let l = log.clone();
        after(1500, move || {
            l.borrow_mut().push(format!("window_open_private_ws={}", mb(memory().private_ws)));
            crate::popup::close();
            let l = l.clone();
            after(1500, move || {
                crate::trim_working_set();
                l.borrow_mut().push(format!("after_close_private_ws={}", mb(memory().private_ws)));
                let _ = std::fs::write(&out, l.borrow().join("\n") + "\n");
                let _ = slint::quit_event_loop();
            });
        });
    });
    let _ = slint::run_event_loop_until_quit();
}
