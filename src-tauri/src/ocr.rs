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

/// Hard cap on the larger dimension of any image handed to Windows.Media.Ocr.
///
/// `OcrEngine::MaxImageDimension` is supposed to be the engine's reliable
/// upper bound, but empirically it under-reports failure: a 4K HoTS draft
/// screenshot consistently loses entire portrait rows in the left half of
/// the image even though it falls under the engine's reported maximum,
/// while the same draft at ~2000 px wide returns every text region cleanly.
/// We cap below the engine's stated max to stay in the regime that works.
const OCR_TARGET_MAX_DIM: u32 = 2000;

/// Factor by which an `width`x`height` image must shrink so neither dimension
/// exceeds `max_dim`. Returns `1.0` when the image already fits.
fn ocr_scale(width: u32, height: u32, max_dim: u32) -> f64 {
    let larger = width.max(height);
    if larger <= max_dim {
        1.0
    } else {
        max_dim as f64 / larger as f64
    }
}

/// Run OCR over an image, returning one entry per recognized text line.
#[cfg(windows)]
pub fn recognize_lines(img: &image::RgbaImage) -> Result<Vec<OcrLine>, String> {
    let engine = OcrEngine::TryCreateFromUserProfileLanguages()
        .map_err(|e| e.to_string())?;
    // MaxImageDimension is a static property on OcrEngine, not an instance one.
    let engine_max = OcrEngine::MaxImageDimension().map_err(|e| e.to_string())?;
    let effective_max = engine_max.min(OCR_TARGET_MAX_DIM);
    log::info!(
        "OCR: input {}x{} (engine max_dim={}, effective max={})",
        img.width(),
        img.height(),
        engine_max,
        effective_max
    );

    let scale = ocr_scale(img.width(), img.height(), effective_max);
    let resized = if scale < 1.0 {
        let new_w = (img.width() as f64 * scale).round().max(1.0) as u32;
        let new_h = (img.height() as f64 * scale).round().max(1.0) as u32;
        log::info!("OCR: resizing input to {}x{}", new_w, new_h);
        // Triangle (bilinear) is ~5-10x faster than Lanczos3 in debug builds
        // and preserves text edges well enough for OCR; quality difference
        // matters only at the pixel-detail level we don't need.
        Some(image::imageops::resize(
            img,
            new_w,
            new_h,
            image::imageops::FilterType::Triangle,
        ))
    } else {
        None
    };
    let to_ocr: &image::RgbaImage = resized.as_ref().unwrap_or(img);
    let (w, h) = (to_ocr.width(), to_ocr.height());

    let writer = DataWriter::new().map_err(|e| e.to_string())?;
    let mut bgra = Vec::with_capacity((w * h * 4) as usize);
    for px in to_ocr.pixels() {
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

    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| e.to_string())?
        .get()
        .map_err(|e| e.to_string())?;

    // Rects come back in resized-image space; scale them back so downstream
    // code keeps working against the caller's original coordinate system.
    let inv_scale = 1.0 / scale;
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
                rect: Rect {
                    x: x0 * inv_scale,
                    y: y0 * inv_scale,
                    width: (x1 - x0) * inv_scale,
                    height: (y1 - y0) * inv_scale,
                },
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ocr_scale_returns_one_when_image_fits() {
        assert_eq!(ocr_scale(1000, 1000, 2600), 1.0);
        assert_eq!(ocr_scale(2600, 2600, 2600), 1.0);
        // Smaller than max on both axes.
        assert_eq!(ocr_scale(1999, 1124, 2600), 1.0);
    }

    #[test]
    fn ocr_scale_shrinks_to_fit_when_larger_axis_exceeds_max() {
        // 4K wide on a 2600 budget: scale by 2600/3840 on the larger axis.
        let s = ocr_scale(3840, 2160, 2600);
        assert!((s - (2600.0 / 3840.0)).abs() < 1e-9, "got {s}");

        // Same image rotated: height now the larger axis, same scale.
        let s = ocr_scale(2160, 3840, 2600);
        assert!((s - (2600.0 / 3840.0)).abs() < 1e-9, "got {s}");
    }

    #[test]
    fn ocr_scale_handles_just_over_threshold() {
        // One pixel over: scale is just under 1.0.
        let s = ocr_scale(2601, 2000, 2600);
        assert!(s < 1.0);
        assert!((s - (2600.0 / 2601.0)).abs() < 1e-9, "got {s}");
    }
}

