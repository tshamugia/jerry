//! The `icon://` protocol: file, drive and standard-folder icons as PNGs for `<img>` (Tech PRD
//! «აიკონები და thumbnail-ები»). In WebView2 its URLs are `http://icon.localhost/<icon>`, with one
//! path segment that `src/ipc/icons.ts` builds:
//!
//! - `type-<key>-<px>`: a type icon, by the icon key from the listing batch.
//! - `entry-<listing>-<entry>-<px>`: the own icon of an entry whose key is 0 (.exe, .lnk, .ico,
//!   .url), by listing and entry id: core reads it at its exact path.
//! - `drive-<letter>-<px>`: a drive, such as `drive-C-24`.
//! - `place-<name>-<px>`: a standard place for the sidebar (`desktop`, `documents`, `downloads`,
//!   `pictures`, `onedrive`, `recyclebin`, `share`).
//!
//! `<px>` is the size in device pixels (CSS size × `devicePixelRatio`), so icons are sharp at
//! every DPI. The query string is ignored: the UI adds one so its URLs never outlive the page,
//! and an entry's modified time, so a changed file gets a new URL.
//!
//! The handler runs on the main thread and never calls the Shell there: an icon already in memory
//! is answered at once, anything else is rendered on the Shell pool and answered from there. Type
//! and entry icons don't change for their URL, so WebView2 may keep them; drives and places
//! change (media, a full Recycle Bin) and are asked for again each time.

use std::sync::Arc;

use jerry_core::{ErrorCode, JerryError};
use jerry_win::icons::{IconSource, Icons, MAX_PX, MIN_PX, Place};
use tauri::http::{Request, Response, StatusCode, header};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::listing::Listings;

/// The protocol's scheme.
pub const SCHEME: &str = "icon";

/// `ERROR_TIMEOUT`: a shell extension hung.
const ERROR_TIMEOUT: u32 = 1460;

/// What an `icon://` URL asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IconUrl {
    icon: Icon,
    px: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Icon {
    Source(IconSource),
    /// An entry of an open listing, which has its own icon.
    Entry {
        listing: u32,
        entry: u32,
    },
}

/// Answers one `icon://` request (registered in `lib.rs`).
pub fn protocol<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let Some(IconUrl { icon, px }) = parse(request.uri().path()) else {
        responder.respond(status(StatusCode::BAD_REQUEST));
        return;
    };
    let app = context.app_handle();
    let source = match icon {
        Icon::Source(source) => source,
        Icon::Entry { listing, entry } => match app.state::<Listings>().entry(listing, entry) {
            Some((path, attributes, modified)) => IconSource::File {
                path,
                attributes,
                modified,
            },
            None => {
                responder.respond(status(StatusCode::NOT_FOUND));
                return;
            }
        },
    };
    let lasting = matches!(source, IconSource::Type(_) | IconSource::File { .. });
    let icons = app.state::<Icons>();
    if let Some(png) = icons.cached(&source, px) {
        responder.respond(image(&png, lasting));
        return;
    }
    icons.render(source, px, move |result| {
        responder.respond(match result {
            Ok(png) => image(&png, lasting),
            Err(error) => failure(&error),
        });
    });
}

/// Parses the path of an `icon://` URL (see the module docs).
fn parse(path: &str) -> Option<IconUrl> {
    let parts: Vec<&str> = path.strip_prefix('/')?.split('-').collect();
    let (px, icon) = parts.split_last()?;
    let px: u32 = px.parse().ok()?;
    if !(MIN_PX..=MAX_PX).contains(&px) {
        return None;
    }
    let icon = match icon {
        ["type", key] => Icon::Source(IconSource::Type(key.parse().ok()?)),
        ["entry", listing, entry] => Icon::Entry {
            listing: listing.parse().ok()?,
            entry: entry.parse().ok()?,
        },
        ["drive", letter] => match letter.as_bytes() {
            [letter] if letter.is_ascii_alphabetic() => {
                Icon::Source(IconSource::Drive(letter.to_ascii_uppercase()))
            }
            _ => return None,
        },
        ["place", name] => Icon::Source(IconSource::Place(Place::from_name(name)?)),
        _ => return None,
    };
    Some(IconUrl { icon, px })
}

fn image(png: &Arc<[u8]>, lasting: bool) -> Response<Vec<u8>> {
    let cache = if lasting {
        "max-age=31536000, immutable"
    } else {
        "no-cache"
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CACHE_CONTROL, cache)
        .body(png.to_vec())
        .unwrap_or_else(|_| status(StatusCode::INTERNAL_SERVER_ERROR))
}

fn failure(error: &JerryError) -> Response<Vec<u8>> {
    status(match error.code {
        ErrorCode::NotFound | ErrorCode::DeviceGone => StatusCode::NOT_FOUND,
        ErrorCode::Cancelled => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::Os { os: ERROR_TIMEOUT } => StatusCode::GATEWAY_TIMEOUT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    })
}

fn status(code: StatusCode) -> Response<Vec<u8>> {
    let mut response = Response::new(Vec::new());
    *response.status_mut() = code;
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        header::HeaderValue::from_static("no-store"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(source: IconSource, px: u32) -> Option<IconUrl> {
        Some(IconUrl {
            icon: Icon::Source(source),
            px,
        })
    }

    #[test]
    fn parses_icon_urls() {
        assert_eq!(parse("/type-7-24"), url(IconSource::Type(7), 24));
        assert_eq!(
            parse("/entry-3-1200-48"),
            Some(IconUrl {
                icon: Icon::Entry {
                    listing: 3,
                    entry: 1200
                },
                px: 48
            })
        );
        assert_eq!(parse("/drive-c-16"), url(IconSource::Drive(b'C'), 16));
        assert_eq!(
            parse("/place-recyclebin-32"),
            url(IconSource::Place(Place::RecycleBin), 32)
        );
    }

    #[test]
    fn rejects_anything_else() {
        for path in [
            "",
            "/",
            "/type-7",
            "/type-x-16",
            "/type-7-16-extra",
            "/type-7-4",
            "/type-7-1024",
            "/entry-3-16",
            "/entry-3-x-16",
            "/entry-3-4-5-16",
            "/drive-CD-16",
            "/drive-1-16",
            "/place-system32-16",
            "/thumb-7-16",
            r"/type-..\..\x-16",
        ] {
            assert_eq!(parse(path), None, "{path:?}");
        }
    }

    #[test]
    fn errors_become_statuses_that_are_never_cached() {
        let response = failure(&JerryError::new(ErrorCode::NotFound));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        let hung = failure(&JerryError::from_win32(ERROR_TIMEOUT));
        assert_eq!(hung.status(), StatusCode::GATEWAY_TIMEOUT);
    }

    #[test]
    fn type_icons_are_kept_and_the_rest_asked_again() {
        let png: Arc<[u8]> = Arc::from(&b"\x89PNG"[..]);
        let kept = image(&png, true);
        assert_eq!(kept.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(
            kept.headers()[header::CACHE_CONTROL],
            "max-age=31536000, immutable"
        );
        assert_eq!(
            image(&png, false).headers()[header::CACHE_CONTROL],
            "no-cache"
        );
    }
}
