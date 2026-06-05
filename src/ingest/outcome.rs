#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutcomeStatus {
	Committed,
	Partial,
	Failed,
}

impl OutcomeStatus {
	pub fn as_str(&self) -> &'static str {
		match self {
			Self::Committed => "committed",
			Self::Partial => "partial",
			Self::Failed => "failed",
		}
	}
}

#[derive(Debug, Clone)]
pub struct FailureReport {
	pub scope: String,
	pub chunk_index: usize,
	pub class: String,
	pub error: String,
}

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
