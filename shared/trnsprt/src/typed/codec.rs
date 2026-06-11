//! Codec trait + concrete implementations.
//!
//! A `Codec` is the wire format. Its API mirrors `tokio_util::codec`:
//! `encode(frame, &mut BytesMut)` writes one frame's bytes into `dst`,
//! `decode(&mut BytesMut)` tries to extract one frame from `src`. A
//! blanket impl below converts any `Codec` into the matching
//! `tokio_util::codec::{Encoder, Decoder}` so codecs compose directly
//! with `FramedRead`/`FramedWrite`.

use bytes::BytesMut;
use serde_json::Value;
use tokio_util::codec::{Decoder, Encoder};

use super::error::CodecError;

pub trait Codec: Send + 'static {
    type Frame: Send;
    fn encode(&mut self, frame: Self::Frame, dst: &mut BytesMut) -> Result<(), CodecError>;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Frame>, CodecError>;
}

// ---- Bridge to `tokio_util::codec` ------------------------------------------
//
// We can't write a generic blanket `impl<T: Codec> Encoder<T::Frame> for T`
// without orphan-rule grief, so each concrete `Codec` impl below also gets
// an explicit `Encoder` + `Decoder` impl that just delegates to the trait.

/// Line-delimited JSON envelope.
///
/// Frames are `serde_json::Value`. Each frame is encoded as the JSON text
/// of the value followed by a `\n`. Decoding scans for the next `\n` in
/// the buffer. The envelope shape (`{ id, method, params }` /
/// `{ id, result|error }`) is owned by the caller — this codec only
/// shuttles `Value`s.
#[derive(Default)]
pub struct JsonEnvelopeCodec;

impl JsonEnvelopeCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Codec for JsonEnvelopeCodec {
    type Frame = Value;

    fn encode(&mut self, frame: Self::Frame, dst: &mut BytesMut) -> Result<(), CodecError> {
        let s = serde_json::to_string(&frame)
            .map_err(|e| CodecError::Encode(e.to_string()))?;
        if s.contains('\n') {
            return Err(CodecError::Encode("frame contained newline".into()));
        }
        dst.extend_from_slice(s.as_bytes());
        dst.extend_from_slice(b"\n");
        Ok(())
    }

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Frame>, CodecError> {
        // Loop (not recursion) over leading blank lines: a buffer of N
        // consecutive newlines previously recursed N deep and could blow the
        // stack. Each iteration consumes one line; an empty one is skipped.
        loop {
            let Some(pos) = src.iter().position(|&b| b == b'\n') else {
                return Ok(None);
            };
            let line = src.split_to(pos + 1);
            let slice = &line[..pos];
            let trimmed = if slice.last() == Some(&b'\r') {
                &slice[..slice.len() - 1]
            } else {
                slice
            };
            if trimmed.is_empty() {
                continue;
            }
            let v: Value = serde_json::from_slice(trimmed)
                .map_err(|e| CodecError::Decode(e.to_string()))?;
            return Ok(Some(v));
        }
    }
}

impl Encoder<Value> for JsonEnvelopeCodec {
    type Error = CodecError;
    fn encode(&mut self, item: Value, dst: &mut BytesMut) -> Result<(), Self::Error> {
        <Self as Codec>::encode(self, item, dst)
    }
}

impl Decoder for JsonEnvelopeCodec {
    type Item = Value;
    type Error = CodecError;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        <Self as Codec>::decode(self, src)
    }
}

/// Length-delimited bincode (bincode 2 standard config).
///
/// Frames are `Vec<u8>` payloads — the caller bincode-encodes their typed
/// envelope into bytes before handing it to the codec. (We could be more
/// clever and parameterise on the frame type, but Phase 1 only needs the
/// JSON codec; this is here as a placeholder for hot-path hops.)
#[derive(Default)]
pub struct BincodeCodec;

/// Hard cap on a single bincode frame (64 MiB). Bounds the buffer the framing
/// layer will grow waiting for a frame: without it, a malformed or malicious
/// 4-byte header claiming up to `u32::MAX` (~4 GiB) makes the reader buffer that
/// much before `decode` ever yields, an OOM/DoS vector. Encode rejects oversized
/// frames too, so we never emit one we'd refuse to decode.
pub const MAX_FRAME_LEN: usize = 64 * 1024 * 1024;

