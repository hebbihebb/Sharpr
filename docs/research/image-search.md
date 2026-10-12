# Offline image-search feasibility

Approved direction: visual search narrows the current folder/bucket; manual tags and bucket membership remain authoritative. Generated labels are not required.

## Primary sources

- https://ai.google.dev/gemma/docs/embeddinggemma/model_card_2
- https://developers.googleblog.com/embeddinggemma-2-the-developer-guide/
- https://github.com/ggml-org/llama.cpp/pull/30054
- https://github.com/ggml-org/llama.cpp/issues/30082 (the author corrected the report; correctly nested image inputs work)
- https://github.com/google-research/big_vision/blob/main/big_vision/configs/proj/image_text/README_siglip2.md
- https://github.com/GNOME/loupe/blob/main/NEWS

## Reference harness

`tools/benchmark-image-search.py` accepts local model files only. A development-only virtual environment lives in `.tmp/search-benchmark`; it is not an application dependency. Input JSON has `images` (explicit paths) and `queries` (objects containing `text` and optional `expected` path lists). Exclude disabled folders when preparing the manifest. Keep private image lists and result artifacts under `.tmp`, not git.

Use a representative 1,000-image sample and 30 predefined queries/expected examples. Do not select expectations after seeing model results. The report records top-20 matches, recall of expected examples, query latency, indexing throughput, memory, and decoding errors. An unjudged query does not count as success. Reference results alone cannot pass the gate: native-runtime parity and measured filmstrip impact are also required.

## Release gate

At least 80% useful top-20 results, warm queries below one second on this laptop, and no more than 10% navigation latency increase while indexing. CPU operation is required; GPU optional. Whole-library indexing overnight is acceptable; initial indexing speed is not a gate. Folder/partial indexing controls and scheduling are later refinements. Choose Gemma if it passes, otherwise SigLIP only if it passes. If neither passes, do not ship visual search. Runtime packaging must be verified before release. Model/setup downloads during development are separate from the application, which has no automatic model downloader.


## Checkpoint status

These tools are preserved development experiments, not application runtime dependencies. Existing reports and vectors remain private under `.tmp/`. They do not establish a release gate: reports measure known-example hits, lack complete provenance, and use one timed query per warmup. Relevance judgments, reproducibility, indexing impact and packaging remain Phase 3 work. The authoritative rubric and candidate decision are in `ROADMAP.md`; do not substitute the hit-rate metric for relevance.

Unfinished in-app trial wiring is preserved on `visual-search-trial-wip`; the ordinary checkpoint has neither that Cargo feature nor an inference client. Tool syntax and help paths were checked without model loading or rerunning the corpus.
