import { Channel } from "@tauri-apps/api/core";

/**
 * A stream from a command (a Rust `Channel<T>`): `onMessage` gets each message, in order. Pass it
 * as the command's channel argument. A Rust `Channel<Bytes>` delivers `ArrayBuffer`s.
 */
export function stream<T>(onMessage: (message: T) => void): Channel<T> {
  return new Channel<T>(onMessage);
}

/** Size of the header every binary message starts with (ADR-0001); the body follows it. */
export const HEADER_BYTES = 16;

/**
 * The header of a binary message: four little-endian `u32`s, the first one the message kind,
 * the other three depending on it. A view into `message`, nothing is copied.
 */
export function messageHeader(message: ArrayBuffer): Uint32Array {
  if (message.byteLength < HEADER_BYTES) {
    throw new RangeError(`A ${message.byteLength}-byte message has no header`);
  }
  return new Uint32Array(message, 0, HEADER_BYTES / Uint32Array.BYTES_PER_ELEMENT);
}
