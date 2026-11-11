use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::Instant;

use windows::Win32::Graphics::Imaging::{
    GUID_ContainerFormatJpeg, GUID_ContainerFormatPng, GUID_WICPixelFormat32bppBGRA,
    IWICImagingFactory, WICBitmapDitherTypeNone, WICBitmapEncoderNoCache,
    WICBitmapPaletteTypeCustom, WICDecodeMetadataCacheOnDemand,
};
use windows::Win32::System::Com::StructuredStorage::IPropertyBag2;
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::Win32::UI::Shell::SHCreateMemStream;
use windows::core::GUID;

use super::*;
use crate::wic;

const LONG: Duration = Duration::from_secs(60);
const FILE: u32 = 0x20;
const RECALL_ON_DATA_ACCESS: u32 = 0x40_0000;

fn render(thumbs: &Thumbnails, source: ThumbSource, px: u32) -> Result<Arc<[u8]>, JerryError> {
    let (sender, result) = mpsc::channel();
    thumbs.render(source, px, move |png| {
        let _ = sender.send(png);
    });
    result.recv_timeout(LONG).expect("answered")
}

fn code<T>(result: Result<T, JerryError>) -> Result<(), ErrorCode> {
    result.map(|_| ()).map_err(|error| error.code)
}

fn source(path: &Path, attributes: u32) -> ThumbSource {
    ThumbSource {
        path: path.to_path_buf(),
        attributes,
        modified: 1,
    }
}

/// WIC on the test thread, for making and reading images.
fn factory() -> IWICImagingFactory {
    // SAFETY: the test thread joins the MTA; nextest gives each test a process of its own.
    let _ = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    wic::factory().expect("WIC")
}

