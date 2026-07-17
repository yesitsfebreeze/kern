# src/tick/cluster.rs — commentary

- `vector_cluster`: the fixed seed centroid (vs an evolving mean) is a deliberate speed/simplicity trade for the tick path; a k-means-style evolving centroid would be more balanced but multi-pass.

## Second-pass migration:
- `anchor_prompt` sampling (moved from inline): for clusters larger than `MAX_SAMPLES` only the thoughts nearest the centroid are included — the most representative members, which also bounds prompt-eval cost. The instruction demands ONLY the phrase (no prefix, no trailing punctuation) so the reply is usable verbatim as anchor text; sampled thoughts follow as a `- ` list.
