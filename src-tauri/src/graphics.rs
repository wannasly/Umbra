//! Select a renderer before Slint creates its event loop. Basic Windows display
//! drivers expose OpenGL 1.1 but cannot run FemtoVG's shaders.

pub fn renderer() -> &'static str {
    if std::env::var("UMBRA_RENDERER").as_deref() == Ok("software") {
        return "software";
    }
    #[cfg(windows)]
    if !supports_gpu_renderer() {
        return "software";
    }
    "femtovg"
}

fn startup_size(work_width: u32, work_height: u32, scale: f64) -> slint::LogicalSize {
    let scale = if scale.is_finite() && scale > 0.0 {
        scale
    } else {
        1.0
    };
    slint::LogicalSize::new(
        (work_width as f64 / scale - 32.0).clamp(480.0, 1240.0) as f32,
        (work_height as f64 / scale - 32.0).clamp(430.0, 800.0) as f32,
    )
}

fn work_area(monitor: &slint::winit_030::winit::monitor::MonitorHandle) -> (i32, i32, u32, u32) {
    #[cfg(windows)]
    {
        use slint::winit_030::winit::platform::windows::MonitorHandleExtWindows;
        use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO};
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(HMONITOR(monitor.hmonitor() as *mut _), &mut info) }.as_bool() {
            let r = info.rcWork;
            return (
                r.left,
                r.top,
                (r.right - r.left) as u32,
                (r.bottom - r.top) as u32,
            );
        }
    }
    let position = monitor.position();
    let size = monitor.size();
    (position.x, position.y, size.width, size.height)
}

pub fn fit_startup_window(window: &slint::Window) {
    use slint::winit_030::{winit::dpi::PhysicalPosition, WinitWindowAccessor};
    let placement = window
        .with_winit_window(|native| {
            let monitor = native
                .current_monitor()
                .or_else(|| native.primary_monitor())?;
            let (x, y, width, height) = work_area(&monitor);
            let scale = native.scale_factor();
            let size = startup_size(width, height, scale);
            let position = PhysicalPosition::new(
                x + ((width as f64 - size.width as f64 * scale) / 2.0).max(0.0) as i32,
                y + ((height as f64 - size.height as f64 * scale) / 2.0).max(0.0) as i32,
            );
            Some((size, position))
        })
        .flatten();
    if let Some((size, position)) = placement {
        window.set_size(size);
        window.with_winit_window(|native| native.set_outer_position(position));
    }
}

/// Winit removes WS_SIZEBOX from undecorated windows. Windows needs that
/// style for the native resize loop, even with a fully custom client area.
pub fn enable_native_resize(window: &slint::Window) {
    use slint::winit_030::WinitWindowAccessor;
    window.with_winit_window(|native| {
        native.set_resizable(true);
        #[cfg(windows)]
        {
            use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
            use windows::Win32::{
                Foundation::HWND,
                UI::WindowsAndMessaging::{
                    GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_STYLE,
                    SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
                    WS_THICKFRAME,
                },
            };
            if let Ok(handle) = native.window_handle() {
                if let RawWindowHandle::Win32(handle) = handle.as_raw() {
                    let hwnd = HWND(handle.hwnd.get() as *mut _);
                    unsafe {
                        let style = GetWindowLongPtrW(hwnd, GWL_STYLE);
                        if style & WS_THICKFRAME.0 as isize == 0 {
                            SetWindowLongPtrW(hwnd, GWL_STYLE, style | WS_THICKFRAME.0 as isize);
                            let _ = SetWindowPos(
                                hwnd,
                                None,
                                0,
                                0,
                                0,
                                0,
                                SWP_FRAMECHANGED
                                    | SWP_NOACTIVATE
                                    | SWP_NOMOVE
                                    | SWP_NOSIZE
                                    | SWP_NOZORDER,
                            );
                        }
                    }
                }
            }
        }
    });
}