/// Writes a `width` × `height` image whose pixel (x, y) is `paint(x, y)` (BGRA, straight).
fn write_image(
    path: &Path,
    container: &GUID,
    width: u32,
    height: u32,
    paint: impl Fn(u32, u32) -> [u8; 4],
) {
    let factory = factory();
    let pixels: Vec<u8> = (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .flat_map(|(x, y)| paint(x, y))
        .collect();
    // SAFETY: the calls below build a bitmap over `pixels` and encode it into a memory stream,
    // following WIC's frame protocol.
    let bytes = unsafe {
        let bitmap = factory
            .CreateBitmapFromMemory(
                width,
                height,
                &GUID_WICPixelFormat32bppBGRA,
                width * 4,
                &pixels,
            )
            .expect("a bitmap");
        let stream = SHCreateMemStream(None).expect("a stream");
        let encoder = factory
            .CreateEncoder(container, std::ptr::null())
            .expect("an encoder");
        encoder
            .Initialize(&stream, WICBitmapEncoderNoCache)
            .expect("initialized");
        let mut frame = None;
        encoder
            .CreateNewFrame(&mut frame, std::ptr::null_mut())
            .expect("a frame");
        let frame = frame.expect("a frame");
        frame.Initialize(None::<&IPropertyBag2>).expect("frame");
        frame.SetSize(width, height).expect("size");
        let mut format = GUID_WICPixelFormat32bppBGRA;
        frame.SetPixelFormat(&mut format).expect("format");
        // A JPEG frame takes 24-bit BGR: convert to whatever it asked for.
        let converter = factory.CreateFormatConverter().expect("converter");
        converter
            .Initialize(
                &bitmap,
                &format,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .expect("converted");
        frame
            .WriteSource(&converter, std::ptr::null())
            .expect("pixels");
        frame.Commit().expect("commit");
        encoder.Commit().expect("commit");
        wic::read_all(&stream).expect("bytes")
    };
    std::fs::write(path, bytes).expect("written");
}

/// A PNG's size and pixels (BGRA, straight).
fn decode(png: &[u8]) -> (u32, u32, Vec<u8>) {
    let factory = factory();
    // SAFETY: a decoder over a memory stream holding `png`, read into a buffer of its size.
    unsafe {
        let stream = SHCreateMemStream(Some(png)).expect("a stream");
        let decoder = factory
            .CreateDecoderFromStream(&stream, std::ptr::null(), WICDecodeMetadataCacheOnDemand)
            .expect("a PNG");
        let frame = decoder.GetFrame(0).expect("a frame");
        let converter = factory.CreateFormatConverter().expect("converter");
        converter
            .Initialize(
                &frame,
                &GUID_WICPixelFormat32bppBGRA,
                WICBitmapDitherTypeNone,
                None,
                0.0,
                WICBitmapPaletteTypeCustom,
            )
            .expect("converted");
        let (mut width, mut height) = (0, 0);
        converter.GetSize(&mut width, &mut height).expect("size");
        let mut pixels = vec![0; (width * height * 4) as usize];
        converter
            .CopyPixels(std::ptr::null(), width * 4, &mut pixels)
            .expect("pixels");
        (width, height, pixels)
    }
}

fn pixel(image: &(u32, u32, Vec<u8>), x: u32, y: u32) -> [u8; 4] {
    let at = ((y * image.0 + x) * 4) as usize;
    image.2[at..at + 4].try_into().expect("4 bytes")
}

/// Whether two BGRA colors are within JPEG and scaling noise of each other.
fn near(a: [u8; 4], b: [u8; 4]) -> bool {
    a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 40)
}

const RED: [u8; 4] = [0, 0, 255, 255];
const BLUE: [u8; 4] = [255, 0, 0, 255];
const GREEN: [u8; 4] = [0, 160, 0, 255];

#[test]
fn photos_get_thumbnails_that_fit_their_box() {
    let dir = tempfile::tempdir().expect("temp folder");
    let photo = dir.path().join("ფოტო.jpg");
    // Wide: red on the left, blue on the right.
    write_image(&photo, &GUID_ContainerFormatJpeg, 400, 200, |x, _| {
        if x < 200 { RED } else { BLUE }
    });
    let thumbs = Thumbnails::new();
    for px in [96, 256] {
        let png = render(&thumbs, source(&photo, FILE), px).expect("a thumbnail");
        let image = decode(&png);
        assert_eq!((image.0, image.1), (px, px / 2), "fits, aspect kept");
        let (quarter, middle) = (px / 4, px / 4);
        assert!(near(pixel(&image, quarter, middle / 2), RED), "{px}");
        assert!(near(pixel(&image, px - quarter, middle / 2), BLUE), "{px}");
        assert!(
            thumbs.cached(&source(&photo, FILE), px).is_some(),
            "kept in memory"
        );
    }
    let changed = ThumbSource {
        modified: 2,
        ..source(&photo, FILE)
    };
    assert!(thumbs.cached(&changed, 96).is_none(), "the file changed");
}

#[test]
fn transparent_images_keep_their_transparency() {
    let dir = tempfile::tempdir().expect("temp folder");
    let logo = dir.path().join("ლოგო.png");
    // Tall: opaque green on top, transparent below.
    write_image(&logo, &GUID_ContainerFormatPng, 200, 400, |_, y| {
        if y < 200 { GREEN } else { [0, 0, 0, 0] }
    });
    let png = render(&Thumbnails::new(), source(&logo, FILE), 128).expect("a thumbnail");
    let image = decode(&png);
    assert_eq!((image.0, image.1), (64, 128));
    assert!(
        near(pixel(&image, 32, 32), GREEN),
        "{:?}",
        pixel(&image, 32, 32)
    );
    assert!(
        pixel(&image, 32, 100)[3] < 40,
        "{:?}",
        pixel(&image, 32, 100)
    );
}

#[test]
fn folders_long_paths_and_types_without_thumbnails_have_none() {
    let dir = tempfile::tempdir().expect("temp folder");
    let thumbs = Thumbnails::new();
    let unknown = dir.path().join("ჩანაწერი.jerrytest");
    std::fs::write(&unknown, b"no handler reads this").expect("written");
    let not_found = Err(ErrorCode::NotFound);
    assert_eq!(code(render(&thumbs, source(&unknown, FILE), 96)), not_found);
    assert_eq!(
        code(render(&thumbs, source(dir.path(), 0x10), 96)),
        not_found
    );
    let long = std::path::PathBuf::from(format!(
        r"\\?\{}\{}\photo.jpg",
        dir.path().display(),
        "ა".repeat(300)
    ));
    assert_eq!(code(render(&thumbs, source(&long, FILE), 96)), not_found);
}

#[test]
fn a_cloud_only_file_shows_only_a_thumbnail_windows_already_has() {
    let dir = tempfile::tempdir().expect("temp folder");
    let photo = dir.path().join("drawing.jpg");
    write_image(&photo, &GUID_ContainerFormatJpeg, 120, 90, |_, _| RED);
    let thumbs = Thumbnails::new();
    // Listed as online-only: never extracted from the file, and this new file isn't cached yet.
    let cloud = source(&photo, FILE | RECALL_ON_DATA_ACCESS);
    assert_eq!(
        code(render(&thumbs, cloud.clone(), 96)),
        Err(ErrorCode::NotFound)
    );
    // Here it is: the thumbnail is made, and Windows keeps it in its thumbcache.
    render(&thumbs, source(&photo, FILE), 96).expect("a thumbnail");
    let cached = render(&thumbs, cloud, 96).expect("from Windows' thumbcache");
    assert_eq!((decode(&cached).0, decode(&cached).1), (96, 72));
}

/// A fake Shell: files named `*.slow` hang until released, others get a tiny PNG.
struct Fake {
    thumbs: Thumbnails,
    calls: Arc<AtomicUsize>,
    release: Mutex<Option<mpsc::Sender<()>>>,
}

fn fake(timeout: Duration) -> Fake {
    let calls = Arc::new(AtomicUsize::new(0));
    let (release, hung) = mpsc::channel::<()>();
    let hung = Arc::new(Mutex::new(hung));
    let counted = calls.clone();
    let thumbs = Thumbnails::with(timeout, move |request| {
        counted.fetch_add(1, Ordering::SeqCst);
        let name = String::from_utf16_lossy(&request.path).to_lowercase();
        if name.trim_end_matches('\0').ends_with(".slow") {
            let _ = lock(&hung).recv();
        }
        Ok(vec![request.px as u8, u8::from(request.cache_only)])
    });
    Fake {
        thumbs,
        calls,
        release: Mutex::new(Some(release)),
    }
}

#[test]
fn a_hung_handler_times_out_and_its_type_is_then_not_asked() {
    let fake = fake(Duration::from_millis(100));
    let timed_out = Err(ErrorCode::Os {
        os: ERROR_TIMEOUT.0,
    });
    let started = Instant::now();
    let (sender, results) = mpsc::channel();
    for name in ["a.slow", "b.SLOW"] {
        let sender = sender.clone();
        fake.thumbs
            .render(source(Path::new(name), FILE), 96, move |result| {
                let _ = sender.send(code(result));
            });
    }
    for _ in 0..HUNG_AFTER {
        assert_eq!(results.recv_timeout(LONG).expect("ends"), timed_out);
    }
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);

    // The type hung twice: its next file fails at once, without a call.
    let third = render(&fake.thumbs, source(Path::new("c.slow"), FILE), 96);
    assert_eq!(code(third), timed_out);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 2);

    // Other types still get theirs, on the workers that replaced the hung ones.
    let png = render(&fake.thumbs, source(Path::new("d.jpg"), FILE), 96).expect("a PNG");
    assert_eq!(&png[..], [96, 0]);

    drop(lock(&fake.release).take());
}

