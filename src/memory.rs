use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, Default)]
pub struct MemorySnapshot {
    pub total_mb: f64,
    pub host_mb: f64,
    pub webview_mb: f64,
    pub private_mb: f64,
    pub process_count: usize,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct TrimReport {
    pub before_mb: f64,
    pub after_mb: f64,
    pub freed_mb: f64,
    pub trimmed_processes: usize,
}

/// Caches the Rustcord + WebView2 process tree PIDs so periodic RAM polling
/// avoids snapshotting every process on Windows every 2.5 seconds.
pub struct ProcessTracker {
    cached_pids: Vec<u32>,
    last_browser_pid: Option<u32>,
    last_scan: Instant,
}

impl ProcessTracker {
    pub fn new() -> Self {
        Self {
            cached_pids: Vec::new(),
            last_browser_pid: None,
            last_scan: Instant::now() - Duration::from_secs(60),
        }
    }

    pub fn invalidate(&mut self) {
        self.cached_pids.clear();
        self.last_scan = Instant::now() - Duration::from_secs(60);
    }

    fn pids(&mut self, webview_browser_pid: Option<u32>, force_rescan: bool) -> &[u32] {
        let stale = force_rescan
            || self.cached_pids.is_empty()
            || self.last_browser_pid != webview_browser_pid
            || self.last_scan.elapsed() >= Duration::from_secs(20);

        if stale {
            self.cached_pids = collect_rustcord_pids(webview_browser_pid);
            self.last_browser_pid = webview_browser_pid;
            self.last_scan = Instant::now();
        }
        &self.cached_pids
    }

    pub fn measure(&mut self, webview_browser_pid: Option<u32>) -> MemorySnapshot {
        let pids = self.pids(webview_browser_pid, false).to_vec();
        measure_pids(&pids)
    }

    pub fn trim(&mut self, webview_browser_pid: Option<u32>) -> TrimReport {
        let pids = self.pids(webview_browser_pid, true).to_vec();
        trim_pids(&pids)
    }

    pub fn set_low_priority(&mut self, webview_browser_pid: Option<u32>, low_priority: bool) {
        let pids = self.pids(webview_browser_pid, true).to_vec();
        set_pids_priority(&pids, low_priority);
    }
}

#[cfg(windows)]
mod win_impl {
    use super::*;
    use windows::Win32::Foundation::{
        CloseHandle, COLORREF, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, WAIT_OBJECT_0,
    };
    use windows::Win32::Graphics::Dwm::{
        DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_USE_IMMERSIVE_DARK_MODE,
        DwmSetWindowAttribute,
    };
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::ProcessStatus::{
        K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows::Win32::System::Threading::{
        BELOW_NORMAL_PRIORITY_CLASS, CreateEventW, CreateMutexW, EVENT_MODIFY_STATE,
        GetCurrentProcessId, INFINITE, NORMAL_PRIORITY_CLASS, OpenEventW, OpenProcess,
        PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION,
        PROCESS_SET_QUOTA, PROCESS_VM_READ, SetEvent, SetPriorityClass, WaitForSingleObject,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        GWL_EXSTYLE, GetWindowLongW, LWA_ALPHA, SW_RESTORE, SetForegroundWindow,
        SetLayeredWindowAttributes, SetWindowLongW, ShowWindow, WS_EX_LAYERED,
    };
    use windows_core::{BOOL, w};

    pub struct SingleInstanceGuard {
        mutex: HANDLE,
        wakeup_event: HANDLE,
    }

    impl Drop for SingleInstanceGuard {
        fn drop(&mut self) {
            unsafe {
                if !self.wakeup_event.is_invalid() {
                    let _ = CloseHandle(self.wakeup_event);
                }
                if !self.mutex.is_invalid() {
                    let _ = CloseHandle(self.mutex);
                }
            }
        }
    }

    /// Acquires the single-instance lock. If Rustcord is already running,
    /// signals the existing instance to restore/focus its window and returns None.
    pub fn acquire_single_instance() -> Option<SingleInstanceGuard> {
        unsafe {
            let mutex = CreateMutexW(None, false, w!("Local\\RustcordSingleInstanceMutex")).ok()?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(mutex);
                // Wake up the already-running Rustcord instance (even if hidden in tray!)
                if let Ok(ev) = OpenEventW(EVENT_MODIFY_STATE, false, w!("Local\\RustcordWakeupEvent")) {
                    let _ = SetEvent(ev);
                    let _ = CloseHandle(ev);
                }
                return None;
            }
            let wakeup_event =
                CreateEventW(None, false, false, w!("Local\\RustcordWakeupEvent"))
                    .unwrap_or(HANDLE::default());
            Some(SingleInstanceGuard {
                mutex,
                wakeup_event,
            })
        }
    }

