# Binary (1-bit) quantization for the in-memory ANN index (design)

**Status:** design / approved-for-planning.

**One line:** Add a `Binary` quantization mode that stores each embedding
dimension as a single sign bit and uses **Hamming distance for candidate
generation**, then **rescores the shortlist with the retained f64 vector** — a
~64× smaller in-memory index vector and faster traversal, validated to hold
recall@k like int8 already is.

---

## Problem & evidence

`aspiration.md` **Stage B1** ("the quick-win bundle") and the Tier-3 row both
call for **binary quant / TurboQuant (1-bit) + rescoring**: *"append `Binary` to
`QuantizationMode` + `b:Vec<u8>` to `QuantizedVec` (bincode-APPEND only). 1-bit
hamming for candidate gen, then rescore with the retained f64 `HnswNode.vec`
(kept even when quantized). 64× smaller, faster traversal. Validate recall@k like
int8."* Today only `None` and `Int8` exist (`src/quant.rs`).

### Data-safety keystone (verified this session — the reason this is tractable)

`QuantizedVec` is **in-memory only**. Verified by grep: it appears solely in
`src/quant.rs` and `src/base/hnsw.rs` (as `HnswNode.qvec` / the `Query` enum) —
**no persisted or wire struct references it**, and `QuantizedVec::dim()` has a
single (test) caller. The on-disk projection is the separate `StoredVec`
(`store.rs:104`), which is **always int8** and documents *why* `QuantizedVec` is
never persisted (its `skip_serializing_if` fields are a bincode positional trap).

**Consequences:**
1. Adding `Binary = 2` to the `#[repr(u8)]` enum is append-only and **changes no
   on-disk bytes** (discriminants `None=0`, `Int8=1` untouched).
2. Adding a `b: Vec<u8>` field to `QuantizedVec` is **data-safe** — the struct is
   never serialized to disk or the wire; it is rebuilt from f64 on load.
3. Binary quant is therefore an **in-memory** index optimization (smaller RAM
   footprint per vector + faster traversal). On-disk stays `StoredVec` int8; a
   binary *disk* format is explicitly out of scope here (separate phase if ever).

## Goals

1. A `Binary` mode selectable via config/`QuantizationMode::parse` ("binary"|"bin").
2. Sign-bit encoding (`b: Vec<u8>`, 8 dims/byte) + Hamming distance for candidate
   generation in the HNSW traversal, with **f64 rescoring of the top-k shortlist**
   using the retained `HnswNode.vec`.
3. **Recall@k parity validated** against f64/int8 on the existing bench harness,
   the same way int8 was (`base::hnsw::tests::int8_recall_tracks_f64`).
4. Zero change to the on-disk format and zero regression for `None`/`Int8`.

## Non-goals

- A binary **on-disk** `StoredVec` variant (disk stays int8).
- Asymmetric distance (float-query vs binary-doc) beyond the f64 rescoring step.
- Rotation/learned-threshold refinements (ITQ etc.) — plain sign quant first.

## Design

### 1. `QuantizationMode::Binary` (`src/quant.rs`)

`Binary = 2`. Touch the **exhaustive** matches (compile-forcing):
`as_str` → `"binary"`; `bytes_per_dim` → `0.125`; `parse` adds the alias;
`QuantizedVec::encode` → `encode_binary`; `decode` → reconstruct `±1.0` per bit;
`dim` → bit count. `quantized_cosine_distance` gains a `(Binary, Binary)` arm
(Hamming proxy); its existing `_` arm already decodes+f64 as a correct fallback.

### 2. `QuantizedVec` gains the bit payload

