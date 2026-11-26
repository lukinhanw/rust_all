//! The primary monitor's real scale. A process takes its scale once, when it starts: the bar and the desktop started while
//! a 125% laptop screen was primary kept that size after a 100% monitor took over, and Windows stretched them (blurry
//! bar, oversized desktop menu and icons). They compare against this and start again when it changes.

use windows_sys::Win32::Foundation::POINT;
use windows_sys::Win32::Graphics::Gdi::{MONITOR_DEFAULTTOPRIMARY, MonitorFromPoint};
use windows_sys::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForMonitor, MDT_EFFECTIVE_DPI, SetThreadDpiAwarenessContext};

/// DPI of the primary monitor as set in Settings (96 = 100%), whatever this process was told when it started.
pub fn primary() -> u32 {
    unsafe {
        // a system-aware process is told its start-up DPI: ask as a per-monitor thread for the real one
        let prev = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let (mut x, mut y) = (96u32, 96u32);
        let ok = GetDpiForMonitor(MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY), MDT_EFFECTIVE_DPI, &mut x, &mut y) == 0;
        if !prev.is_null() {
            SetThreadDpiAwarenessContext(prev);
        }
        if ok && x > 0 { x } else { 96 }
    }
}

/// Starts this exe again with the same arguments; the caller exits right after. Every size is then set from the new scale.
pub fn relaunch() {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).args(std::env::args_os().skip(1)).spawn();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn primary_dpi_is_a_real_scale() {
        let d = super::primary();
        assert!((96..=96 * 5).contains(&d), "{d}");
    }
}