impl BincodeCodec {
    pub fn new() -> Self {
        Self
    }
}

impl Codec for BincodeCodec {
    type Frame = Vec<u8>;

    fn encode(&mut self, frame: Self::Frame, dst: &mut BytesMut) -> Result<(), CodecError> {
        if frame.len() > MAX_FRAME_LEN {
            return Err(CodecError::Encode(format!(
                "frame length {} exceeds max {MAX_FRAME_LEN}",
                frame.len()
            )));
        }
        let len = u32::try_from(frame.len())
            .map_err(|_| CodecError::Encode("frame too large".into()))?;
        dst.extend_from_slice(&len.to_be_bytes());
        dst.extend_from_slice(&frame);
        Ok(())
    }

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Frame>, CodecError> {
        if src.len() < 4 {
            return Ok(None);
        }
        let mut len_buf = [0u8; 4];
        len_buf.copy_from_slice(&src[..4]);
        let len = u32::from_be_bytes(len_buf) as usize;
        // Reject before waiting for / reserving the buffer — a bogus length must
        // fail fast, not stall the reader into buffering gigabytes.
        if len > MAX_FRAME_LEN {
            return Err(CodecError::Decode(format!(
                "frame length {len} exceeds max {MAX_FRAME_LEN}"
            )));
        }
        if src.len() < 4 + len {
            return Ok(None);
        }
        let _ = src.split_to(4);
        let payload = src.split_to(len);
        Ok(Some(payload.to_vec()))
    }
}

impl Encoder<Vec<u8>> for BincodeCodec {
    type Error = CodecError;
    fn encode(&mut self, item: Vec<u8>, dst: &mut BytesMut) -> Result<(), Self::Error> {
        <Self as Codec>::encode(self, item, dst)
    }
}

impl Decoder for BincodeCodec {
    type Item = Vec<u8>;
    type Error = CodecError;
    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        <Self as Codec>::decode(self, src)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // `encode`/`decode` are ambiguous at call sites because both the `Codec`
    // trait and tokio_util's `Encoder`/`Decoder` are in scope for these types.
    // These generic shims are bound only by `Codec`, so they resolve to the
    // trait methods unambiguously and keep the tests readable.
    fn enc<C: Codec>(c: &mut C, frame: C::Frame, b: &mut BytesMut) -> Result<(), CodecError> {
        c.encode(frame, b)
    }
    fn dec<C: Codec>(c: &mut C, b: &mut BytesMut) -> Result<Option<C::Frame>, CodecError> {
        c.decode(b)
    }

    // ---- JsonEnvelopeCodec --------------------------------------------------

    #[test]
    fn json_roundtrip_single_frame() {
        let mut c = JsonEnvelopeCodec::new();
        let mut buf = BytesMut::new();
        enc(&mut c, json!({"id": 1, "method": "ping"}), &mut buf).unwrap();
        let got = dec(&mut c, &mut buf).unwrap().expect("one frame");
        assert_eq!(got, json!({"id": 1, "method": "ping"}));
        // Buffer fully consumed; nothing more to decode.
        assert!(dec(&mut c, &mut buf).unwrap().is_none());
    }

    #[test]
    fn json_decodes_multiple_frames_from_one_buffer() {
        let mut c = JsonEnvelopeCodec::new();
        let mut buf = BytesMut::new();
        enc(&mut c, json!({"a": 1}), &mut buf).unwrap();
        enc(&mut c, json!({"b": 2}), &mut buf).unwrap();
        assert_eq!(dec(&mut c, &mut buf).unwrap().unwrap(), json!({"a": 1}));
        assert_eq!(dec(&mut c, &mut buf).unwrap().unwrap(), json!({"b": 2}));
        assert!(dec(&mut c, &mut buf).unwrap().is_none());
    }