Add `#[serde(default, skip_serializing_if = "Vec::is_empty")] b: Vec<u8>` plus a
`dim_bits: usize` (so the padded last byte doesn't corrupt `dim()`/decode). Both
are inert for `None`/`Int8`. Safe because the struct is never serialized (above);
the serde attrs are belt-and-suspenders so a future accidental serialization of a
non-binary vec stays byte-identical.

```
encode_binary(v): bit i set iff v[i] >= 0.0 ;  b = ceil(dim/8) bytes
hamming(a,b)    = Σ popcount(a.b[i] ^ b.b[i])           // padding bits match → 0
distance_proxy  = hamming as f64 / dim_bits as f64       // ∈ [0,1], monotone in angle
```

Padding bits are always 0 in both operands (sign of absent dims), so they never
contribute to Hamming — `dim_bits` is only needed for the proxy denominator and
`decode`/`dim`.

### 3. HNSW wire-in (`src/base/hnsw.rs`)

- `Query` enum gains `Binary { q: QuantizedVec, raw: &'a [f64] }`; `Query::new`
  matches `Binary` (today non-`Int8` falls to `Float`, so this is the real
  behaviour switch).
- `distance_to_query` / `distance_between`: `(Binary, Binary)` → Hamming proxy on
  the stored `qvec`; the retained `node.vec` (f64) stays for rescoring.
- **Rescore**: after the ef-search collects candidates by Hamming, re-rank the
  top-k by exact f64 cosine over `node.vec` before returning `HnswHit`s. This is
  the accuracy recovery that makes 1-bit candidate-gen acceptable; `node.vec` is
  already retained (confirmed: `HnswNode.vec: Vec<f64>`).

### 4. Store (`src/base/store.rs`)

No format change. `StoredVec` remains int8. On load the index is rebuilt with the
configured `quant_mode`; for `Binary` it re-encodes the decoded f64 to sign bits
in memory. (A `quant_mode = Binary` graph still persists int8 on disk.)

## Acceptance criteria (EARS)

- **WHERE** `quant_mode = Binary`, the index **SHALL** store each vector as
  `ceil(dim/8)` bytes and answer queries via Hamming candidate-gen + f64 rescore.
- **WHEN** the same corpus is indexed under `None`, `Int8`, and `Binary`, a
  recall@10 test **SHALL** assert `Binary` recall ≥ a documented floor of f64
  recall (target ≥ 0.90 with rescoring), mirroring `int8_recall_tracks_f64`.
- The on-disk bytes for an existing `None`/`Int8` graph **SHALL** be unchanged by
  this feature (round-trip test over `StoredVec`/`StoredKern`).
- `None` and `Int8` query results **SHALL** be byte-identical before/after.
- `bytes_per_dim(Binary) == 0.125`, surfaced in `memory::estimate_memory` so the
  64× claim is a measured column, not an assertion.

## Risks & notes

- **Recall floor is the gate.** 1-bit sign quant is coarse; the f64 rescore is
  what saves recall. If the rescored recall@k floor isn't met on the bench, the
  mode ships disabled-by-default and the SPEC revisits rotation/2-bit. Measure
  first — do not claim the 64× without the recall number beside it.
- **`dim_bits` correctness**: off-by-one in bit packing silently corrupts Hamming.
  Unit-test pack/unpack round-trip and a hand-computed Hamming case.
- **In-memory only**: be explicit in docs/CHANGELOG that this shrinks RAM/traversal,
  not disk (disk stays int8 `StoredVec`).
- **No-compat law**: one quantizer path; `Binary` extends the same `QuantizedVec`
  + `quantized_cosine_distance`, no parallel code path.

## Implementation findings (Phase 1, commit pending)

Two facts surfaced once the code was written — they correct the original plan:

1. **The f64 vector is NOT retained under quantization.** `HnswIndex::insert`
   stores `stored_vec = Vec::new()` for Int8 (the f64 is dropped; the int8 *is*
   the stored form — that is where int8's 8× saving comes from). So "rescore with
   the retained f64 `HnswNode.vec`" is impossible as written: there is no f64 to
   rescore against. Rescoring must use a *retained int8* representation instead.
2. **Pure 1-bit Hamming (no rescore) measures recall@10 ≈ 0.33** at dim=32
   (`base::hnsw::tests::binary_recall_tracks_f64`), vs int8's 0.75. Unusable as a
   drop-in. This is the measured proof that **rescore is mandatory**, not optional.

**Decision:** Phase 1 ships the tested binary primitive + the HNSW wiring +
the recall measurement, but `Binary` is **removed from `QuantizationMode::parse`**
(internal/`with_mode`-only) so no user can select a 0.33-recall mode. The "better
tool" for a configurable quant mode today remains int8. Phase 2 (below) makes
binary viable by retaining int8 per binary node and rescoring the Hamming
shortlist; only then is it exposed via config.

## Phasing

1. **quant.rs primitive + hnsw wiring + measurement (DONE).** `Binary` variant,
   `b`/`dim_bits`, `encode_binary`, decode, `dim`, Hamming proxy in
   `quantized_cosine_distance`, `bytes_per_dim`, `as_str`; `Query::Binary`,
   distance arms, `insert` stores binary qvec. Unit tests (pack round-trip,
   Hamming, monotonicity) + `binary_recall_tracks_f64` measuring **0.33**.
   `Binary` is **NOT** in `parse` (internal-only) given that number.
2. **int8-rescore (makes binary viable).** Retain an int8 vector per binary node
   (≈1.125 B/dim total, still ~7× smaller than f64); after Hamming candidate-gen,
   rescore the top-`ef` shortlist by int8 cosine and re-rank before truncating to
   `k`. Re-measure `binary_recall_tracks_f64`; target ≥ int8's 0.75 floor.
3. **Config + default** — once Phase 2 clears the floor, add `binary` back to
   `parse` and surface the `estimate_memory` 0.125/1.125 column; ship default-off.

## Leverages (already built)

The retained `HnswNode.vec` (for rescoring), the `QuantizationMode`/`QuantizedVec`
seam, `quantized_cosine_distance`'s fallback arm, `estimate_memory`, and the
`int8_recall_tracks_f64` validation template — Binary slots into all of them.
