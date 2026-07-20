use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ShutdownRes {
	pub ok: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct HealthRes {
	pub ok: bool,
	#[serde(default)]
	pub data_dir: String,
	#[serde(default)]
	pub kerns: u64,
	#[serde(default)]
	pub entities: u64,
	// Ms since the last real tool call (health polls excluded). 0 from older
	// daemons that predate the field — the hub treats that as "never idle".
	#[serde(default)]
	pub idle_ms: u64,
	#[serde(default)]
	pub queue_depth: u64,
	#[serde(default)]
	pub tasks_done: u64,
	// Lifetime mean over `tasks_done`, not a recent window: it converges and
	// stops moving, so read it as a baseline, never as current load.
	#[serde(default)]
	pub task_avg_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolReq {
	pub name: String,
	#[serde(default)]
	pub args: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CallToolRes {
	pub envelope: serde_json::Value,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListToolsReq {}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ListToolsRes {
	pub tools: Vec<serde_json::Value>,
}

#[cfg(test)]
mod dto_serde_tests {
	use super::*;

	#[test]
	fn an_older_health_payload_without_queue_fields_still_deserializes() {
		let old = r#"{"ok":true,"data_dir":"/d","kerns":3,"entities":7,"idle_ms":42}"#;
		let h: HealthRes = serde_json::from_str(old).expect("append-only: old shape must decode");
		assert_eq!(h.kerns, 3);
		assert_eq!(h.idle_ms, 42);
		assert_eq!(h.queue_depth, 0, "absent field defaults, never errors");
		assert_eq!(h.tasks_done, 0);
		assert_eq!(h.task_avg_ms, 0);

		let ancient = r#"{"ok":true}"#;
		let h2: HealthRes = serde_json::from_str(ancient).expect("only `ok` is required");
		assert!(h2.ok);
		assert_eq!(h2.task_avg_ms, 0);
	}
}
