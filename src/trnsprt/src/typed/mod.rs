pub mod adapter;
pub mod channel;
pub mod codec;
pub mod error;
pub mod local;

pub use adapter::{Adapter, AsyncStdioAdapter, InprocAdapter, TcpAdapter};
pub use channel::Channel;
pub use codec::{BincodeCodec, Codec, JsonEnvelopeCodec};
pub use error::{AdapterError, CodecError, RpcError};
#[cfg(windows)]
pub use local::NamedPipeAdapter;
#[cfg(unix)]
pub use local::UnixStreamAdapter;
pub use local::{
	bind_kern_listener, connect_kern, BindError, BindOutcome, Endpoint, LocalAdapter, LocalListener,
};
