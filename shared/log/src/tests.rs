use crate::*;

#[test]
fn sink_ring_caps_and_evicts_oldest() {
	let s = Sink::new();
	for i in 0..(MAX_ENTRIES + 10) {
		s.push(Entry {
			level: Level::Info,
			source: "t".into(),
			message: format!("{i}"),
			when_ms: i as u64,
		});
	}
	let snap = s.snapshot();
	assert_eq!(snap.len(), MAX_ENTRIES);
	assert_eq!(snap.first().unwrap().message, "10");
	assert_eq!(snap.last().unwrap().message, format!("{}", MAX_ENTRIES + 9));
}

#[test]
fn sink_clear_empties_ring() {
	let s = Sink::new();
	s.push(Entry {
		level: Level::Warn,
		source: "t".into(),
		message: "x".into(),
		when_ms: 0,
	});
	s.clear();
	assert!(s.snapshot().is_empty());
}

#[test]
fn level_tag_is_three_letter_code_per_variant() {
	assert_eq!(Level::Info.tag(), "INF");
	assert_eq!(Level::Warn.tag(), "WRN");
	assert_eq!(Level::Error.tag(), "ERR");
}

#[test]
fn global_sink_installs_once_then_routes_log() {
	// This is the ONLY test that touches the process-global SINK OnceLock; the
	// others use a local `Sink::new()`. That keeps this assertion deterministic
	// across the shared test process: no other test can have set it first, so
	// the pre-install state is observably the eprintln-fallback branch.
	assert!(sink().is_none(), "no global sink before first install");

	let s = Sink::new();
	assert!(install_sink(s.clone()).is_ok(), "first install succeeds");

	// Second install is rejected (OnceLock::set returns Err with the sink back).
	assert!(install_sink(Sink::new()).is_err(), "double install errors");

	// With a sink installed, log() routes into it instead of eprintln. Drive one
	// call through the renamed `klog!` macro (format args) and one direct.
	crate::klog!(Level::Warn, "unit", "n={}", 7);
	log(Level::Error, "unit", "boom");
	let snap = s.snapshot();
	assert_eq!(snap.len(), 2, "both logs routed to the installed sink");
	assert_eq!(snap[0].level, Level::Warn);
	assert_eq!(snap[0].message, "n=7", "klog! formats its args");
	assert_eq!(snap[1].level, Level::Error);
	assert_eq!(snap[1].source, "unit");
	assert_eq!(snap[1].message, "boom");
}

#[test]
fn concurrent_pushes_are_thread_safe_and_stay_capped() {
	use std::thread;
	// `Sink` is `Clone` (shared `Arc<Mutex<..>>`), so every thread pushes into
	// the same ring. 8*500 = 4000 pushes >> MAX_ENTRIES exercises eviction under
	// contention; the assertion is that we neither panic/deadlock nor exceed cap.
	let s = Sink::new();
	let handles: Vec<_> = (0..8u8)
		.map(|t| {
			let s = s.clone();
			thread::spawn(move || {
				for i in 0..500u32 {
					s.push(Entry {
						level: Level::Info,
						source: "c".into(),
						message: format!("{t}-{i}"),
						when_ms: 0,
					});
				}
			})
		})
		.collect();
	for h in handles {
		h.join().unwrap();
	}
	assert_eq!(s.snapshot().len(), MAX_ENTRIES, "ring stays capped under contention");
}
