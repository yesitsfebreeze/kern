//! `tracing-subscriber` layer that forwards every event into the shared
//! journal as a `Kind::Log` entry. Install in every binary that uses
//! `tracing::*` macros so logs end up alongside RPC + turn events.

use std::fmt;

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::layer::{Context, Layer};

use crate::{Entry, Kind};

pub struct JournalTracingLayer {
	pub source: &'static str,
}

impl JournalTracingLayer {
	pub fn new(source: &'static str) -> Self {
		Self { source }
}
}

impl<S: Subscriber> Layer<S> for JournalTracingLayer {
	fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
		crate::emit(event_to_entry(self.source, event));
	}
}

/// Build the `Kind::Log` journal entry for a tracing event: capture its fields
/// via [`FieldRecorder`] and assemble the `{src,target,level,msg}` payload. Split
/// out of `on_event` (which only adds the global emit) so the entry shape is
/// unit-testable without standing up the process-global journal.
fn event_to_entry(source: &str, event: &Event<'_>) -> Entry {
	let meta = event.metadata();
	let mut v = FieldRecorder::default();
	event.record(&mut v);
	let payload = serde_json::json!({
		"src": source,
		"target": meta.target(),
		"level": meta.level().to_string(),
		"msg": v.finish(),
	});
	Entry::new(Kind::Log, meta.target(), payload)
}

/// Generic `tracing` field visitor that captures the `message` field (the
/// most common case) and falls back to the first non-message field's
/// `name=value` rendering. Shared by [`JournalTracingLayer`] and any other
/// tracing layer that wants the same single-line summary semantics.
#[derive(Default)]
pub struct FieldRecorder {
	pub message: Option<String>,
	pub fallback: Option<String>,
}

impl FieldRecorder {
	/// Consume the recorder and return `message` if present, else `fallback`,
	/// else an empty string.
	pub fn finish(self) -> String {
		self.message.or(self.fallback).unwrap_or_default()
	}
}

impl Visit for FieldRecorder {
	fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
		if field.name() == "message" {
			// The message usually Debug-renders unquoted (format_args!); when it does
			// arrive quoted (a Debug'd &str) strip exactly the one outer quote pair,
			// not trim_matches('"') which would also eat a quote that is legitimately
			// the first/last character of the message.
			let raw = format!("{value:?}");
			let msg = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')).unwrap_or(&raw);
			self.message = Some(msg.to_string());
		} else if self.fallback.is_none() {
			self.fallback = Some(format!("{}={:?}", field.name(), value));
		}
}
	fn record_str(&mut self, field: &Field, value: &str) {
		if field.name() == "message" {
			self.message = Some(value.to_string());
		} else if self.fallback.is_none() {
			self.fallback = Some(format!("{}={}", field.name(), value));
		}
}
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::{Arc, Mutex};
	use tracing_subscriber::layer::SubscriberExt;

	/// Test layer that runs the real `event_to_entry` over each event and stores
	/// the resulting Entry — so we drive actual `tracing::*` events through the
	/// production field-capture + payload path without the global journal.
	#[derive(Clone, Default)]
	struct CaptureLayer(Arc<Mutex<Vec<Entry>>>);
	impl<S: Subscriber> Layer<S> for CaptureLayer {
		fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
			self.0.lock().unwrap().push(event_to_entry("test-src", event));
		}
	}

	fn capture(f: impl FnOnce()) -> Vec<Entry> {
		let cap = CaptureLayer::default();
		let sub = tracing_subscriber::registry().with(cap.clone());
		tracing::subscriber::with_default(sub, f);
		let entries = cap.0.lock().unwrap().clone();
		entries
	}

	#[test]
	fn info_event_becomes_a_kind_log_entry_with_message() {
		let entries = capture(|| tracing::info!("hello world"));
		assert_eq!(entries.len(), 1);
		let e = &entries[0];
		assert!(matches!(e.kind, Kind::Log), "a tracing event maps to Kind::Log");
		assert_eq!(e.payload["msg"], "hello world", "message recorded unquoted");
		assert_eq!(e.payload["level"], "INFO");
		assert_eq!(e.payload["src"], "test-src");
	}

	#[test]
	fn message_field_wins_over_other_fields() {
		let entries = capture(|| tracing::warn!(user = "kern", "did {}", 3));
		assert_eq!(entries[0].payload["msg"], "did 3", "message beats the user field");
		assert_eq!(entries[0].payload["level"], "WARN");
	}

	#[test]
	fn first_non_message_field_is_the_fallback() {
		// No message -> the first recorded field renders as `name=value` (record_debug
		// fallback for the integer field).
		let entries = capture(|| tracing::info!(answer = 42));
		assert_eq!(entries[0].payload["msg"], "answer=42");
	}

	#[test]
	fn finish_prefers_message_then_fallback_then_empty() {
		assert_eq!(FieldRecorder::default().finish(), "", "empty -> empty string");

		let mut only_fallback = FieldRecorder::default();
		only_fallback.fallback = Some("k=v".into());
		assert_eq!(only_fallback.finish(), "k=v", "no message -> fallback");

		let both = FieldRecorder { message: Some("m".into()), fallback: Some("k=v".into()) };
		assert_eq!(both.finish(), "m", "message beats fallback");
	}
}
