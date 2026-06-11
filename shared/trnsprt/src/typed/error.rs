//! Errors for the typed-RPC stack.
//!
//! Split into three orthogonal kinds (per design doc):
//! - [`AdapterError`] — byte-level transport (I/O, connect, EOF, kill).
//! - [`CodecError`] — frame parse/encode at the wire format layer.
//! - [`RpcError`] — application-level / generated-stub layer.
//!
//! These are NEW and live alongside the legacy `McpError`. Phase 2 may
//! remove `McpError`; for now both coexist.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum AdapterError {
    #[error("adapter i/o: {0}")]
    Io(#[from] std::io::Error),
    #[error("adapter eof")]
    Eof,
    #[error("adapter codec: {0}")]
    Codec(#[from] CodecError),
    #[error("adapter: {0}")]
    Other(String),
}

#[derive(Debug, Error)]
pub enum CodecError {
    #[error("codec encode: {0}")]
    Encode(String),
    #[error("codec decode: {0}")]
    Decode(String),
}

#[derive(Debug, Error)]
pub enum RpcError {
    #[error("rpc adapter: {0}")]
    Adapter(String),
    #[error("rpc codec: {0}")]
    Codec(String),
    #[error("rpc method not found: {0}")]
    MethodNotFound(String),
    #[error("rpc deadline exceeded")]
    Deadline,
    #[error("rpc application error: {0}")]
    Application(String),
}

impl From<serde_json::Error> for CodecError {
    fn from(e: serde_json::Error) -> Self {
        CodecError::Decode(e.to_string())
    }
}

// `tokio_util::codec::Framed{Read,Write}` lifts the underlying I/O errors
// into the codec's `Error` type, so we have to absorb `io::Error` here.
// Treat I/O at the Decode/Encode boundary as a decode-side failure (the
// `Channel` layer above wraps it back into `AdapterError::Codec`).
impl From<std::io::Error> for CodecError {
    fn from(e: std::io::Error) -> Self {
        CodecError::Decode(format!("io: {e}"))
    }
}

// Collapse the lower transport/wire errors into the application-layer `RpcError`
// (as a string, since `RpcError` is the boundary the generated stubs return).
// Lets the `service!`-generated client/serve code use `?` instead of repeating
// `.map_err(|e| RpcError::Adapter(e.to_string()))` at every call site.
impl From<AdapterError> for RpcError {
    fn from(e: AdapterError) -> Self {
        RpcError::Adapter(e.to_string())
    }
}

impl From<CodecError> for RpcError {
    fn from(e: CodecError) -> Self {
        RpcError::Codec(e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn io_error_into_codec_is_a_decode_carrying_the_original_message() {
        // Guards `From<io::Error>` against silently truncating/altering the text.
        let io = std::io::Error::new(std::io::ErrorKind::BrokenPipe, "pipe is gone");
        let codec: CodecError = io.into();
        assert!(matches!(codec, CodecError::Decode(_)));
        let shown = codec.to_string();
        assert!(shown.starts_with("codec decode:"), "displayed as a decode error: {shown}");
        assert!(shown.contains("pipe is gone"), "original io message survives: {shown}");
    }

    #[test]
    fn serde_error_into_codec_preserves_the_serde_message() {
        let serde_err = serde_json::from_str::<serde_json::Value>("{ not json").unwrap_err();
        let original = serde_err.to_string();
        let codec: CodecError = serde_err.into();
        assert!(matches!(codec, CodecError::Decode(_)));
        assert!(codec.to_string().contains(&original), "serde message preserved");
    }

    #[test]
    fn rpc_error_absorbs_adapter_and_codec_via_from() {
        let a: RpcError = AdapterError::Eof.into();
        assert!(matches!(a, RpcError::Adapter(_)));
        assert!(a.to_string().contains("eof"), "{a}");

        let c: RpcError = CodecError::Encode("bad frame".into()).into();
        assert!(matches!(c, RpcError::Codec(_)));
        assert!(c.to_string().contains("bad frame"), "{c}");
    }
}
