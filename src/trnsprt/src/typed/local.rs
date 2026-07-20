#[cfg(unix)]
use std::path::{Path, PathBuf};

use super::adapter::{Adapter, DynRead, DynWrite};
use super::error::AdapterError;

#[derive(Clone, Debug)]
pub enum Endpoint {
	#[cfg(unix)]
	Unix(PathBuf),
	#[cfg(windows)]
	NamedPipe(String),
}

impl Endpoint {
	pub fn kern() -> Self {
		let dir = std::env::current_dir().unwrap_or_default();
		Self::kern_for(&dir)
	}

	// Same socket a daemon running with cwd `root` binds — lets the hub address
	// a node's endpoint without being in that directory.
	pub fn kern_for(root: &std::path::Path) -> Self {
		let tag = path_tag(root);
		Self::scoped(&format!("kern-{tag}"))
	}

	pub fn hub() -> Self {
		Self::scoped("kern-hub")
	}

	// Reconstruct from the wire form produced by `display()`.
	pub fn parse(s: &str) -> Self {
		#[cfg(unix)]
		{
			Endpoint::Unix(PathBuf::from(s))
		}
		#[cfg(windows)]
		{
			Endpoint::NamedPipe(s.to_string())
		}
	}

	fn scoped(name: &str) -> Self {
		#[cfg(unix)]
		{
			let path = std::env::var_os("XDG_RUNTIME_DIR")
				.map(PathBuf::from)
				.map(|d| d.join(format!("{name}.sock")))
				.unwrap_or_else(|| {
					let user = std::env::var("USER").unwrap_or_else(|_| "default".into());
					PathBuf::from(format!("/tmp/{name}-{user}.sock"))
				});
			Endpoint::Unix(path)
		}
		#[cfg(windows)]
		{
			let user = std::env::var("USERNAME").unwrap_or_else(|_| "default".into());
			Endpoint::NamedPipe(format!(r"\\.\pipe\{name}-{user}"))
		}
	}

	pub fn display(&self) -> String {
		match self {
			#[cfg(unix)]
			Endpoint::Unix(p) => p.display().to_string(),
			#[cfg(windows)]
			Endpoint::NamedPipe(n) => n.clone(),
		}
	}
}

// FNV-1a over the canonical path — MUST stay stable across processes (unlike
// DefaultHasher's randomized state) so daemon, hub, and clients always agree.
fn path_tag(dir: &std::path::Path) -> String {
	let canon = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
	let s = canon.to_string_lossy();
	let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
	for b in s.as_bytes() {
		hash ^= *b as u64;
		hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
	}
	format!("{hash:016x}")
}

#[cfg(test)]
mod cwd_tag_tests {
	use super::*;

	#[test]
	fn path_tag_is_stable_and_nonempty() {
		let dir = std::env::current_dir().unwrap();
		let a = path_tag(&dir);
		let b = path_tag(&dir);
		assert_eq!(a, b, "same path must yield the same tag");
		assert_eq!(a.len(), 16, "tag is 16 hex chars");
		assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
	}

	#[test]
	fn endpoint_kern_includes_tag() {
		let dir = std::env::current_dir().unwrap();
		let ep = Endpoint::kern();
		assert!(
			ep.display().contains(&path_tag(&dir)),
			"endpoint scoped by cwd tag"
		);
	}

	#[test]
	fn kern_for_cwd_matches_kern() {
		let dir = std::env::current_dir().unwrap();
		assert_eq!(
			Endpoint::kern().display(),
			Endpoint::kern_for(&dir).display(),
			"hub-computed endpoint must match the node's own"
		);
	}

	#[test]
	fn parse_round_trips_display() {
		let ep = Endpoint::hub();
		assert_eq!(Endpoint::parse(&ep.display()).display(), ep.display());
	}
}

#[cfg(unix)]
pub struct UnixStreamAdapter {
	stream: tokio::net::UnixStream,
}

#[cfg(unix)]
impl UnixStreamAdapter {
	pub fn new(stream: tokio::net::UnixStream) -> Self {
		Self { stream }
	}
	pub async fn connect(path: &Path) -> Result<Self, AdapterError> {
		let stream = tokio::net::UnixStream::connect(path).await?;
		Ok(Self { stream })
	}
}

#[cfg(unix)]
impl Adapter for UnixStreamAdapter {
	fn split(self: Box<Self>) -> (DynRead, DynWrite) {
		let (r, w) = self.stream.into_split();
		(Box::new(r), Box::new(w))
	}
}

