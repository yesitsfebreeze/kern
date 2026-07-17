# src/bench_support/locomo.rs — commentary

- `lcs_len`: the quadratic `m*n` is fine in practice because both inputs are normalized-answer token lists — for LoCoMo a handful to a few dozen tokens — and `rouge_l` never feeds document-length text here; the inline doc keeps the "cap long inputs first" caller rule.
- `category_name`: LoCoMo categories are 1=multi-hop, 2=temporal, 3=open-domain, 4=single-hop, 5=adversarial.
- `RawSample`/`RawQa`/`RawTurn`: mirror the on-disk LoCoMo JSON schema; `convert_sample` collects (index → date_time) and (index → turns) from the dynamic `session_N`/`session_N_date_time` keys, then pairs and sorts by index.

Second-pass migration: this file was already near the bar — nearly every remaining comment is a one-line trap or contract that earns its place (the `INCORRECT` contains `CORRECT` substring trap on `parse_judge_verdict`, the both-empty-is-a-vacuous-match guard in `token_f1`/`rouge_l`, the BLIP-caption fold on `Turn`, category-5-has-no-`answer` on `QaItem`, and the `lcs_len` cap-long-inputs caller rule). Only deletions: the `Sample` doc (restated the field names) and two test labels that duplicated their asserts (integer-answer coercion, unrecognized-verdict). The worked F1/ROUGE-L examples in `f1_partial_overlap` and `rouge_l_lcs_partial` were KEPT — each is a single line justifying a magic expected value (0.8 and 6/7).

Strict-bar pass (comments): removed more inline docs than the previous pass kept. Knowledge preserved here:
- Module: LoCoMo eval corpus = loader + answer scorers (#36), the pure half of the eval harness. Dataset is CC BY-NC 4.0 — supplied via a path, NEVER redistributed/bundled in the repo.
- `Turn`: image turns fold their BLIP caption into `text` (see `convert_turn`: `"<text> [shared image: <caption>]"`).
- `Session.index`: 1-based, parsed from the `session_N` key.
- `QaItem`: category 5 is adversarial (unanswerable) — `answer` is `None` and `adversarial_answer` holds the plausible-but-unsupported distractor. `is_adversarial()` == `category == 5`.
- `normalize_answer`: SQuAD-style — lowercase, strip punctuation, drop articles a/an/the, collapse whitespace.
- `token_f1`: token-level F1 over normalized tokens. Both-empty → vacuous match 1.0; exactly one empty → 0.0.
- `rouge_l`: ROUGE-L F1 (longest-common-subsequence based) over normalized tokens; same both-empty guard as token_f1 (avoids divide-by-zero).
- `is_abstention`: heuristic detecting whether the prediction declines to answer; used to score the adversarial category, where correct behavior is abstention. Lowercasing is char-wise so unicode around a marker doesn't break detection.
- `judge_prompt`: builds the LLM-judge prompt asking whether `pred` conveys the same facts as gold; judge replies CORRECT / INCORRECT.
- Test derivations (removed from source): `f1_partial_overlap` — pred {cat,sat} vs gold {cat,sat,down}: P=1, R=2/3, F1=0.8. `rouge_l_lcs_partial` — pred [x,b,c,d] vs gold [x,c,d]: LCS=3, R=1, P=3/4, F=6/7.
