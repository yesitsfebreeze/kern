use std::sync::Arc;

use kern::config::Config;
use kern::llm::{Client, Endpoint};
use kern::store::Registry;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registry_idempotent_and_isolated() {
	let dir_a = tempfile::tempdir().unwrap();
	let dir_b = tempfile::tempdir().unwrap();

	let cfg = Config::default();
	let llm = Client::new(
		Endpoint::default(),
		Endpoint::default(),
		Endpoint::default(),
	);

	let reg = Registry::new();

	let a1 = reg.open(dir_a.path(), &cfg, llm.clone(), None, None, None);
	let a2 = reg.open(dir_a.path(), &cfg, llm.clone(), None, None, None);
	let b = reg.open(dir_b.path(), &cfg, llm.clone(), None, None, None);

	assert_eq!(reg.len(), 2);
	assert!(Arc::ptr_eq(&a1, &a2));
	assert!(!Arc::ptr_eq(&a1.graph, &b.graph));
	assert_ne!(a1.key, b.key);
}
