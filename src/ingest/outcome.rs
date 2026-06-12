#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
	Committed,
	Partial,
	/// The document matched an existing entity (cosine ≥ dedup threshold) and
	/// was MERGED into it instead of placed fresh: the acked content-hash
	/// `doc_id` never enters the graph — the surviving id is the existing
	/// entity's. Distinct from `Committed` so a merge is distinguishable from
	/// silent loss at every call site (in-memory only; never persisted).
	Deduped,
	Failed,
}

impl OutcomeStatus {
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Committed => "committed",
			Self::Partial => "partial",
			Self::Deduped => "deduped",
			Self::Failed => "failed",
		}
	}
}

/// A single chunk- or document-scope failure recorded during ingest.
///
/// `class` is `"permanent"` or `"transient"`: a **permanent** failure will not
/// succeed on retry (bad model/400, non-UTF-8 content, poisoned lock) and is
/// final; a **transient** failure (timeout, 5xx, 429, connect error) is
/// retryable and may yet commit on a later sweep. `scope` is `"document"` or
/// `"chunk"`; `chunk_index` is `0` for document-scope failures. `error` is the
/// human-readable cause.
#[derive(Debug, Clone)]
pub struct FailureReport {
	pub scope: String,
	pub chunk_index: usize,
	pub class: String,
	pub error: String,
}

impl FailureReport {
	/// A permanent, document-scope failure (`chunk_index = 0`) — the shared shape
	/// for document-level errors: a poisoned graph lock, a failed document
	/// embedding, or an enqueue/send failure. Keeps the four-field literal in one
	/// place so the scope/class strings can't drift between call sites.
	pub fn document_permanent(error: impl Into<String>) -> Self {
		Self {
			scope: "document".into(),
			chunk_index: 0,
			class: "permanent".into(),
			error: error.into(),
		}
	}
}

/// The terminal result of an ingest call.
///
/// `total_chunks` were attempted, `embedded_chunks` committed, `failed_chunks`
/// did not. The `failures` are partitioned by retryability into
/// `transient_failures` vs `permanent_failures` (counts that sum to
/// `failures.len()`): transient failures are why a `Partial`/`Failed` outcome
/// might still recover on a later retry sweep, while permanent ones never will.
/// `message` is a human-readable summary; `status` is the coarse verdict.
#[derive(Debug, Clone)]
pub struct Outcome {
	pub status: OutcomeStatus,
	pub doc_id: String,
	pub total_chunks: usize,
	pub embedded_chunks: usize,
	pub failed_chunks: usize,
	pub transient_failures: usize,
	pub permanent_failures: usize,
	pub failures: Vec<FailureReport>,
	pub message: String,
}

impl Outcome {
	/// A wholly-failed outcome: no document committed, all chunk counters
	/// zero. Used for enqueue/dispatch failures where nothing was processed.
	pub fn failed(message: impl Into<String>, failures: Vec<FailureReport>) -> Self {
		Self {
			status: OutcomeStatus::Failed,
			doc_id: String::new(),
			total_chunks: 0,
			embedded_chunks: 0,
			failed_chunks: 0,
			transient_failures: 0,
			permanent_failures: 0,
			failures,
			message: message.into(),
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn status_as_str_is_the_lowercase_label() {
		assert_eq!(OutcomeStatus::Committed.as_str(), "committed");
		assert_eq!(OutcomeStatus::Partial.as_str(), "partial");
		assert_eq!(OutcomeStatus::Failed.as_str(), "failed");
		// Dedup-merge must be distinguishable from a fresh commit: the caller's
		// acked content-hash doc_id does NOT exist in the graph after a merge
		// (the surviving id is the existing entity's), so reporting it as a
		// plain "committed" made merges indistinguishable from silent loss.
		assert_eq!(OutcomeStatus::Deduped.as_str(), "deduped");
	}

	#[test]
	fn failed_zeroes_every_counter_and_carries_the_failures() {
		let f = FailureReport::document_permanent("boom");
		let o = Outcome::failed("nothing processed", vec![f]);
		assert_eq!(o.status, OutcomeStatus::Failed);
		assert!(o.doc_id.is_empty(), "no document committed");
		assert_eq!(
			(o.total_chunks, o.embedded_chunks, o.failed_chunks, o.transient_failures, o.permanent_failures),
			(0, 0, 0, 0, 0),
			"all counters zero on a wholly-failed outcome"
		);
		assert_eq!(o.failures.len(), 1, "failure detail preserved");
		assert_eq!(o.message, "nothing processed");
	}

	#[test]
	fn document_permanent_has_the_canonical_shape() {
		let f = FailureReport::document_permanent("lock poisoned");
		assert_eq!(f.scope, "document");
		assert_eq!(f.chunk_index, 0);
		assert_eq!(f.class, "permanent");
		assert_eq!(f.error, "lock poisoned");
	}
}
