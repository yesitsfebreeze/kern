use crate::base::graph::GraphGnn;
use crate::ingest::Config;

pub type RephraseCandidate = (String, String, f32);

pub fn find_rephrase_candidates(graph: &GraphGnn, cfg: &Config) -> Vec<RephraseCandidate> {
	let mut seen = std::collections::HashSet::<(String, String)>::new();
	let mut out = Vec::new();

	for kern in graph.map().values() {
		for t in kern.entities.values() {
			if t.vector.is_empty() {
				continue;
			}
			let hits = graph.entity_idx.search(&t.vector, cfg.hnsw_k, cfg.hnsw_ef);
			for h in hits {
				if h.id == t.id {
					continue;
				}
				let sim = h.score;
				if sim <= cfg.rephrase_lower || sim >= cfg.rephrase_upper {
					continue;
				}
				let (a, b) = if t.id < h.id {
					(t.id.clone(), h.id.clone())
				} else {
					(h.id.clone(), t.id.clone())
				};
				if seen.insert((a.clone(), b.clone())) {
					out.push((a, b, sim as f32));
				}
			}
		}
	}

	out
}