    #[test]
    fn json_tolerates_crlf_and_skips_blank_lines() {
        let mut c = JsonEnvelopeCodec::new();
        // Leading blank lines (incl. CRLF blanks) then a CRLF-terminated frame.
        let mut buf = BytesMut::from(&b"\n\r\n{\"ok\":true}\r\n"[..]);
        let got = dec(&mut c, &mut buf).unwrap().expect("frame after blanks");
        assert_eq!(got, json!({"ok": true}));
        assert!(dec(&mut c, &mut buf).unwrap().is_none());
    }

    #[test]
    fn json_many_consecutive_newlines_do_not_overflow() {
        // Regression: the old recursive blank-line skip blew the stack on a
        // buffer of many newlines. The loop handles 100k blanks in O(n) flat.
        let mut c = JsonEnvelopeCodec::new();
        let mut bytes = vec![b'\n'; 100_000];
        bytes.extend_from_slice(b"{\"v\":42}\n");
        let mut buf = BytesMut::from(&bytes[..]);
        assert_eq!(dec(&mut c, &mut buf).unwrap().unwrap(), json!({"v": 42}));
    }

    #[test]
    fn json_partial_line_yields_none_until_newline() {
        let mut c = JsonEnvelopeCodec::new();
        let mut buf = BytesMut::from(&b"{\"partial\":1}"[..]); // no trailing \n
        assert!(dec(&mut c, &mut buf).unwrap().is_none(), "incomplete line -> None");
        buf.extend_from_slice(b"\n");
        assert_eq!(dec(&mut c, &mut buf).unwrap().unwrap(), json!({"partial": 1}));
    }

    // ---- BincodeCodec -------------------------------------------------------

    #[test]
    fn bincode_roundtrip_and_multi_frame() {
        let mut c = BincodeCodec::new();
        let mut buf = BytesMut::new();
        enc(&mut c, vec![1, 2, 3], &mut buf).unwrap();
        enc(&mut c, vec![9, 8], &mut buf).unwrap();
        assert_eq!(dec(&mut c, &mut buf).unwrap().unwrap(), vec![1, 2, 3]);
        assert_eq!(dec(&mut c, &mut buf).unwrap().unwrap(), vec![9, 8]);
        assert!(dec(&mut c, &mut buf).unwrap().is_none());
    }

    #[test]
    fn bincode_partial_header_and_partial_payload_yield_none() {
        let mut c = BincodeCodec::new();
        // Fewer than 4 header bytes -> None.
        let mut buf = BytesMut::from(&[0u8, 0u8][..]);
        assert!(dec(&mut c, &mut buf).unwrap().is_none());
        // Full header (len=5) but only 2 payload bytes -> None, header untouched.
        let mut buf = BytesMut::new();
        buf.extend_from_slice(&5u32.to_be_bytes());
        buf.extend_from_slice(&[1, 2]);
        assert!(dec(&mut c, &mut buf).unwrap().is_none());
        assert_eq!(buf.len(), 6, "buffer left intact awaiting the rest");
    }

    #[test]
    fn bincode_decode_rejects_oversized_length_header() {
        let mut c = BincodeCodec::new();
        let mut buf = BytesMut::new();
        // A header claiming > MAX_FRAME_LEN must fail fast, not buffer/alloc it.
        buf.extend_from_slice(&((MAX_FRAME_LEN as u32) + 1).to_be_bytes());
        let err = dec(&mut c, &mut buf).unwrap_err();
        assert!(matches!(err, CodecError::Decode(_)), "oversized header -> Decode err, got {err:?}");
    }

    #[test]
    fn bincode_encode_rejects_oversized_frame() {
        let mut c = BincodeCodec::new();
        let mut buf = BytesMut::new();
        // Symmetric with decode's cap: a frame past MAX_FRAME_LEN is refused and
        // nothing is written, so we never emit a frame we'd later reject.
        let oversized = vec![0u8; MAX_FRAME_LEN + 1];
        let err = enc(&mut c, oversized, &mut buf).unwrap_err();
        assert!(matches!(err, CodecError::Encode(_)), "oversized frame -> Encode err, got {err:?}");
        assert!(buf.is_empty(), "nothing written for a rejected frame");
    }
}
