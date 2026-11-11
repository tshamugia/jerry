//! The `thumb://` protocol: thumbnails as PNGs for `<img>` (Tech PRD «აიკონები და
//! thumbnail-ები», #67). In WebView2 its URLs are `http://thumb.localhost/<thumbnail>`, with one
//! path segment that `src/ipc/thumbnails.ts` builds:
//!
//! - `entry-<listing>-<entry>-<px>`: an entry of an open listing, by listing and entry id: core
//!   reads it at its exact path.
//! - `conflict-<job>-<side>-<px>`: a side (`incoming` or `existing`) of the conflict transfer job
//!   `<job>` waits on, for the conflict window.
//!
//! `<px>` is the box the thumbnail fits in, in device pixels (CSS size × `devicePixelRatio`).
//! The query string is ignored: the UI puts the entry's modified time and attributes in it, or
//! the conflict it shows, so a changed file or another question gets a new URL.
//!
//! The handler runs on the main thread and never calls the Shell there: a thumbnail in memory is
//! answered at once, anything else is made on the thumbnails' Shell pool and answered from there.
//! A file with no thumbnail is `404`, and the UI keeps its type icon. Entry thumbnails don't
//! change for their URL, so WebView2 may keep them; a conflict's is asked for again.

use jerry_core::transfer::queue::ConflictSide;
use jerry_win::thumbnails::{MAX_PX, MIN_PX, ThumbSource, Thumbnails};
use tauri::http::{Request, StatusCode};
use tauri::{Manager, Runtime, UriSchemeContext, UriSchemeResponder};

use crate::icons::{failure, image, status};
use crate::listing::Listings;
use crate::transfers::Transfers;

/// The protocol's scheme.
pub const SCHEME: &str = "thumb";

/// What a `thumb://` URL asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ThumbUrl {
    thumb: Thumb,
    px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Thumb {
    Entry { listing: u32, entry: u32 },
    Conflict { job: u32, side: ConflictSide },
}

/// Answers one `thumb://` request (registered in `lib.rs`).
pub fn protocol<R: Runtime>(
    context: UriSchemeContext<'_, R>,
    request: Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let Some(ThumbUrl { thumb, px }) = parse(request.uri().path()) else {
        responder.respond(status(StatusCode::BAD_REQUEST));
        return;
    };
    let app = context.app_handle();
    let source = match thumb {
        Thumb::Entry { listing, entry } => {
            app.state::<Listings>()
                .entry(listing, entry)
                .map(|(path, attributes, modified)| ThumbSource {
                    path,
                    attributes,
                    modified,
                })
        }
        Thumb::Conflict { job, side } => app
            .try_state::<Transfers>()
            .and_then(|transfers| transfers.conflict_file(job, side))
            .map(|file| ThumbSource {
                path: file.path,
                attributes: file.attributes,
                modified: file.modified,
            }),
    };
    let Some(source) = source else {
        responder.respond(status(StatusCode::NOT_FOUND));
        return;
    };
    let lasting = matches!(thumb, Thumb::Entry { .. });
    let thumbnails = app.state::<Thumbnails>();
    if let Some(png) = thumbnails.cached(&source, px) {
        responder.respond(image(&png, lasting));
        return;
    }
    thumbnails.render(source, px, move |result| {
        responder.respond(match result {
            Ok(png) => image(&png, lasting),
            Err(error) => failure(&error),
        });
    });
}

/// Parses the path of a `thumb://` URL (see the module docs).
fn parse(path: &str) -> Option<ThumbUrl> {
    let parts: Vec<&str> = path.strip_prefix('/')?.split('-').collect();
    let (px, thumb) = parts.split_last()?;
    let px: u32 = px.parse().ok()?;
    if !(MIN_PX..=MAX_PX).contains(&px) {
        return None;
    }
    let thumb = match thumb {
        ["entry", listing, entry] => Thumb::Entry {
            listing: listing.parse().ok()?,
            entry: entry.parse().ok()?,
        },
        ["conflict", job, side] => Thumb::Conflict {
            job: job.parse().ok()?,
            side: match *side {
                "incoming" => ConflictSide::Incoming,
                "existing" => ConflictSide::Existing,
                _ => return None,
            },
        },
        _ => return None,
    };
    Some(ThumbUrl { thumb, px })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_thumbnail_urls() {
        assert_eq!(
            parse("/entry-3-1200-144"),
            Some(ThumbUrl {
                thumb: Thumb::Entry {
                    listing: 3,
                    entry: 1200
                },
                px: 144
            })
        );
        assert_eq!(
            parse("/conflict-7-existing-96"),
            Some(ThumbUrl {
                thumb: Thumb::Conflict {
                    job: 7,
                    side: ConflictSide::Existing
                },
                px: 96
            })
        );
        assert_eq!(
            parse("/conflict-7-incoming-1024").map(|url| url.thumb),
            Some(Thumb::Conflict {
                job: 7,
                side: ConflictSide::Incoming
            })
        );
    }

    #[test]
    fn rejects_anything_else() {
        for path in [
            "",
            "/",
            "/entry-3-96",
            "/entry-3-x-96",
            "/entry-3-4-5-96",
            "/entry-3-4-8",
            "/entry-3-4-2048",
            "/conflict-7-96",
            "/conflict-7-both-96",
            "/conflict-x-existing-96",
            "/type-7-96",
            r"/entry-..\..\x-96",
        ] {
            assert_eq!(parse(path), None, "{path:?}");
        }
    }
}