#[test]
fn one_timeout_doesnt_stop_a_type() {
    let fake = fake(Duration::from_millis(100));
    let first = render(&fake.thumbs, source(Path::new("a.slow"), FILE), 96);
    assert_eq!(
        code(first),
        Err(ErrorCode::Os {
            os: ERROR_TIMEOUT.0
        })
    );
    drop(lock(&fake.release).take());
    // Released: the next file of the type is asked again.
    let second = render(&fake.thumbs, source(Path::new("b.slow"), FILE), 96).expect("a PNG");
    assert_eq!(&second[..], [96, 0]);
}

#[test]
fn sizes_are_clamped_and_cloud_state_is_part_of_the_cache_key() {
    let fake = fake(LONG);
    let file = source(Path::new(r"C:\jerry-test\photo.jpg"), FILE);
    assert_eq!(
        &render(&fake.thumbs, file.clone(), 2).expect("png")[..],
        [16, 0]
    );
    assert_eq!(
        &render(&fake.thumbs, file.clone(), 5000).expect("png")[..],
        [0, 0]
    );
    assert!(fake.thumbs.cached(&file, 4).is_some(), "clamped to 16 too");
    let cloud = source(&file.path, FILE | RECALL_ON_DATA_ACCESS);
    assert!(fake.thumbs.cached(&cloud, 16).is_none());
    assert_eq!(&render(&fake.thumbs, cloud, 16).expect("png")[..], [16, 1]);
    assert_eq!(fake.calls.load(Ordering::SeqCst), 3);
}

