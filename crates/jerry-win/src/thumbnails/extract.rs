//! The Shell call behind one thumbnail, and its bitmap as a PNG. Runs on a Shell pool worker
//! (STA, with a timeout), never on the main thread or the tokio runtime.

use jerry_core::{ErrorCode, JerryError};
use windows::Win32::Foundation::SIZE;
use windows::Win32::Graphics::Gdi::{DeleteObject, HBITMAP, HPALETTE};
use windows::Win32::Graphics::Imaging::{
    GUID_WICPixelFormat32bppBGRA, GUID_WICPixelFormat32bppPBGRA, WICBitmapDitherTypeNone,
    WICBitmapPaletteTypeCustom, WICBitmapUsePremultipliedAlpha,
};
use windows::Win32::UI::Shell::{
    IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_INCACHEONLY, SIIGBF_RESIZETOFIT,
    SIIGBF_THUMBNAILONLY,
};
use windows::core::{Interface, PCWSTR};

use super::Request;
use crate::wic;

/// The thumbnail `request` asks for, fitted into its box with its aspect ratio kept, as a PNG.
/// `NotFound` if the Shell has none: the type has no thumbnail handler, or the file is
/// cloud-only and Windows has none cached.
pub(super) fn thumbnail(request: &Request) -> Result<Vec<u8>, JerryError> {
    // SAFETY: a NUL-terminated path; COM is initialized on this thread (a Shell pool worker).
    let item: IShellItemImageFactory =
        unsafe { SHCreateItemFromParsingName(PCWSTR(request.path.as_ptr()), None) }?;
    let mut flags = SIIGBF_RESIZETOFIT | SIIGBF_THUMBNAILONLY;
    if request.cache_only {
        flags |= SIIGBF_INCACHEONLY;
    }
    let side = i32::try_from(request.px).map_err(|_| JerryError::internal())?;
    // SAFETY: a size and flags; the bitmap that comes back is ours and deleted on drop.
    let bitmap = match unsafe { item.GetImage(SIZE { cx: side, cy: side }, flags) } {
        Ok(bitmap) => Bitmap(bitmap),
        Err(error) => {
            tracing::debug!(%error, cache_only = request.cache_only, "the Shell has no thumbnail");
            return Err(JerryError::new(ErrorCode::NotFound));
        }
    };
    png_of(&bitmap)
}

/// The Shell's bitmap as a PNG with straight alpha.
fn png_of(bitmap: &Bitmap) -> Result<Vec<u8>, JerryError> {
    let factory = wic::factory()?;
    // SAFETY: a valid bitmap and no palette; WIC copies its pixels. The Shell's 32-bit bitmaps
    // carry premultiplied alpha, as `AlphaBlend` takes it.
    let source = unsafe {
        factory.CreateBitmapFromHBITMAP(
            bitmap.0,
            HPALETTE::default(),
            WICBitmapUsePremultipliedAlpha,
        )
    }?;
    // SAFETY: a new converter, over the bitmap above; a no-op for a 32-bit one, and an opaque
    // alpha for anything else.
    let converter = unsafe { factory.CreateFormatConverter() }?;
    // SAFETY: as above; no palette is needed for a 32-bit format.
    unsafe {
        converter.Initialize(
            &source,
            &GUID_WICPixelFormat32bppPBGRA,
            WICBitmapDitherTypeNone,
            None,
            0.0,
            WICBitmapPaletteTypeCustom,
        )
    }?;
    let (mut width, mut height) = (0, 0);
    // SAFETY: two writable sizes.
    unsafe { converter.GetSize(&mut width, &mut height) }?;
    let stride = width.checked_mul(4).ok_or_else(JerryError::internal)?;
    let length = (stride as usize)
        .checked_mul(height as usize)
        .ok_or_else(JerryError::internal)?;
    let mut pixels = vec![0_u8; length];
    // SAFETY: the whole bitmap into a buffer of `stride` × `height` bytes.
    unsafe { converter.CopyPixels(std::ptr::null(), stride, &mut pixels) }?;
    straighten(&mut pixels);
    // SAFETY: `pixels` holds `height` rows of `stride` bytes; WIC copies them.
    let straight = unsafe {
        factory.CreateBitmapFromMemory(
            width,
            height,
            &GUID_WICPixelFormat32bppBGRA,
            stride,
            &pixels,
        )
    }?;
    wic::png(&factory, &straight.cast()?, width, height)
}

/// Premultiplied BGRA pixels to straight alpha, in place, as PNG takes them. A bitmap whose
/// alpha is 0 everywhere has none (an opaque thumbnail, such as a photo's, in a 32-bit bitmap):
/// it becomes opaque.
fn straighten(pixels: &mut [u8]) {
    let (pixels, _) = pixels.as_chunks_mut::<4>();
    if pixels.iter().all(|pixel| pixel[3] == 0) {
        for pixel in pixels {
            pixel[3] = u8::MAX;
        }
        return;
    }
    for pixel in pixels {
        let alpha = u32::from(pixel[3]);
        match alpha {
            255 => {}
            0 => pixel[..3].fill(0),
            _ => {
                for channel in &mut pixel[..3] {
                    let straight = (u32::from(*channel) * 255 + alpha / 2) / alpha;
                    *channel = straight.min(255) as u8;
                }
            }
        }
    }
}

/// A bitmap handle, deleted on drop.
struct Bitmap(HBITMAP);

impl Drop for Bitmap {
    fn drop(&mut self) {
        // SAFETY: a bitmap the Shell gave this module, selected into no device context, deleted
        // once.
        let _ = unsafe { DeleteObject(self.0.into()) };
    }
}

#[cfg(test)]
mod tests {
    use super::straighten;

    #[test]
    fn premultiplied_pixels_become_straight() {
        // Opaque, half-transparent red, fully transparent.
        let mut pixels = [10, 20, 30, 255, 0, 0, 128, 128, 5, 5, 5, 0];
        straighten(&mut pixels);
        assert_eq!(pixels, [10, 20, 30, 255, 0, 0, 255, 128, 0, 0, 0, 0]);
    }

    #[test]
    fn a_bitmap_with_no_alpha_becomes_opaque() {
        let mut pixels = [10, 20, 30, 0, 40, 50, 60, 0];
        straighten(&mut pixels);
        assert_eq!(pixels, [10, 20, 30, 255, 40, 50, 60, 255]);
    }
}
