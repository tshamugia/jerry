//! Renders a system image list icon as a PNG at an exact size in device pixels, with WIC.
//!
//! The image lists come in four sizes (small and large follow the system DPI: 16 and 32 px at
//! 100 %, 24 and 48 at 150 %; extra large is 48, jumbo 256). The smallest list at least as big
//! as the request is used, so at the system DPI the usual sizes need no scaling; any other size
//! is scaled down with high-quality cubic filtering.

use jerry_core::JerryError;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICBitmapSource, IWICImagingFactory, WICBitmapEncoderNoCache,
    WICBitmapInterpolationModeHighQualityCubic,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IStream, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::UI::Controls::{IImageList, ILD_TRANSPARENT};
use windows::Win32::UI::Shell::{
    IDO_SHGIOI_LINK, SHCreateMemStream, SHGetIconOverlayIndexW, SHGetImageList, SHIL_EXTRALARGE,
    SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, HICON};
use windows::core::{Interface, PCWSTR};

use super::shell::Glyph;

/// `glyph` at `px` × `px` as a PNG (32-bit, straight alpha).
pub fn png(glyph: Glyph, px: u32) -> Result<Vec<u8>, JerryError> {
    let (list, size) = image_list_for(px)?;
    let mut flags = ILD_TRANSPARENT.0;
    if glyph.link {
        flags |= link_overlay() << 8; // INDEXTOOVERLAYMASK
    }
    // SAFETY: a system image list and an index from the Shell; the icon is destroyed on drop.
    let icon = Icon(unsafe { list.GetIcon(glyph.index, flags) }?);
    // SAFETY: COM is initialized on this thread (a Shell pool worker).
    let factory: IWICImagingFactory =
        unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?;
    // SAFETY: a valid icon; WIC copies its pixels (alpha, or the mask for old icons).
    let bitmap = unsafe { factory.CreateBitmapFromHICON(icon.0) }?;
    let source: IWICBitmapSource = if size == px {
        bitmap.cast()?
    } else {
        // SAFETY: a new scaler over the bitmap above.
        let scaler = unsafe { factory.CreateBitmapScaler() }?;
        // SAFETY: as above.
        unsafe { scaler.Initialize(&bitmap, px, px, WICBitmapInterpolationModeHighQualityCubic) }?;
        scaler.cast()?
    };
    encode(&factory, &source, px)
}

/// The smallest system image list at least `px` big, or the biggest, with its size.
fn image_list_for(px: u32) -> Result<(IImageList, u32), JerryError> {
    let mut lists = Vec::with_capacity(4);
    for which in [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE, SHIL_JUMBO] {
        // SAFETY: a valid image list id; the Shell returns its shared list.
        let list: IImageList = unsafe { SHGetImageList(which as i32) }?;
        let (mut width, mut height) = (0, 0);
        // SAFETY: two writable sizes.
        unsafe { list.GetIconSize(&mut width, &mut height) }?;
        lists.push((list, width.max(0) as u32));
    }
    // Extra large doesn't follow the DPI, so at 200 % it is smaller than large.
    lists.sort_by_key(|(_, size)| *size);
    let index = lists
        .iter()
        .position(|(_, size)| *size >= px)
        .unwrap_or(lists.len() - 1);
    Ok(lists.swap_remove(index))
}

/// The overlay number of the shortcut arrow in the system image lists, or 0 (no overlay).
fn link_overlay() -> u32 {
    // SAFETY: no icon path; asks for the standard link overlay.
    let overlay = unsafe { SHGetIconOverlayIndexW(PCWSTR::null(), IDO_SHGIOI_LINK as i32) };
    u32::try_from(overlay).unwrap_or(0) & 0xF
}

fn encode(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    px: u32,
) -> Result<Vec<u8>, JerryError> {
    // SAFETY: an empty in-memory stream.
    let stream = unsafe { SHCreateMemStream(None) }.ok_or_else(JerryError::internal)?;
    // SAFETY: the PNG container; no preferred vendor.
    let encoder = unsafe { factory.CreateEncoder(&GUID_ContainerFormatPng, std::ptr::null()) }?;
    // SAFETY: the stream above.
    unsafe { encoder.Initialize(&stream, WICBitmapEncoderNoCache) }?;
    let mut frame = None;
    // SAFETY: a writable frame slot; no encoder options.
    unsafe { encoder.CreateNewFrame(&mut frame, std::ptr::null_mut()) }?;
    let frame = frame.ok_or_else(JerryError::internal)?;
    // SAFETY: the calls below follow WIC's frame protocol: initialize, size, format, pixels,
    // commit.
    unsafe {
        frame.Initialize(None::<&IPropertyBag2>)?;
        frame.SetSize(px, px)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    read_all(&stream)
}

fn read_all(stream: &IStream) -> Result<Vec<u8>, JerryError> {
    let mut stat = STATSTG::default();
    // SAFETY: a writable STATSTG; no name is asked for, so nothing needs freeing.
    unsafe { stream.Stat(&mut stat, STATFLAG_NONAME) }?;
    let length = u32::try_from(stat.cbSize).map_err(|_| JerryError::internal())?;
    // SAFETY: rewinds our own stream.
    unsafe { stream.Seek(0, STREAM_SEEK_SET, None) }?;
    let mut bytes = vec![0_u8; length as usize];
    let mut read = 0;
    // SAFETY: `bytes` has room for `length` bytes.
    unsafe { stream.Read(bytes.as_mut_ptr().cast(), length, Some(&mut read)) }.ok()?;
    bytes.truncate(read as usize);
    Ok(bytes)
}

/// An icon handle, destroyed on drop.
struct Icon(HICON);

impl Drop for Icon {
    fn drop(&mut self) {
        // SAFETY: an icon this module got from the image list, destroyed once.
        let _ = unsafe { DestroyIcon(self.0) };
    }
}