/// A fake OneDrive: a Cloud Files sync root in a temp folder, with an online-only placeholder
/// whose content would have to be fetched from the "cloud". Every fetch is counted and refused.
///
/// Windows doesn't fetch a placeholder's content for a thumbnail on its own either (checked on
/// Windows 11: without `SIIGBF_INCACHEONLY` it asks the sync provider for one, and this fake has
/// none); with OneDrive that is a thumbnail from OneDrive's servers, which the flag rules out
/// (a hand check, `docs/checklists/`). This test guards Jerry's whole path: nothing it does for
/// a thumbnail reads the content.
mod placeholder {
    use std::os::windows::ffi::OsStrExt;

    use windows::Win32::Foundation::NTSTATUS;
    use windows::Win32::Storage::CloudFilters::{
        CF_CALLBACK_INFO, CF_CALLBACK_PARAMETERS, CF_CALLBACK_REGISTRATION,
        CF_CALLBACK_TYPE_FETCH_DATA, CF_CALLBACK_TYPE_NONE, CF_CONNECT_FLAG_NONE,
        CF_CONNECTION_KEY, CF_CREATE_FLAG_NONE, CF_FS_METADATA, CF_HARDLINK_POLICY_NONE,
        CF_HYDRATION_POLICY, CF_HYDRATION_POLICY_FULL, CF_HYDRATION_POLICY_MODIFIER_NONE,
        CF_INSYNC_POLICY_NONE, CF_OPERATION_INFO, CF_OPERATION_PARAMETERS,
        CF_OPERATION_PARAMETERS_0, CF_OPERATION_PARAMETERS_0_0,
        CF_OPERATION_TRANSFER_DATA_FLAG_NONE, CF_OPERATION_TYPE_TRANSFER_DATA,
        CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC, CF_PLACEHOLDER_CREATE_INFO,
        CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT, CF_POPULATION_POLICY,
        CF_POPULATION_POLICY_ALWAYS_FULL, CF_POPULATION_POLICY_MODIFIER_NONE,
        CF_REGISTER_FLAG_NONE, CF_SYNC_POLICIES, CF_SYNC_REGISTRATION, CfConnectSyncRoot,
        CfCreatePlaceholders, CfDisconnectSyncRoot, CfExecute, CfRegisterSyncRoot,
        CfUnregisterSyncRoot,
    };
    use windows::Win32::Storage::FileSystem::{FILE_BASIC_INFO, GetFileAttributesW};
    use windows::core::{GUID, PCWSTR, w};

    use super::*;

    static FETCHES: AtomicUsize = AtomicUsize::new(0);

    /// `STATUS_CLOUD_FILE_UNSUCCESSFUL`: the provider refuses the fetch.
    const REFUSED: NTSTATUS = NTSTATUS(0xC000_CF0C_u32 as i32);

