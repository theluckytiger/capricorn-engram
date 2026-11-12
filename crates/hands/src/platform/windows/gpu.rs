//! What each process is doing on the GPU, from the same performance counters Task Manager reads
//! (`\GPU Engine(*)\Utilization Percentage`), and whether the window in front covers its monitor.

use std::collections::HashMap;

use anyhow::{Result, bail};
use windows::Win32::Foundation::{HWND, RECT};
use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow};
use windows::Win32::System::Performance::{
    PDH_FMT_COUNTERVALUE_ITEM_W, PDH_FMT_DOUBLE, PDH_HCOUNTER, PDH_HQUERY, PDH_MORE_DATA, PdhAddEnglishCounterW, PdhCloseQuery, PdhCollectQueryData, PdhGetFormattedCounterArrayW, PdhOpenQueryW,
};
use windows::Win32::UI::Shell::{QUNS_BUSY, QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowRect};
use windows::core::w;

/// A PDH query for 3D engine use per process. Each `sample()` reports use since the previous one.
pub struct GpuMeter {
    query: PDH_HQUERY,
    counter: PDH_HCOUNTER,
}

// SAFETY: the query handle is only used from the thread that owns the meter, one call at a time.
unsafe impl Send for GpuMeter {}

impl GpuMeter {
    pub fn new() -> Result<Self> {
        unsafe {
            let mut query = PDH_HQUERY::default();
            let r = PdhOpenQueryW(None, 0, &mut query);
            if r != 0 {
                bail!("PdhOpenQuery failed: {r:#x}");
            }
            let mut counter = PDH_HCOUNTER::default();
            let r = PdhAddEnglishCounterW(query, w!("\\GPU Engine(*)\\Utilization Percentage"), 0, &mut counter);
            if r != 0 {
                let _ = PdhCloseQuery(query);
                bail!("no GPU counters on this PC ({r:#x})");
            }
            // The first collection only sets the baseline.
            let _ = PdhCollectQueryData(query);
            Ok(Self { query, counter })
        }
    }

    /// Percent of the GPU's 3D engines each process used since the last call, by process id.
    pub fn sample(&mut self) -> Result<HashMap<u32, f64>> {
        unsafe {
            let r = PdhCollectQueryData(self.query);
            if r != 0 {
                bail!("PdhCollectQueryData failed: {r:#x}");
            }
            let mut size = 0u32;
            let mut count = 0u32;
            let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, None);
            if r != PDH_MORE_DATA && r != 0 {
                bail!("PdhGetFormattedCounterArray failed: {r:#x}");
            }
            let mut buf = vec![0u8; size as usize];
            let items = buf.as_mut_ptr() as *mut PDH_FMT_COUNTERVALUE_ITEM_W;
            let r = PdhGetFormattedCounterArrayW(self.counter, PDH_FMT_DOUBLE, &mut size, &mut count, Some(items));
            if r != 0 {
                bail!("PdhGetFormattedCounterArray failed: {r:#x}");
            }
            let mut by_pid: HashMap<u32, f64> = HashMap::new();
            for i in 0..count as usize {
                let item = &*items.add(i);
                let name = item.szName.to_string().unwrap_or_default();
                // "pid_1234_luid_0x0000_0x0000C3B1_phys_0_eng_0_engtype_3D"
                if !name.ends_with("engtype_3D") {
                    continue;
                }
                let Some(pid) = name.strip_prefix("pid_").and_then(|r| r.split('_').next()).and_then(|p| p.parse::<u32>().ok()) else { continue };
                *by_pid.entry(pid).or_default() += item.FmtValue.Anonymous.doubleValue;
            }
            Ok(by_pid)
        }
    }
}

impl Drop for GpuMeter {
    fn drop(&mut self) {
        unsafe {
            let _ = PdhCloseQuery(self.query);
        }
    }
}

/// Whether the window in front fills its monitor (borderless or exclusive fullscreen), and Windows' own opinion:
/// it reports "busy" or "Direct3D fullscreen" while a fullscreen app runs.
pub fn front_fullscreen() -> bool {
    unsafe {
        if let Ok(state) = SHQueryUserNotificationState()
            && (state == QUNS_RUNNING_D3D_FULL_SCREEN || state == QUNS_BUSY)
        {
            return true;
        }
        let hwnd: HWND = GetForegroundWindow();
        if hwnd.is_invalid() {
            return false;
        }
        let mut r = RECT::default();
        if GetWindowRect(hwnd, &mut r).is_err() {
            return false;
        }
        let mon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO { cbSize: std::mem::size_of::<MONITORINFO>() as u32, ..Default::default() };
        if !GetMonitorInfoW(mon, &mut info).as_bool() {
            return false;
        }
        let m = info.rcMonitor;
        r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
    }
}
