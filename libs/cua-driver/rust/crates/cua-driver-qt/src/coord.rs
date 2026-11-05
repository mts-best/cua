use crate::model::Rect;

/// DPI scale of a top-level window. UIA bounding rectangles are physical
/// screen pixels; the normalized model keeps window-local logical pixels.
#[cfg(target_os = "windows")]
pub fn window_scale(hwnd: u64) -> f64 {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::HiDpi::GetDpiForWindow;

    let dpi = unsafe { GetDpiForWindow(HWND(hwnd as *mut _)) };
    if dpi == 0 {
        1.0
    } else {
        f64::from(dpi) / 96.0
    }
}

#[cfg(not(target_os = "windows"))]
pub fn window_scale(_hwnd: u64) -> f64 {
    1.0
}

/// Physical screen rect (left, top, right, bottom) → window-local logical rect.
pub fn screen_rect_to_window_local(
    rect: (i32, i32, i32, i32),
    window_x: i32,
    window_y: i32,
    scale: f64,
) -> Rect {
    let (left, top, right, bottom) = rect;
    Rect {
        x: f64::from(left - window_x) / scale,
        y: f64::from(top - window_y) / scale,
        w: f64::from(right - left) / scale,
        h: f64::from(bottom - top) / scale,
    }
}

/// The window-local logical box, used by the out-of-bounds invariant.
pub fn window_local_bounds(width: i32, height: i32, scale: f64) -> Rect {
    Rect {
        x: 0.0,
        y: 0.0,
        w: f64::from(width) / scale,
        h: f64::from(height) / scale,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screen_rect_is_translated_then_scaled() {
        // 150% display: screen (300, 450)-(396, 498) in a window whose
        // physical origin is (150, 300).
        let rect = screen_rect_to_window_local((300, 450, 396, 498), 150, 300, 1.5);
        assert_eq!(
            rect,
            Rect {
                x: 100.0,
                y: 100.0,
                w: 64.0,
                h: 32.0
            }
        );
    }

    #[test]
    fn window_bounds_are_logical() {
        let bounds = window_local_bounds(1200, 900, 1.5);
        assert_eq!(bounds.w, 800.0);
        assert_eq!(bounds.h, 600.0);
    }
}