    /// Counts a fetch and refuses it, so nothing is downloaded and the reader isn't kept waiting.
    unsafe extern "system" fn fetch_data(
        info: *const CF_CALLBACK_INFO,
        parameters: *const CF_CALLBACK_PARAMETERS,
    ) {
        FETCHES.fetch_add(1, Ordering::SeqCst);
        // SAFETY: Cloud Files passes valid pointers for the length of the callback; `FetchData`
        // is the member a FETCH_DATA callback gets.
        unsafe {
            let info = &*info;
            let fetch = (*parameters).Anonymous.FetchData;
            let operation = CF_OPERATION_INFO {
                StructSize: size_of::<CF_OPERATION_INFO>() as u32,
                Type: CF_OPERATION_TYPE_TRANSFER_DATA,
                ConnectionKey: info.ConnectionKey,
                TransferKey: info.TransferKey,
                CorrelationVector: info.CorrelationVector,
                SyncStatus: std::ptr::null(),
                RequestKey: info.RequestKey,
            };
            let mut refusal = CF_OPERATION_PARAMETERS {
                ParamSize: size_of::<CF_OPERATION_PARAMETERS>() as u32,
                Anonymous: CF_OPERATION_PARAMETERS_0 {
                    TransferData: CF_OPERATION_PARAMETERS_0_0 {
                        Flags: CF_OPERATION_TRANSFER_DATA_FLAG_NONE,
                        CompletionStatus: REFUSED,
                        Buffer: std::ptr::null(),
                        Offset: fetch.RequiredFileOffset,
                        Length: fetch.RequiredLength,
                    },
                },
            };
            let _ = CfExecute(&operation, &mut refusal);
        }
    }

    /// The sync root, unregistered on drop.
    struct SyncRoot {
        path: Vec<u16>,
        connection: CF_CONNECTION_KEY,
    }

    impl SyncRoot {
        /// `None` where Cloud Files isn't available (Windows Server runners).
        fn register(root: &Path) -> Option<SyncRoot> {
            let mut path: Vec<u16> = root.as_os_str().encode_wide().collect();
            path.push(0);
            let identity = b"jerry-test-root";
            let registration = CF_SYNC_REGISTRATION {
                StructSize: size_of::<CF_SYNC_REGISTRATION>() as u32,
                ProviderName: w!("Jerry test cloud"),
                ProviderVersion: w!("1"),
                SyncRootIdentity: identity.as_ptr().cast(),
                SyncRootIdentityLength: identity.len() as u32,
                FileIdentity: std::ptr::null(),
                FileIdentityLength: 0,
                ProviderId: GUID::from_u128(0x6a65_7272_7974_6573_7463_6c6f_7564_0001),
            };
            let policies = CF_SYNC_POLICIES {
                StructSize: size_of::<CF_SYNC_POLICIES>() as u32,
                Hydration: CF_HYDRATION_POLICY {
                    Primary: CF_HYDRATION_POLICY_FULL,
                    Modifier: CF_HYDRATION_POLICY_MODIFIER_NONE,
                },
                Population: CF_POPULATION_POLICY {
                    Primary: CF_POPULATION_POLICY_ALWAYS_FULL,
                    Modifier: CF_POPULATION_POLICY_MODIFIER_NONE,
                },
                InSync: CF_INSYNC_POLICY_NONE,
                HardLink: CF_HARDLINK_POLICY_NONE,
                PlaceholderManagement: CF_PLACEHOLDER_MANAGEMENT_POLICY_DEFAULT,
            };
            // SAFETY: a NUL-terminated folder and valid structures, read during the call.
            let registered = unsafe {
                CfRegisterSyncRoot(
                    PCWSTR(path.as_ptr()),
                    &registration,
                    &policies,
                    CF_REGISTER_FLAG_NONE,
                )
            };
            if let Err(error) = registered {
                eprintln!("Cloud Files isn't available here: {error}");
                return None;
            }
            let callbacks = [
                CF_CALLBACK_REGISTRATION {
                    Type: CF_CALLBACK_TYPE_FETCH_DATA,
                    Callback: Some(fetch_data),
                },
                CF_CALLBACK_REGISTRATION {
                    Type: CF_CALLBACK_TYPE_NONE,
                    Callback: None,
                },
            ];
            // SAFETY: the registered folder and a callback table ended by CF_CALLBACK_TYPE_NONE;
            // the callback is a static function.
            let connected = unsafe {
                CfConnectSyncRoot(
                    PCWSTR(path.as_ptr()),
                    callbacks.as_ptr(),
                    None,
                    CF_CONNECT_FLAG_NONE,
                )
            };
            match connected {
                Ok(connection) => Some(SyncRoot { path, connection }),
                Err(error) => {
                    // SAFETY: the folder registered above.
                    let _ = unsafe { CfUnregisterSyncRoot(PCWSTR(path.as_ptr())) };
                    eprintln!("Cloud Files didn't connect: {error}");
                    None
                }
            }
        }

