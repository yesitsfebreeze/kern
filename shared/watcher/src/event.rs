use std::path::PathBuf;
use std::time::SystemTime;

/// Kind of filesystem change.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum WatchKind {
	Created,
	Modified,
	Deleted,
	Renamed { from: PathBuf, to: PathBuf },
}

/// Single coalesced filesystem event emitted by [`crate::FileWatcher`].
///
/// `path` is the canonical path the event concerns. For `Renamed`, `path`
/// equals `to` (the new location); the rename payload also carries `from`.
/// Build via [`WatchEvent::new`] so that invariant is enforced rather than left
/// to each caller. Derives `Hash` so events can be used as `HashMap`/`HashSet`
/// keys (e.g. for dedup) without downstream boilerplate.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct WatchEvent {
	pub path: PathBuf,
	pub kind: WatchKind,
	pub ts: SystemTime,
}

impl WatchEvent {
	/// Construct an event, enforcing the `Renamed` invariant: `path` is always the
	/// NEW location (`to`). For other kinds `path` is used as given. Centralising
	/// it here means a caller can't accidentally emit a `Renamed` whose `path`
	/// points at the old location.
	pub fn new(path: PathBuf, kind: WatchKind, ts: SystemTime) -> Self {
		let path = match &kind {
			WatchKind::Renamed { to, .. } => to.clone(),
			_ => path,
		};
		Self { path, kind, ts }
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn renamed_event_path_is_forced_to_the_new_location() {
		// Even when the caller passes the OLD path, `new` overrides it with `to`,
		// pinning the documented `path == to` invariant in code.
		let ev = WatchEvent::new(
			PathBuf::from("/old.txt"),
			WatchKind::Renamed { from: "/old.txt".into(), to: "/new.txt".into() },
			SystemTime::UNIX_EPOCH,
		);
		assert_eq!(ev.path, PathBuf::from("/new.txt"), "Renamed path is the new location");
		match ev.kind {
			WatchKind::Renamed { from, to } => {
				assert_eq!(from, PathBuf::from("/old.txt"));
				assert_eq!(to, PathBuf::from("/new.txt"));
			}
			other => panic!("kind preserved, got {other:?}"),
		}
	}

	#[test]
	fn non_renamed_event_keeps_its_given_path() {
		let ev = WatchEvent::new(PathBuf::from("/a.rs"), WatchKind::Modified, SystemTime::UNIX_EPOCH);
		assert_eq!(ev.path, PathBuf::from("/a.rs"));
	}

	#[test]
	fn watch_event_works_as_a_hash_set_key() {
		use std::collections::HashSet;
		let a = WatchEvent::new(PathBuf::from("/a"), WatchKind::Created, SystemTime::UNIX_EPOCH);
		let mut set = HashSet::new();
		set.insert(a.clone());
		assert!(set.contains(&a), "Hash derive lets WatchEvent be a set/map key");
	}
}
