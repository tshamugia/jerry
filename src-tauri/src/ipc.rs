//! Shared IPC types. Commands (`invoke`) return `Result<T, JerryError>`; streams use a Tauri
//! `Channel`, binary ones a `Channel<Bytes>`; events go to all windows through tauri-specta
//! events; `<img>` and `<video>` data comes from custom protocols. The UI's side is `src/ipc/`.

use serde::Deserialize;
use specta::Type;
use specta::datatype::DataType;
use tauri::ipc::{InvokeResponseBody, IpcResponse};

/// Any JSON value from the UI, `unknown` in the bindings: the command checks its shape (a
/// setting's value, say). The UI's typed wrapper decides what it sends.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(transparent)]
pub struct Json(pub serde_json::Value);

impl Type for Json {
    fn definition(_: &mut specta::Types) -> DataType {
        DataType::Reference(specta_typescript::define("unknown"))
    }
}

/// A binary IPC payload: it reaches the UI as an `ArrayBuffer`, with no JSON in between
/// (ADR-0001). Return it from a command or send it through a `Channel<Bytes>`. Binary messages
/// start with a 16-byte header whose first `u32` (little-endian) is the message kind, so the
/// columns that follow stay aligned for typed-array views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bytes(pub Vec<u8>);

impl IpcResponse for Bytes {
    fn body(self) -> tauri::Result<InvokeResponseBody> {
        Ok(InvokeResponseBody::Raw(self.0))
    }
}

impl Type for Bytes {
    fn definition(_: &mut specta::Types) -> DataType {
        DataType::Reference(specta_typescript::define("ArrayBuffer"))
    }
}

/// Builds a binary message: the 16-byte header `[kind, a, b, c]` (little-endian `u32`s), then
/// the body. Columns in the body should keep 8-byte alignment where they hold `f64`s.
pub fn message(header: [u32; 4], body: &[u8]) -> Bytes {
    let mut bytes = message_buffer(header, body.len());
    bytes.extend_from_slice(body);
    Bytes(bytes)
}

/// A binary message's buffer: the 16-byte header, with room for `body_capacity` more bytes.
/// Write the body after it and wrap it in [`Bytes`]; for bodies built in place, with no copy.
pub fn message_buffer(header: [u32; 4], body_capacity: usize) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(16 + body_capacity);
    for word in header {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_comes_in_as_is() {
        let value = serde_json::json!({ "kind": "folder", "path": r"D:\ნახაზები" });
        let json: Json = serde_json::from_value(value.clone()).expect("deserializes");
        assert_eq!(json, Json(value));
    }

    #[test]
    fn bytes_go_out_raw() {
        let body = Bytes(vec![1, 2, 3]).body().expect("body");
        assert!(matches!(body, InvokeResponseBody::Raw(raw) if raw == [1, 2, 3]));
    }

    #[test]
    fn messages_start_with_a_little_endian_header() {
        let Bytes(bytes) = message([7, 1, 0x0102_0304, 0], &[0xAA]);
        assert_eq!(bytes.len(), 17);
        assert_eq!(bytes[..4], [7, 0, 0, 0]);
        assert_eq!(bytes[8..12], [4, 3, 2, 1]);
        assert_eq!(bytes[16], 0xAA);
    }
}