        /// An online-only file of `size` bytes named `name` in the root.
        fn placeholder(&self, name: &str, size: i64) {
            let name: Vec<u16> = name.encode_utf16().chain([0]).collect();
            let identity = b"jerry-test-file";
            let filetime = 133_500_000_000_000_000; // 2024
            let mut info = [CF_PLACEHOLDER_CREATE_INFO {
                RelativeFileName: PCWSTR(name.as_ptr()),
                FsMetadata: CF_FS_METADATA {
                    BasicInfo: FILE_BASIC_INFO {
                        CreationTime: filetime,
                        LastAccessTime: filetime,
                        LastWriteTime: filetime,
                        ChangeTime: filetime,
                        FileAttributes: 0x80, // FILE_ATTRIBUTE_NORMAL
                    },
                    FileSize: size,
                },
                FileIdentity: identity.as_ptr().cast(),
                FileIdentityLength: identity.len() as u32,
                Flags: CF_PLACEHOLDER_CREATE_FLAG_MARK_IN_SYNC,
                ..Default::default()
            }];
            // SAFETY: the root, NUL-terminated, and one placeholder description.
            unsafe {
                CfCreatePlaceholders(
                    PCWSTR(self.path.as_ptr()),
                    &mut info,
                    CF_CREATE_FLAG_NONE,
                    None,
                )
            }
            .expect("a placeholder");
            info[0].Result.ok().expect("a placeholder");
        }
    }

    impl Drop for SyncRoot {
        fn drop(&mut self) {
            // SAFETY: the connection and the folder this SyncRoot made, undone once.
            unsafe {
                let _ = CfDisconnectSyncRoot(self.connection);
                let _ = CfUnregisterSyncRoot(PCWSTR(self.path.as_ptr()));
            }
        }
    }

    fn attributes(path: &Path) -> u32 {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        // SAFETY: a NUL-terminated path.
        unsafe { GetFileAttributesW(PCWSTR(wide.as_ptr())) }
    }

    #[test]
    fn an_online_only_file_is_never_downloaded_for_its_thumbnail() {
        let dir = tempfile::tempdir().expect("temp folder");
        let Some(root) = SyncRoot::register(dir.path()) else {
            // GitHub's Windows Server runners have no Cloud Files filter; it runs everywhere
            // Jerry does.
            assert!(std::env::var_os("CI").is_some(), "Cloud Files is missing");
            return;
        };
        root.placeholder("ნახაზი.jpg", 200_000);
        root.placeholder("გეგმა.pdf", 200_000);
        let thumbs = Thumbnails::new();
        for name in ["ნახაზი.jpg", "გეგმა.pdf"] {
            let path = dir.path().join(name);
            let listed = attributes(&path);
            assert_ne!(listed & CLOUD_ONLY, 0, "{name} is online-only: {listed:#x}");

            // As listed, and as a stale listing that thinks the file is here.
            for attributes in [listed, FILE] {
                let result = render(&thumbs, source(&path, attributes), 96);
                assert_eq!(code(result), Err(ErrorCode::NotFound), "{name}");
            }
            assert_eq!(FETCHES.load(Ordering::SeqCst), 0, "{name} was fetched");
            assert_ne!(
                attributes(&path) & CLOUD_ONLY,
                0,
                "{name} is still online-only"
            );
        }

        // The fake cloud does see a read: reading the content asks for it.
        let read = std::fs::read(dir.path().join("ნახაზი.jpg"));
        assert!(read.is_err(), "the fetch is refused");
        assert!(FETCHES.load(Ordering::SeqCst) > 0, "a read is a fetch");
        drop(root);
    }
}
