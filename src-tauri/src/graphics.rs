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
    use super::valid_proc_address;

    #[test]
    fn rejects_missing_and_wgl_sentinel_entry_points() {
        for address in [0, 1, 2, 3, usize::MAX] {
            assert!(!valid_proc_address(address));
        }
        assert!(valid_proc_address(0x10000));
    }
}