    /// Spawns a zero-CPU kernel-blocking listener thread that triggers `on_wake`
    /// whenever the user launches `Rustcord.exe` a second time.
    pub fn start_wakeup_listener<F>(guard: &SingleInstanceGuard, mut on_wake: F)
    where
        F: FnMut() + Send + 'static,
    {
        if guard.wakeup_event.is_invalid() {
            return;
        }
        let raw_handle = guard.wakeup_event.0 as usize;
        std::thread::spawn(move || loop {
            let h = HANDLE(raw_handle as *mut _);
            let res = unsafe { WaitForSingleObject(h, INFINITE) };
            if res == WAIT_OBJECT_0 {
                on_wake();
            } else {
                break;
            }
        });
    }

    /// Collects Rustcord's own PID plus all descendants of Rustcord AND all descendants
    /// of the active WebView2 browser process PID.
    pub fn collect_rustcord_pids(webview_browser_pid: Option<u32>) -> Vec<u32> {
        let self_pid = unsafe { GetCurrentProcessId() };
        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();

        unsafe {
            let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else {
                let mut v = vec![self_pid];
                if let Some(b) = webview_browser_pid {
                    if b != self_pid {
                        v.push(b);
                    }
                }
                return v;
            };

            let mut entry = PROCESSENTRY32W {
                dwSize: size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    children_map
                        .entry(entry.th32ParentProcessID)
                        .or_default()
                        .push(entry.th32ProcessID);
                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }
            let _ = CloseHandle(snapshot);
        }

        let mut visited: HashSet<u32> = HashSet::new();
        let mut stack = vec![self_pid];
        if let Some(b) = webview_browser_pid {
            if b != 0 {
                stack.push(b);
            }
        }

        while let Some(pid) = stack.pop() {
            if visited.insert(pid) {
                if let Some(kids) = children_map.get(&pid) {
                    for &kid in kids {
                        if !visited.contains(&kid) {
                            stack.push(kid);
                        }
                    }
                }
            }
        }

        visited.into_iter().collect()
    }

    fn query_pid_memory(pid: u32) -> Option<(u64, u64)> {
        unsafe {
            let handle = OpenProcess(
                PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ,
                false,
                pid,
            )
            .ok()?;

            let mut pmc = PROCESS_MEMORY_COUNTERS_EX2::default();
            pmc.cb = size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32;

            let ok = K32GetProcessMemoryInfo(
                handle,
                &mut pmc as *mut PROCESS_MEMORY_COUNTERS_EX2 as *mut PROCESS_MEMORY_COUNTERS,
                size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            );
            let _ = CloseHandle(handle);

            if ok.as_bool() {
                let resident_private = if pmc.PrivateWorkingSetSize > 0 {
                    pmc.PrivateWorkingSetSize as u64
                } else {
                    pmc.WorkingSetSize as u64
                };
                Some((resident_private, pmc.PrivateUsage as u64))
            } else {
                None
            }
        }
    }

    pub fn measure_pids(pids: &[u32]) -> MemorySnapshot {
        let self_pid = unsafe { GetCurrentProcessId() };
        let mut host_ws: u64 = 0;
        let mut webview_ws: u64 = 0;
        let mut total_private: u64 = 0;
        let mut count: usize = 0;

        for &pid in pids {
            if let Some((ws, priv_bytes)) = query_pid_memory(pid) {
                count += 1;
                total_private = total_private.saturating_add(priv_bytes);
                if pid == self_pid {
                    host_ws = ws;
                } else {
                    webview_ws = webview_ws.saturating_add(ws);
                }
            }
        }

        const MB: f64 = 1024.0 * 1024.0;
        let host_mb = (host_ws as f64) / MB;
        let webview_mb = (webview_ws as f64) / MB;
        MemorySnapshot {
            total_mb: host_mb + webview_mb,
            host_mb,
            webview_mb,
            private_mb: (total_private as f64) / MB,
            process_count: count,
        }
    }