#[cfg(windows)]
pub struct NamedPipeAdapter {
	inner: NamedPipeInner,
}

#[cfg(windows)]
enum NamedPipeInner {
	Server(tokio::net::windows::named_pipe::NamedPipeServer),
	Client(tokio::net::windows::named_pipe::NamedPipeClient),
}

#[cfg(windows)]
impl NamedPipeAdapter {
	pub fn from_server(server: tokio::net::windows::named_pipe::NamedPipeServer) -> Self {
		Self {
			inner: NamedPipeInner::Server(server),
		}
	}
	pub async fn connect(pipe_name: &str) -> Result<Self, AdapterError> {
		let client = tokio::net::windows::named_pipe::ClientOptions::new().open(pipe_name)?;
		Ok(Self {
			inner: NamedPipeInner::Client(client),
		})
	}
}

#[cfg(windows)]
impl Adapter for NamedPipeAdapter {
	fn split(self: Box<Self>) -> (DynRead, DynWrite) {
		match self.inner {
			NamedPipeInner::Server(s) => {
				let (r, w) = tokio::io::split(s);
				(Box::new(r), Box::new(w))
			}
			NamedPipeInner::Client(c) => {
				let (r, w) = tokio::io::split(c);
				(Box::new(r), Box::new(w))
			}
		}
	}
}

pub enum LocalAdapter {
	#[cfg(unix)]
	Unix(UnixStreamAdapter),
	#[cfg(windows)]
	NamedPipe(NamedPipeAdapter),
}

impl Adapter for LocalAdapter {
	fn split(self: Box<Self>) -> (DynRead, DynWrite) {
		match *self {
			#[cfg(unix)]
			LocalAdapter::Unix(a) => Box::new(a).split(),
			#[cfg(windows)]
			LocalAdapter::NamedPipe(a) => Box::new(a).split(),
		}
	}
}

pub async fn connect_kern(endpoint: &Endpoint) -> Result<LocalAdapter, AdapterError> {
	match endpoint {
		#[cfg(unix)]
		Endpoint::Unix(path) => Ok(LocalAdapter::Unix(UnixStreamAdapter::connect(path).await?)),
		#[cfg(windows)]
		Endpoint::NamedPipe(name) => Ok(LocalAdapter::NamedPipe(
			NamedPipeAdapter::connect(name).await?,
		)),
	}
}

pub enum BindOutcome {
	Bound(LocalListener),
	AlreadyRunning,
}

#[derive(Debug, thiserror::Error)]
pub enum BindError {
	#[error("bind: {0}")]
	Io(#[from] std::io::Error),
}

// The socket serves unauthenticated graph reads AND mutations, so it must be
// owner-only. chmod-after-bind leaves a sub-ms window at the umask default
// (0755); the alternative is flipping the process-global umask, which in a
// multi-threaded daemon would race every unrelated file created concurrently.
#[cfg(unix)]
fn harden_socket(path: &Path) -> std::io::Result<()> {
	use std::os::unix::fs::PermissionsExt;
	std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
}

pub async fn bind_kern_listener(endpoint: &Endpoint) -> Result<BindOutcome, BindError> {
	match endpoint {
		#[cfg(unix)]
		Endpoint::Unix(path) => {
			let listener = match tokio::net::UnixListener::bind(path) {
				Ok(listener) => listener,
				Err(e) if e.kind() != std::io::ErrorKind::AddrInUse => {
					return Err(e.into());
				}
				Err(_) => {
					if tokio::net::UnixStream::connect(path).await.is_ok() {
						return Ok(BindOutcome::AlreadyRunning);
					}
					let _ = std::fs::remove_file(path);
					tokio::net::UnixListener::bind(path)?
				}
			};
			harden_socket(path)?;
			Ok(BindOutcome::Bound(LocalListener {
				inner: listener,
				socket_path: path.clone(),
			}))
		}
		#[cfg(windows)]
		Endpoint::NamedPipe(name) => {
			use tokio::net::windows::named_pipe::ServerOptions;
			match ServerOptions::new()
                .first_pipe_instance(true)
                .create(name)
            {
                Ok(server) => Ok(BindOutcome::Bound(LocalListener {
                    pipe_name: name.clone(),
                    current: Some(server),
                })),
                Err(e)
                    if e.kind() == std::io::ErrorKind::PermissionDenied
                        || e.raw_os_error() == Some(5)    // ERROR_ACCESS_DENIED
                        || e.raw_os_error() == Some(231)  // ERROR_PIPE_BUSY
                =>
                {
                    Ok(BindOutcome::AlreadyRunning)
                }
                Err(e) => Err(e.into()),
            }
		}
	}
}

pub struct LocalListener {
	#[cfg(unix)]
	inner: tokio::net::UnixListener,
	#[cfg(unix)]
	socket_path: PathBuf,
	#[cfg(windows)]
	pipe_name: String,
	#[cfg(windows)]
	current: Option<tokio::net::windows::named_pipe::NamedPipeServer>,
}

impl LocalListener {
	pub async fn accept(&mut self) -> Result<LocalAdapter, std::io::Error> {
		#[cfg(unix)]
		{
			let (stream, _peer) = self.inner.accept().await?;
			Ok(LocalAdapter::Unix(UnixStreamAdapter::new(stream)))
		}
		#[cfg(windows)]
		{
			let server = self.current.take().expect("listener uninitialised");
			server.connect().await?;
			// Pre-create the next instance so subsequent accept doesn't race
			// a fast reconnect.
			self.current =
				Some(tokio::net::windows::named_pipe::ServerOptions::new().create(&self.pipe_name)?);
			Ok(LocalAdapter::NamedPipe(NamedPipeAdapter::from_server(
				server,
			)))
		}
	}
}

#[cfg(unix)]
impl Drop for LocalListener {
	fn drop(&mut self) {
		// Best-effort cleanup so the next daemon doesn't trip the stale-sock probe.
		let _ = std::fs::remove_file(&self.socket_path);
	}
}

#[cfg(all(test, windows))]
mod bind_tests_windows {
	use super::*;

