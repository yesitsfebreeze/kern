use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Instant;

use crate::base::graph::GraphGnn;
use crate::base::locks::{lock_recovered, read_recovered, write_recovered};
use crate::config::Config;
use crate::ingest::{LlmFunc as IngestLlmFunc, Worker};
use crate::llm::Client as LlmClient;
use crate::tick::queue::Queue;
use crate::tick::tasks::{BroadcastQuestionFunc, EmbedFunc, LlmFunc as TickLlmFunc};

pub type StoreKey = PathBuf;

pub struct StoreEntry {
    pub key: StoreKey,
    pub graph: Arc<RwLock<GraphGnn>>,
    pub worker: Arc<Worker>,
    pub tick_q: Arc<Queue>,
    pub tick_handle: tokio::task::JoinHandle<()>,
    /// Persists this store's graph. Single instance per store — Worker
    /// holds a clone of the same Arc; downstream consumers (mcp::Server,
    /// gossip handlers) should obtain it from `StoreEntry.save_fn.clone()`
    /// rather than building a duplicate closure over the same graph.
    pub save_fn: Arc<dyn Fn() + Send + Sync>,
    pub last_touch: RwLock<Instant>,
}

#[derive(Default)]
pub struct Registry {
    stores: RwLock<HashMap<StoreKey, Arc<StoreEntry>>>,
    /// Per-key build locks. Serialize concurrent `open()`s of the SAME data dir
    /// so only one caller constructs the (expensive) graph + worker + tick task;
    /// different keys still build in parallel. Without this, two racers both
    /// build, and the one that loses the final insert is dropped — detaching its
    /// already-spawned worker/tick tasks onto an orphaned graph.
    builds: Mutex<HashMap<StoreKey, Arc<Mutex<()>>>>,
}

impl Registry {
    pub fn new() -> Self { Self::default() }

    fn canon(p: &Path) -> StoreKey {
        std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
    }

    pub fn get(&self, data_dir: &Path) -> Option<Arc<StoreEntry>> {
        read_recovered(&self.stores).get(&Self::canon(data_dir)).cloned()
    }

    pub fn len(&self) -> usize { read_recovered(&self.stores).len() }

    pub fn is_empty(&self) -> bool { read_recovered(&self.stores).is_empty() }

    #[allow(clippy::too_many_arguments)]
    pub fn open(
        &self,
        data_dir: &Path,
        cfg: &Config,
        llm_client: LlmClient,
        ingest_llm: Option<IngestLlmFunc>,
        tick_llm: Option<TickLlmFunc>,
        tick_embed: Option<EmbedFunc>,
        broadcast_q: Option<BroadcastQuestionFunc>,
    ) -> Arc<StoreEntry> {
        let key = Self::canon(data_dir);
        if let Some(e) = read_recovered(&self.stores).get(&key) {
            *write_recovered(&e.last_touch) = Instant::now();
            return e.clone();
        }

        // Hold this key's build lock for the whole construction below. A second
        // caller for the same dir blocks here, then falls into the re-check and
        // returns the entry we insert — instead of redundantly building its own.
        let build_lock = lock_recovered(&self.builds).entry(key.clone()).or_default().clone();
        let _build = lock_recovered(&build_lock);

        // Re-check under the build lock: a prior builder for this key may have
        // finished and inserted while we were waiting on the lock above.
        if let Some(e) = read_recovered(&self.stores).get(&key) {
            *write_recovered(&e.last_touch) = Instant::now();
            return e.clone();
        }

        let mut store_cfg = cfg.clone();
        store_cfg.data_dir = data_dir.to_string_lossy().into_owned();

        let graph = Arc::new(RwLock::new(crate::commands::load_graph(&store_cfg)));

        // The ONE persist closure for this store's graph. The Worker gets a clone
        // and the StoreEntry holds another (StoreEntry.save_fn), so every consumer
        // — mcp::Server, gossip handlers — persists through this same closure
        // rather than building a duplicate over the same graph.
        let save_g = graph.clone();
        let save_fn: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let g = read_recovered(&save_g);
            crate::commands::save_graph(&g);
        });

        let worker = Arc::new(Worker::new(
            graph.clone(),
            llm_client,
            ingest_llm,
            Some(save_fn.clone()),
        ));

        let tick_q = Arc::new(Queue::new(cfg.tick.queue_capacity.max(1)));

        let tick_handle = crate::tick::start(
            tick_q.clone(),
            graph.clone(),
            crate::tick::TickContext {
                llm: tick_llm,
                embed: tick_embed,
                broadcast_q,
                gnn_cfg: cfg.gnn.into(),
                tick_cfg: cfg.tick,
            },
        );

        crate::tick::enqueue_all(&tick_q, &graph);

        let entry = Arc::new(StoreEntry {
            key: key.clone(),
            graph,
            worker,
            tick_q,
            tick_handle,
            save_fn,
            last_touch: RwLock::new(Instant::now()),
        });

        write_recovered(&self.stores)
            .entry(key)
            .or_insert_with(|| entry.clone())
            .clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn dead_client() -> LlmClient {
        LlmClient::new_embed_only("http://127.0.0.1:1", "test")
    }

    #[tokio::test]
    async fn open_dedups_and_touches_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Registry::new();
        let cfg = Config::default();

        let a = reg.open(dir.path(), &cfg, dead_client(), None, None, None, None);
        let first_touch = *read_recovered(&a.last_touch);

        tokio::time::sleep(Duration::from_millis(2)).await;

        let b = reg.open(dir.path(), &cfg, dead_client(), None, None, None, None);
        assert!(Arc::ptr_eq(&a, &b), "re-open of the same dir returns the same StoreEntry");
        assert_eq!(reg.len(), 1, "no duplicate store registered");
        assert!(
            *read_recovered(&b.last_touch) > first_touch,
            "last_touch advanced on re-open",
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_open_of_same_dir_yields_one_store() {
        let dir = tempfile::tempdir().unwrap();
        let reg = Arc::new(Registry::new());
        let cfg = Config::default();
        let path = dir.path().to_path_buf();

        let mut handles = Vec::new();
        for _ in 0..4 {
            let reg = reg.clone();
            let cfg = cfg.clone();
            let path = path.clone();
            handles.push(tokio::spawn(async move {
                reg.open(&path, &cfg, dead_client(), None, None, None, None)
            }));
        }
        let mut entries = Vec::new();
        for h in handles {
            entries.push(h.await.unwrap());
        }

        // The per-key build lock means every racer ends up with the SAME entry and
        // exactly one store is registered (no duplicate build wins).
        for e in &entries[1..] {
            assert!(Arc::ptr_eq(&entries[0], e), "all concurrent opens share one StoreEntry");
        }
        assert_eq!(reg.len(), 1, "exactly one store registered despite the race");
    }
}
