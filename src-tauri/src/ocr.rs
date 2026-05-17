//! OCR via Windows.Media.Ocr.

use crate::draft_types::OcrLine;
#[cfg(windows)]
use crate::draft_types::Rect;
#[cfg(windows)]
use windows::Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap};
#[cfg(windows)]
use windows::Media::Ocr::OcrEngine;
#[cfg(windows)]
use windows::Storage::Streams::DataWriter;

/// Run OCR over an image, returning one entry per recognized text line.
#[cfg(windows)]
pub fn recognize_lines(img: &image::RgbaImage) -> Result<Vec<OcrLine>, String> {
    let (w, h) = (img.width(), img.height());

    let writer = DataWriter::new().map_err(|e| e.to_string())?;
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for px in img.pixels() {
        // RGBA -> BGRA. Alpha is forced opaque: GDI screen capture leaves the
        // alpha byte unset, and OCR needs a non-transparent image.
        bgra.extend_from_slice(&[px[2], px[1], px[0], 255]);
    }
    writer.WriteBytes(&bgra).map_err(|e| e.to_string())?;
    let buffer = writer.DetachBuffer().map_err(|e| e.to_string())?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &buffer,
        BitmapPixelFormat::Bgra8,
        w as i32,
        h as i32,
        BitmapAlphaMode::Straight,
    )
    .map_err(|e| e.to_string())?;

    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| e.to_string())?;
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;

    let mut out = Vec::new();
    for line in result.Lines().map_err(|e| e.to_string())? {
        let text = line.Text().map_err(|e| e.to_string())?.to_string();
        let mut x0 = f64::INFINITY;
        let mut y0 = f64::INFINITY;
        let mut x1 = f64::NEG_INFINITY;
        let mut y1 = f64::NEG_INFINITY;
        for word in line.Words().map_err(|e| e.to_string())? {
            let r = word.BoundingRect().map_err(|e| e.to_string())?;
            x0 = x0.min(r.X as f64);
            y0 = y0.min(r.Y as f64);
            x1 = x1.max((r.X + r.Width) as f64);
            y1 = y1.max((r.Y + r.Height) as f64);
        }
        if x0.is_finite() {
            out.push(OcrLine {
                text,
                rect: Rect { x: x0, y: y0, width: x1 - x0, height: y1 - y0 },
            });
        }
    }
    Ok(out)
}

/// OCR is only supported on Windows.
#[cfg(not(windows))]
pub fn recognize_lines(_img: &image::RgbaImage) -> Result<Vec<OcrLine>, String> {
    Err("OCR is only supported on Windows".into())
}

