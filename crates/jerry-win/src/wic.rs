//! WIC (Windows Imaging Component) helpers that icons and thumbnails share: the imaging factory
//! and PNG encoding. Call them on a thread where COM is initialized (a Shell pool worker).

use jerry_core::JerryError;
use windows::Win32::Graphics::Imaging::{
    CLSID_WICImagingFactory, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICBitmapSource, IWICImagingFactory, WICBitmapEncoderNoCache,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::Com::{
    CLSCTX_INPROC_SERVER, CoCreateInstance, IStream, STATFLAG_NONAME, STATSTG, STREAM_SEEK_SET,
};
use windows::Win32::UI::Shell::SHCreateMemStream;

/// A WIC imaging factory.
pub fn factory() -> Result<IWICImagingFactory, JerryError> {
    // SAFETY: COM is initialized on this thread (a Shell pool worker).
    Ok(unsafe { CoCreateInstance(&CLSID_WICImagingFactory, None, CLSCTX_INPROC_SERVER) }?)
}

/// `source`, `width` × `height` pixels, as a PNG (32-bit BGRA, straight alpha).
pub fn png(
    factory: &IWICImagingFactory,
    source: &IWICBitmapSource,
    width: u32,
    height: u32,
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
        frame.SetSize(width, height)?;
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format)?;
        frame.WriteSource(source, std::ptr::null())?;
        frame.Commit()?;
        encoder.Commit()?;
    }
    read_all(&stream)
}

pub(crate) fn read_all(stream: &IStream) -> Result<Vec<u8>, JerryError> {
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