#[cfg(windows)]
fn supports_gpu_renderer() -> bool {
    use windows::{
        core::{s, w},
        Win32::{
            Graphics::{
                Gdi::{GetDC, ReleaseDC},
                OpenGL::{
                    wglCreateContext, wglDeleteContext, wglGetCurrentContext, wglGetCurrentDC,
                    wglGetProcAddress, wglMakeCurrent, ChoosePixelFormat, SetPixelFormat,
                    PFD_DOUBLEBUFFER, PFD_DRAW_TO_WINDOW, PFD_SUPPORT_OPENGL, PFD_TYPE_RGBA,
                    PIXELFORMATDESCRIPTOR,
                },
            },
            UI::WindowsAndMessaging::{CreateWindowExW, DestroyWindow, WINDOW_EX_STYLE, WS_POPUP},
        },
    };
    // No visible window or frame is created. The temporary context is needed
    // because wglGetProcAddress is meaningful only with a current context.
    unsafe {
        let Ok(window) = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            w!("STATIC"),
            w!("Umbra renderer probe"),
            WS_POPUP,
            0,
            0,
            1,
            1,
            None,
            None,
            None,
            None,
        ) else {
            return false;
        };
        let dc = GetDC(Some(window));
        let previous_dc = wglGetCurrentDC();
        let previous_context = wglGetCurrentContext();
        let mut context = None;
        let supported = (|| -> windows::core::Result<bool> {
            if dc.is_invalid() {
                return Ok(false);
            }
            let format = PIXELFORMATDESCRIPTOR {
                nSize: std::mem::size_of::<PIXELFORMATDESCRIPTOR>() as u16,
                nVersion: 1,
                dwFlags: PFD_DRAW_TO_WINDOW | PFD_SUPPORT_OPENGL | PFD_DOUBLEBUFFER,
                iPixelType: PFD_TYPE_RGBA,
                cColorBits: 24,
                cAlphaBits: 8,
                ..Default::default()
            };
            let index = ChoosePixelFormat(dc, &format);
            if index == 0 {
                return Ok(false);
            }
            SetPixelFormat(dc, index, &format)?;
            let probe_context = wglCreateContext(dc)?;
            context = Some(probe_context);
            wglMakeCurrent(dc, probe_context)?;
            Ok([
                s!("glCreateShader"),
                s!("glBindFramebuffer"),
                s!("glGenVertexArrays"),
            ]
            .iter()
            .all(|symbol| {
                wglGetProcAddress(*symbol)
                    .map(|function| valid_proc_address(function as usize))
                    .unwrap_or(false)
            }))
        })()
        .unwrap_or(false);
        if let Some(context) = context {
            let _ = wglMakeCurrent(previous_dc, previous_context);
            let _ = wglDeleteContext(context);
        }
        if !dc.is_invalid() {
            ReleaseDC(Some(window), dc);
        }
        let _ = DestroyWindow(window);
        supported
    }
}

#[cfg(any(windows, test))]
fn valid_proc_address(address: usize) -> bool {
    // WGL drivers may return sentinel values in addition to NULL.
    address > 3 && address != usize::MAX
}

#[cfg(test)]
mod tests {
    use super::{startup_size, valid_proc_address};

    #[test]
    fn startup_size_respects_work_area_and_dpi() {
        let large = startup_size(1920, 1040, 1.25);
        assert_eq!((large.width, large.height), (1240.0, 800.0));
        let small = startup_size(1366, 728, 1.25);
        assert!(small.width as f64 * 1.25 < 1366.0);
        assert!(small.height as f64 * 1.25 < 728.0);
        assert!(small.width >= 480.0 && small.height >= 430.0);
    }

    #[test]
    fn rejects_missing_and_wgl_sentinel_entry_points() {
        for address in [0, 1, 2, 3, usize::MAX] {
            assert!(!valid_proc_address(address));
        }
        assert!(valid_proc_address(0x10000));
    }
}
