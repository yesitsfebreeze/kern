# src/bench_support/locomo.rs — commentary

- `lcs_len`: the quadratic `m*n` is fine in practice because both inputs are normalized-answer token lists — for LoCoMo a handful to a few dozen tokens — and `rouge_l` never feeds document-length text here; the inline doc keeps the "cap long inputs first" caller rule.
- `category_name`: LoCoMo categories are 1=multi-hop, 2=temporal, 3=open-domain, 4=single-hop, 5=adversarial.
- `RawSample`/`RawQa`/`RawTurn`: mirror the on-disk LoCoMo JSON schema; `convert_sample` collects (index → date_time) and (index → turns) from the dynamic `session_N`/`session_N_date_time` keys, then pairs and sorts by index.

Second-pass migration: this file was already near the bar — nearly every remaining comment is a one-line trap or contract that earns its place (the `INCORRECT` contains `CORRECT` substring trap on `parse_judge_verdict`, the both-empty-is-a-vacuous-match guard in `token_f1`/`rouge_l`, the BLIP-caption fold on `Turn`, category-5-has-no-`answer` on `QaItem`, and the `lcs_len` cap-long-inputs caller rule). Only deletions: the `Sample` doc (restated the field names) and two test labels that duplicated their asserts (integer-answer coercion, unrecognized-verdict). The worked F1/ROUGE-L examples in `f1_partial_overlap` and `rouge_l_lcs_partial` were KEPT — each is a single line justifying a magic expected value (0.8 and 6/7).
