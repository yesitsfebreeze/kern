use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const MAX_ENTRIES: usize = 1024;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Level {
	Info,
	Warn,
	Error,
}

impl Level {
	pub fn tag(self) -> &'static str {
		match self {
			Self::Info => "INF",
			Self::Warn => "WRN",
			Self::Error => "ERR",
		}
	}
}

#[derive(Clone, Debug)]
pub struct Entry {
	pub level: Level,
	pub source: String,
	pub message: String,
	pub when_ms: u64,
}

#[derive(Clone)]
pub struct Sink {
	inner: Arc<Mutex<VecDeque<Entry>>>,
}

impl Default for Sink {
	fn default() -> Self {
		Self {
			inner: Arc::new(Mutex::new(VecDeque::with_capacity(MAX_ENTRIES))),
		}
	}
}

impl Sink {
	pub fn new() -> Self {
		Self::default()
	}

	pub fn push(&self, entry: Entry) {
		let Ok(mut g) = self.inner.lock() else {
			return;
		};
		if g.len() == MAX_ENTRIES {
			g.pop_front();
		}
		g.push_back(entry);
	}

	pub fn snapshot(&self) -> Vec<Entry> {
		match self.inner.lock() {
			Ok(g) => g.iter().cloned().collect(),
			Err(_) => Vec::new(),
		}
	}

	pub fn clear(&self) {
		if let Ok(mut g) = self.inner.lock() {
			g.clear();
		}
	}
}

static SINK: OnceLock<Sink> = OnceLock::new();

pub fn install_sink(sink: Sink) -> Result<(), Sink> {
	SINK.set(sink)
}

pub fn sink() -> Option<&'static Sink> {
	SINK.get()
}

pub fn log(level: Level, source: &str, message: impl Into<String>) {
	let message = message.into();
	if let Some(s) = SINK.get() {
		// Sink path: move `message` straight into the Entry — no clone. The
		// previous unconditional `message.clone()` only existed to keep a copy
		// for the eprintln fallback, which the else branch borrows instead.
		s.push(Entry {
			level,
			source: source.to_string(),
			message,
			when_ms: now_ms(),
		});
	} else {
		eprintln!("[{}][{}] {}", level.tag(), source, message);
	}
}

fn now_ms() -> u64 {
	SystemTime::now()
		.duration_since(UNIX_EPOCH)
		.map(|d| d.as_millis() as u64)
		.unwrap_or(0)
}

/// Log at an explicit [`Level`]. Named `klog!` (not `log!`) so it never shadows
/// the ubiquitous `log::log!` macro from the `log` crate in downstream code that
/// imports both. The level-specific [`info!`] / [`warn!`] / [`error!`] macros
/// are the usual entry points; reach for `klog!` only when the level is dynamic.
#[macro_export]
macro_rules! klog {
	($level:expr, $source:expr, $($arg:tt)+) => {{
		$crate::log($level, $source, format!($($arg)+));
	}};
}

#[macro_export]
macro_rules! info {
	($source:expr, $($arg:tt)+) => {
		$crate::log($crate::Level::Info, $source, format!($($arg)+))
	};
}

#[macro_export]
macro_rules! warn {
	($source:expr, $($arg:tt)+) => {
		$crate::log($crate::Level::Warn, $source, format!($($arg)+))
	};
}

#[macro_export]
macro_rules! error {
	($source:expr, $($arg:tt)+) => {
		$crate::log($crate::Level::Error, $source, format!($($arg)+))
	};
}

#[cfg(test)]
mod tests;