    pub fn trim_pids(pids: &[u32]) -> TrimReport {
        let before = measure_pids(pids);
        let mut trimmed = 0usize;

        unsafe {
            for &pid in pids {
                if let Ok(handle) = OpenProcess(
                    PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA | PROCESS_VM_READ,
                    false,
                    pid,
                ) {
                    if K32EmptyWorkingSet(handle).as_bool() {
                        trimmed += 1;
                    }
                    let _ = CloseHandle(handle);
                }
            }
        }

        let after = measure_pids(pids);
        let freed = (before.total_mb - after.total_mb).max(0.0);
        TrimReport {
            before_mb: before.total_mb,
            after_mb: after.total_mb,
            freed_mb: freed,
            trimmed_processes: trimmed,
        }
    }

    pub fn set_pids_priority(pids: &[u32], low_priority: bool) {
        let class = if low_priority {
            BELOW_NORMAL_PRIORITY_CLASS
        } else {
            NORMAL_PRIORITY_CLASS
        };
        unsafe {
            for &pid in pids {
                if let Ok(handle) = OpenProcess(PROCESS_SET_INFORMATION, false, pid) {
                    let _ = SetPriorityClass(handle, class);
                    let _ = CloseHandle(handle);
                }
            }
        }
    }

    pub fn apply_dark_titlebar(hwnd_raw: isize, caption_rgb: (u8, u8, u8)) {
        let hwnd = HWND(hwnd_raw as *mut _);
        let dark = BOOL(1);
        let colorref: u32 = ((caption_rgb.2 as u32) << 16)
            | ((caption_rgb.1 as u32) << 8)
            | (caption_rgb.0 as u32);
        let border_colorref: u32 = 0x002B2624;
        unsafe {
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_USE_IMMERSIVE_DARK_MODE,
                &dark as *const BOOL as *const _,
                size_of::<BOOL>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_CAPTION_COLOR,
                &colorref as *const u32 as *const _,
                size_of::<u32>() as u32,
            );
            let _ = DwmSetWindowAttribute(
                hwnd,
                DWMWA_BORDER_COLOR,
                &border_colorref as *const u32 as *const _,
                size_of::<u32>() as u32,
            );
        }
    }

    pub fn apply_window_opacity(hwnd_raw: isize, opacity_percent: u8) {
        let hwnd = HWND(hwnd_raw as *mut _);
        let clamped = opacity_percent.clamp(45, 100);
        unsafe {
            let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE);
            if clamped >= 100 {
                if (ex_style & (WS_EX_LAYERED.0 as i32)) != 0 {
                    let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 255, LWA_ALPHA);
                    let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_LAYERED.0 as i32));
                }
            } else {
                if (ex_style & (WS_EX_LAYERED.0 as i32)) == 0 {
                    let _ = SetWindowLongW(hwnd, GWL_EXSTYLE, ex_style | (WS_EX_LAYERED.0 as i32));
                }
                let alpha = ((clamped as f32 / 100.0) * 255.0).round() as u8;
                let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), alpha, LWA_ALPHA);
            }
        }
    }

    pub fn focus_window_native(hwnd_raw: isize) {
        let hwnd = HWND(hwnd_raw as *mut _);
        unsafe {
            let _ = ShowWindow(hwnd, SW_RESTORE);
            let _ = SetForegroundWindow(hwnd);
        }
    }
}

#[cfg(windows)]
pub use win_impl::*;

pub fn current_binary_size_mb() -> f64 {
    std::env::current_exe()
        .ok()
        .and_then(|p| std::fs::metadata(p).ok())
        .map(|m| (m.len() as f64) / (1024.0 * 1024.0))
        .unwrap_or(0.0)
}

/// Cleans up leftover `Rustcord.old.exe` created during seamless in-place updates.
pub fn cleanup_old_binary() {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let old = dir.join("Rustcord.old.exe");
            if old.exists() {
                let _ = std::fs::remove_file(old);
            }
        }
    }
}