	#[tokio::test]
	async fn a_second_bind_of_the_same_pipe_reports_already_running() {
		let ep = Endpoint::NamedPipe(format!(r"\\.\pipe\kern-bindtest-{}", std::process::id()));
		let first = bind_kern_listener(&ep).await.unwrap();
		assert!(
			matches!(first, BindOutcome::Bound(_)),
			"first bind owns the pipe"
		);
		let second = bind_kern_listener(&ep).await.unwrap();
		assert!(
			matches!(second, BindOutcome::AlreadyRunning),
			"second bind sees AlreadyRunning"
		);
		drop(first); // keep the first instance alive until the assertion above
	}
}

#[cfg(all(test, unix))]
mod bind_tests_unix {
	use super::*;

	#[tokio::test]
	async fn a_live_owner_reports_already_running() {
		let dir = tempfile::tempdir().unwrap();
		let ep = Endpoint::Unix(dir.path().join("kern.sock"));
		let first = bind_kern_listener(&ep).await.unwrap();
		let BindOutcome::Bound(_listener) = first else {
			panic!("first bind should own the socket")
		};
		let second = bind_kern_listener(&ep).await.unwrap();
		assert!(
			matches!(second, BindOutcome::AlreadyRunning),
			"a live owner -> AlreadyRunning"
		);
	}

	#[tokio::test]
	async fn a_bound_socket_is_owner_only() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("kern.sock");
		let ep = Endpoint::Unix(path.clone());
		let BindOutcome::Bound(_listener) = bind_kern_listener(&ep).await.unwrap() else {
			panic!("first bind should own the socket")
		};
		let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
		assert_eq!(mode, 0o600, "socket must be owner-only, got {mode:o}");
	}

	#[tokio::test]
	async fn a_rebound_stale_socket_is_also_owner_only() {
		use std::os::unix::fs::PermissionsExt;
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("kern.sock");
		{
			let _l = tokio::net::UnixListener::bind(&path).unwrap();
		}
		let ep = Endpoint::Unix(path.clone());
		let BindOutcome::Bound(_listener) = bind_kern_listener(&ep).await.unwrap() else {
			panic!("stale file should be removed and rebound")
		};
		let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
		assert_eq!(
			mode, 0o600,
			"the stale-rebind path hardens too, got {mode:o}"
		);
	}

	#[tokio::test]
	async fn a_stale_socket_file_is_removed_and_rebound() {
		let dir = tempfile::tempdir().unwrap();
		let path = dir.path().join("kern.sock");
		{
			let _l = tokio::net::UnixListener::bind(&path).unwrap();
		}
		assert!(
			path.exists(),
			"stale socket file remains after the listener drops"
		);
		let ep = Endpoint::Unix(path);
		let outcome = bind_kern_listener(&ep).await.unwrap();
		assert!(
			matches!(outcome, BindOutcome::Bound(_)),
			"stale file removed, endpoint rebound"
		);
	}
}
