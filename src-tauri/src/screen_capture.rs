//! Primary-monitor screen capture (Windows GDI).

use image::RgbaImage;
use windows::Win32::Graphics::Gdi::{
    BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject, GetDIBits, GetDC,
    ReleaseDC, SelectObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS, SRCCOPY,
};
use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};

/// Capture the primary monitor as an RGBA image (screen-pixel dimensions).
pub fn capture_primary_monitor() -> Result<RgbaImage, String> {
    unsafe {
        let width = GetSystemMetrics(SM_CXSCREEN);
        let height = GetSystemMetrics(SM_CYSCREEN);
        if width <= 0 || height <= 0 {
            return Err("could not read screen dimensions".into());
        }

        let screen_dc = GetDC(None);
        if screen_dc.is_invalid() {
            return Err("GetDC failed".into());
        }
        let mem_dc = CreateCompatibleDC(Some(screen_dc));
        let bitmap = CreateCompatibleBitmap(screen_dc, width, height);
        let old = SelectObject(mem_dc, bitmap.into());

        let blt = BitBlt(mem_dc, 0, 0, width, height, Some(screen_dc), 0, 0, SRCCOPY);

        let mut buf = vec![0u8; (width * height * 4) as usize];
        let mut info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height, // negative => top-down rows
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB.0,
                ..Default::default()
            },
            ..Default::default()
        };
        let scanlines = GetDIBits(
            mem_dc,
            bitmap,
            0,
            height as u32,
            Some(buf.as_mut_ptr() as *mut _),
            &mut info,
            DIB_RGB_COLORS,
        );

        SelectObject(mem_dc, old);
        let _ = DeleteObject(bitmap.into());
        let _ = DeleteDC(mem_dc);
        ReleaseDC(None, screen_dc);

        if blt.is_err() || scanlines == 0 {
            return Err("BitBlt/GetDIBits failed".into());
        }

        // GDI gives BGRA; swap B and R for RGBA.
        for px in buf.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        RgbaImage::from_raw(width as u32, height as u32, buf)
            .ok_or_else(|| "image buffer size mismatch".into())
    }
}

/// DEV-ONLY: capture the screen and save it to the temp dir for inspection.
#[tauri::command]
pub fn dev_capture_screen() -> Result<String, String> {
    let img = capture_primary_monitor()?;
    let path = std::env::temp_dir().join("storm-almanac-capture.png");
    img.save(&path).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}
