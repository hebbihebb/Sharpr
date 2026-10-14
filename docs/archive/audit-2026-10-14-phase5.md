# Phase 5 audit — 2026-10-14

## 1. Summary

Phase 5 needs fixes before full acceptance: **0 Critical, 3 High, 10 Medium, and 4 Low findings** (16 CONFIRMED, 1 PLAUSIBLE). No new path in this range deletes tag rows, existing bucket membership, settings, or original image bytes, but tag conversion is not atomic, and indexing can persist a vector under the wrong fingerprint. Changed models can continue using old vectors, and inherited llama.cpp download settings defeat the offline guarantee and can replace an installed model. Ordinary browsing/search smoke testing of deployed `c176934` can proceed with local models and no inherited download configuration; the findings block accepting its failure handling, model replacement, disabled-folder behavior, and stable paging. The isolated suite passed 181/181; GTK search assertions completed, but its screenshot step timed out, and the 10% browsing-impact acceptance limit was not measured here.

Reviewed all six commits in `87ca43e..bd250fb`: `58ac0ef`, `78a0639`, `b07a226`, `c5407cf`, `c176934`, and `bd250fb`, including their changed documentation. Locations below refer to `bd250fb`. Another session added documentation-only commits during the review; application source and tools still matched the reviewed revision. Nothing under the prohibited real-data/model/install directories or Pictures was opened. Only this report is committed; reproduction helpers and logs are isolated under `.tmp/audit/`.

## 2. Findings

### 1. A changed file can poison the vector for its old content

**High · correctness · CONFIRMED**

**Location:** `sharpr/src/visual_search/index.rs:208`, `:232`, `:239`.

The pass validates every path's size/mtime while building `todo`, then embeds those paths later without another stamp check. If `a.jpg` and `copy.jpg` initially have identical content A, and an external editor replaces `a.jpg` with content B after the initial checks, the pass stores B's vector under A's fingerprint. The unchanged copy subsequently ranks as B; `Store::done` prevents correcting that vector on later passes. Deletion or loss of access during that interval can similarly record a permanent failure for otherwise readable identical copies.

**Evidence:** `run_pass` → upfront cached-stamp check → `(hash, path)` queue → `embed_image(path)` → `put_vector(model, hash, vector)`; the queue does not retain the stamp. The temporary probe calls the unchanged production `run_pass`, edits the representative file in its first progress callback, and embeds according to the file's current contents. Output: `fingerprint race: unchanged copy vector axis=1 (expected 0)`. Its scanner/index/preparation boundaries are stubs; the reviewed queue and storage path are real.

**Suggested fix:** Retain the expected stamp in `todo`; check it immediately before decoding and after preparation/embedding, before saving either a vector or a failure. Leave changed/disappeared paths pending for a fresh fingerprint pass; try an unchanged identical copy where available.

### 2. Model replacement at the same paths never invalidates the search model key

**High · correctness · CONFIRMED**

**Location:** `sharpr/src/visual_search/search.rs:204`, `:205`, `:216`; `sharpr/src/visual_search/files.rs:13`.

`ModelFiles` equality compares paths, not file stamps or hashes. Once a search has cached its model key, replacing either GGUF in place leaves that key unchanged for the search thread's lifetime. The indexing service computes the new key on its next pass, while searches keep loading old-key vectors. After the idle timeout, the query server can load the new model while still ranking old-model image vectors, producing incompatible comparisons.

**Evidence:** `State::prepare` reuses `self.model` whenever `known == files`; only the other branch calls `Store::model_key`, resets the server, and clears the last query. `vectors_version` reloads vectors under the already cached key. The probe replaces the model in place, independently verifies a changed store key, adds a new-key vector, and bumps the version: `model replacement: key_changed=true, old_vectors_used=true, new_vectors_used=false`.

**Suggested fix:** Revalidate the model key from current file stamps on each prepare request; the store already caches expensive hashes. On a changed key, invalidate the server, generic/query vectors, loaded image vectors, and the current search snapshot.

### 3. Inherited runtime download configuration permits networking and model replacement

**High · offline · CONFIRMED**

**Location:** `sharpr/src/visual_search/runtime.rs:157`, `:164`, `:169`; `sharpr/packaging/io.github.hebbihebb.Sharpr.yml:62`.

The managed child inherits its environment and is not passed `--offline`. Launching Sharpr from an environment containing `LLAMA_ARG_MODEL_URL` or similar llama.cpp remote-model settings can therefore trigger a download, even though Sharpr supplies a local `--model` path. For a URL download, llama.cpp uses that supplied path as its destination and can delete/replace the installed file. `LLAMA_OPENSSL=OFF` removes HTTPS support, not the HTTP downloader. This requires inherited configuration; it was not observed in a normal clean launch.

**Evidence:** The pinned local llama.cpp source maps `LLAMA_ARG_MODEL_URL` to `params.model.url` (`common/arg.cpp:3047`), creates a download task targeting `model.path` (`:430`), and executes it unless offline (`:693`). `common/download.cpp:321` removes an existing file when the remote ETag differs. The local-only probe used the scratch b11457 CPU runtime, Sharpr's query-server arguments, a fake GGUF, and a fixture HTTP server:

```text
offline=False, requests=['HEAD /replacement.gguf', 'GET /replacement.gguf'], model=b'GGUF downloaded!', exit=1
offline=True, requests=[], model=b'GGUF local-install', exit=1
```

Both processes exited because the fixture is not a complete model. No external endpoint or installed model was used.

**Suggested fix:** Force `--offline` for every managed server and remove inherited remote-model/projector/router configuration. Verify that child settings cannot redirect reads or writes outside the selected local files. Describe the Flatpak build as disabling HTTPS/UI fetches, rather than eliminating all network capabilities.

### 4. Tag-to-bucket conversion can leave an empty bucket after failure

**Medium · data-safety · CONFIRMED**

**Location:** `sharpr/src/buckets.rs:539`; `sharpr/src/library_index/mod.rs:785`; `sharpr/src/library_index/identity.rs:185`.

Bucket creation autocommits before member insertion begins its own transaction. A process crash between them leaves a durable empty bucket; an insertion error followed by a cleanup error does the same. Because the bucket's primary tag is now protected, `tag_offers` stops offering that tag on the next launch, although conversion did not succeed. Existing tag rows survive, but the promised “on failure nothing is created” guarantee is false.

**Evidence:** `bucket_from_tag` → `create_collection` → `add_to_bucket` → separate `add_bucket_members` transaction; rollback calls a third operation, `delete_collection`, and explicitly handles its failure. A scratch SQLite reproduction of these transaction boundaries, with failing insert/delete triggers, returned:

```text
add failed: member write failure
cleanup failed: rollback write failure
buckets left: [('nature',)]
members left: 0
```

This reproduces the SQL boundaries, not an injected fault in the running GTK application. The committed conversion test exercises only successful creation and duplicate-name rejection.

**Suggested fix:** Gather candidate paths/fingerprints on the worker, then create the collection and insert all its members in one library-index transaction. A failure or crash must roll back both together.

### 5. Conversion includes automatic rows excluded from the offer

**Medium · correctness · CONFIRMED**

**Location:** `sharpr/src/buckets.rs:503`, `:534`; `sharpr/src/tags/db.rs:317`, `:480`.

Offers exclude old automatic format rows, but conversion fetches every stored row with the selected tag. With `a.png` automatically tagged `png` and `b.jpg` manually tagged `png`, the dialog offers `png — 1 image`; creating it puts both files into the bucket. The new bucket also protects `png` from the automatic-tag cleanup, so the classification mismatch persists.

**Evidence:** `tag_offers` filters `(path, tag)` pairs through `automatic_tag_rows`; `bucket_from_tag` instead calls `stored_paths_for_all_tags_checked` and filters only by root. These exact inputs already appear in `unused_manual_tags_become_buckets_and_tag_rows_stay`, but that test converts `nature`, not `png`. The scratch SQL/classification reproduction printed `png offered count: 1` and `png conversion paths: [('/fixture/a.png',), ('/fixture/b.jpg',)]`.

**Suggested fix:** Share one eligibility function between the offer and conversion, including automatic-row exclusion. Recompute it when the user confirms and report the actual member count.

### 6. Disabled-folder changes do not stop indexing, and conversion writes disabled members

**Medium · correctness · CONFIRMED**

**Location:** `sharpr/src/ui/window.rs:1636`, `:917`; `sharpr/src/visual_search/index.rs:412`; `sharpr/src/buckets.rs:534`.

Disabling a folder updates settings and the view but does not request/cancel fingerprint or visual-index jobs. An already running visual pass can therefore continue decoding and writing vectors for the newly disabled subtree until its original job finishes or a later folder open causes another fingerprint pass. Enabling a subtree while idle likewise does not request its indexing. Independently, tag conversion has no disabled-folder argument and calls `add_to_bucket` on all tagged paths under the root, hashing and writing members from disabled folders.

**Evidence:** The complete ignored-folder callback (`window.rs:1636–1687`) updates settings, sidebar, viewer, and thumbnail generation, but never calls either service. Visual cancellation is set only when `Service::request` receives a different `Job`; those requests originate in the fingerprint-pass callback. Conversion filters only `starts_with(root)` before `add_to_bucket` → `current_fingerprint` → membership insertion. Static end-to-end finding; folder-toggle timing was not injected dynamically. Final search application does re-filter disabled paths in `load_virtual_async:404`, so this finding does not claim disabled images leak into the displayed search results.

**Suggested fix:** Immediately invalidate/requeue the background jobs when enabled scope changes, with cancellation checks before saving. Pass disabled folders into offer/conversion eligibility and exclude them from reads/hashes/writes.

### 7. “Show More” recomputes ranking and changes already shown results

**Medium · correctness · CONFIRMED**

**Location:** `sharpr/src/ui/window/search_controller.rs:420`, `:540`, `:625`; `sharpr/src/visual_search/search.rs:215`.

“Show More” reruns the search against the newest vector snapshot. During indexing, newly available high-scoring images can enter before previously shown results, moving the entire first page. Applying the response also clears selection and navigates to index zero. The query, scope, and filters have not changed, violating the stable-results requirement.

**Evidence:** The button raises `visual_limit`, then calls `run_browse_query_with_limit`; that invokes `Searcher::prepare`, which reloads on `vectors_version`, and reranks all candidates. `search_controller.rs:669–675` clears selection and discards the preferred selected path for every ranked response. The production search/rank probe added a better vector between pages: `paging: first=["old.jpg"], show_more=["new.jpg", "old.jpg"]`.

**Suggested fix:** Capture one ranked result sequence/vector snapshot per query/scope/filter combination. Extend its visible prefix on paging, preserving selection and scroll; refresh only when the search inputs change or through an explicit refresh action.

### 8. Fresh fingerprints are not loaded when the query or filters change

**Medium · correctness · CONFIRMED**

**Location:** `sharpr/src/ui/window/search_controller.rs:37`, `:171`, `:610`, `:617`.

The fingerprint map is cached for the whole browse session. Editing search words or chips reranks without advancing that session or reloading the map. If a candidate was not fingerprinted when the first query ran, completing fingerprinting/indexing does not make it searchable in later queries in the same open search. An edited candidate can remain excluded by the stale stamp even after its new vector is ready. Clearing and reopening search repairs it.

**Evidence:** `capture_browse_view` increments `browse_session` only when it creates the snapshot; `BrowseCaches::fingerprints` reloads only for a different session. Subsequent queries use the same snapshot and call `cache.fingerprints(session, index)`; the stamp guard then rejects the old entry or finds none. Vector-version changes reach `Searcher`, but not this cache. Static code path; not separately injected in GTK.

**Suggested fix:** Version the fingerprint cache by completed fingerprint passes or reload it when search inputs change. Keep paging's snapshot stable separately, as in finding 7.

### 9. A live GPU error permanently fails an image instead of falling back

**Medium · robustness · CONFIRMED**

**Location:** `sharpr/src/visual_search/index.rs:135`, `:143`, `:247`; `sharpr/src/visual_search/store.rs:124`.

GPU fallback is reached only if the child has died. An alive server returning HTTP 500 for a device/allocation failure, or timing out while still running, is classified as an image error and saved in `failures`. Later passes regard that fingerprint/model as done and never try it on the CPU. Turning off GPU indexing does not clear the failure. The same classification can permanently suppress images after transient CPU connection/server failures.

**Evidence:** The `Err(error) if server.is_alive()` branch precedes the GPU branch and special-cases only a wrong embedding dimension. `run_pass` persists every `EmbedError::Image`; `Store::done` unions failures with vectors. The probe's GPU child advertises sufficient memory, answers health successfully, stays alive, and returns 500: `live GPU HTTP500: Err(Image("llama-server answered 500: GPU allocation failed"))`. That exact error is eligible for permanent failure storage.

**Suggested fix:** Distinguish invalid-image rejection from transport/device/server errors. Retry an infrastructure error once on the CPU; stop/resume the pass for remaining infrastructure errors without marking the fingerprint permanently bad.

### 10. A stalled embedding blocks every newer browse/filter request

**Medium · lifecycle · CONFIRMED**

**Location:** `sharpr/src/ui/window/search_controller.rs:96`, `:540`; `sharpr/src/visual_search/search.rs:175`; `sharpr/src/visual_search/runtime.rs:55`, `:375`.

The sole coalescing browse worker blocks in `Searcher::prepare`, which waits synchronously for the search thread. Neither wait observes the browse cancellation generation. If the runtime stalls on a query or generic prompt, a newer query, sort, or filter request cannot execute until that call returns, potentially after the 300-second socket timeout. Main-thread navigation can continue, but search/filter responsiveness cannot. The socket timeout is per I/O operation, so a child sending intermittent partial data can extend the wait further.

**Evidence:** `BrowseWorker` executes one closure at a time; it cannot coalesce queued work while that closure waits in `response.recv()`. The query thread's `prepare` performs up to ten generic embeddings followed by the query; `http_request` uses `read_to_end` with a socket timeout, not a request deadline. Generation checks after preparation drop stale results but do not interrupt the wait. Static hang path; a five-minute hang was not deliberately run.

**Suggested fix:** Give query preparation a cancellation token and absolute deadline, poll replies without monopolizing the browse worker, and kill/reset a stalled owned child. Ensure nonvisual filters can proceed after cancellation.

### 11. The advertised decode budget does not bound JPEG XL decoding

**Medium · robustness · PLAUSIBLE**

**Location:** `sharpr/src/visual_search/prepare.rs:16`, `:63`; `sharpr/src/jxl.rs:277`.

The 3 GiB limit applies only to the `image::ImageReader` path. JPEG XL bypasses it and requests a full decode through `jpegxl-rs` before reducing to 768 pixels. A very large JXL in the enabled library can allocate beyond this budget automatically during indexing, in the Sharpr process itself. Exhausting the 14 GB machine or killing Sharpr is plausible; it was not induced.

**Evidence:** `prepare_png` → `decode_upright` → JXL early return → `decode_path` → mmap → `decode_from_bytes` → default decoder → `decode_to_image`. The installed `jpegxl-rs` decoder's `setup_image_buffer` (`src/decode.rs:417–421`) obtains the full output size and calls `pixels.resize(size, 0)` without Sharpr's limit. A 60,000 × 60,000 RGB8 output alone is 10.8 GB, before decoder working memory and the server. Code confirms the missing bound; the OOM consequence remains untested. No huge allocation was attempted.

**Suggested fix:** Validate dimensions/output size against an indexing memory budget before full JXL decode. Reject oversize inputs with a recorded image error or use a supported reduced decode; account for decoder scratch memory as well as output bytes.

### 12. Deleting the vector file while idle leaves cached results and does not reindex

**Medium · robustness · CONFIRMED**

**Location:** `sharpr/src/visual_search/search.rs:216`; `sharpr/src/ui/window.rs:917`, `:956`; `docs/reference/development.md:119`.

With an already populated search cache and idle indexing, deleting `visual-search.sqlite` does not invalidate the cached vectors or request another pass. Queries can keep claiming the deleted index's coverage, and the database can stay absent until a later fingerprint request or restart. The five-minute query-server idle timeout clears the server/generic vector, not the loaded image vectors. Deletion loses no irreplaceable data, but it does not reliably “force a reindex” while running.

**Evidence:** `State::loaded` is reused solely by model key and in-memory version; there is no file identity/existence check. Index requests follow completed fingerprint jobs, not file deletion. The probe removed the actual search store after loading vectors: `vector deletion: deleted_file=true, cached_vectors=2`. This is a different condition from deleting the file while Sharpr is closed, which works.

**Suggested fix:** Detect store replacement/removal and invalidate loaded vectors/model metadata, then request indexing. Alternatively make the supported delete/reindex procedure explicitly require closing Sharpr and provide an in-app reset operation.

### 13. AVIF and HEIC/HEIF library files cannot be visually indexed

**Medium · robustness · CONFIRMED**

**Location:** `sharpr/src/visual_search/prepare.rs:69`; `sharpr/src/model/library.rs:77`; `sharpr/Cargo.toml:23`.

The library collector includes AVIF, HEIC, and HEIF, but indexing's decoder supports neither AVIF nor HEIC/HEIF in this build. These otherwise viewable formats count as eligible images, then are permanently recorded as unreadable for this model key. Installing a working model/GPU cannot make them searchable by content.

**Evidence:** Preparation special-cases JPEG and JXL, then uses `ImageReader::decode`. Cargo disables default `image` features and enables only JPEG/PNG/GIF/WebP/TIFF/BMP/ICO; its compiled feature metadata agrees. A probe linked against the checkout's actual `image` rlib and supplied an AVIF signature to `ImageReader::into_decoder`:

```text
AVIF decoder: Some(Unsupported(UnsupportedError { format: Exact(Avif), kind: Format(Exact(Avif)) }))
```

This rejection precedes payload decoding; it does not depend on the fixture being a complete AVIF. HEIC/HEIF have no enabled decoder path either. `ServerEmbedder::embed_image:130` maps decode errors to permanent image failures.

**Suggested fix:** Support the library's established formats with a bounded worker-side decoder, considering Flatpak costs. Until then, distinguish unsupported formats from corrupt files and make the coverage limitation explicit; version preparation/failure handling when support is added.

### 14. Coverage excludes filename matches and can contradict the displayed results

**Low · correctness · CONFIRMED**

**Location:** `sharpr/src/ui/window/search_controller.rs:614`, `:625`, `:727`, `:732`.

Coverage and strength are computed only from candidates that did not match the filename. If four eligible images are indexed and one matches the filename, the status says “all 3 images searched” while showing four. If every image matches by filename, it says “None of these images are indexed yet · all 0 images searched”, even if every one has a vector. Many filename matches can also drop the strength sample below the 50-image threshold.

**Evidence:** Candidates are split into `by_name` and `others`; only `others` is passed to `rank`, whose `eligible`/`indexed` fields feed the status. The committed GTK test explicitly expects `all 3 images searched` for its four-image fixture (`gtk_tests.rs:732`), and reached that assertion successfully during this audit.

**Suggested fix:** Compute coverage over all filtered candidates; deduplicate filename matches only when constructing the displayed sequence. Identify the visual subset separately if its strength sample intentionally excludes name matches.

### 15. Closing search or opening another folder leaves the old search status visible

**Low · UI/docs · CONFIRMED**

**Location:** `sharpr/src/ui/window.rs:3468`, `:3475`; `sharpr/src/ui/window/search_controller.rs:281`, `:435`.

After a ranked search, Escape restores the original view but retains the strength/coverage line and possibly “Show More”. Opening another folder also calls `clear_inline_search` without hiding this status. The old query's summary is therefore displayed under an empty/closed search box for unrelated images.

**Evidence:** `close_browse_search` → `clear_inline_search` → `restore_browse_view`; folder open calls the same clear helper (`window.rs:1118`). The clear helper suppresses entry signals and resets snapshot/options/chips, but never calls `show_search_status(None)`. `sync_search_chip_bar` keeps the bar revealed while its status child remains visible. Static UI-state finding; no additional GUI action was injected.

**Suggested fix:** Clear the summary and paging control in the shared search-clear helper, including Escape and folder/bucket navigation.

### 16. Query failures send the user to Preferences, which has no query error

**Low · UI/docs · CONFIRMED**

**Location:** `sharpr/src/ui/window/search_controller.rs:702`; `sharpr/src/ui/preferences.rs:229`.

A query-only server failure is replaced by “visual search isn't available (Preferences → Library shows why)”. Preferences displays only the indexing service's status, which may say all images are indexed and contain no error. For example, after successful indexing, a text-server startup failure or HTTP error falls back to filenames without giving the user the actual reason anywhere in the interface.

**Evidence:** The full error arrives in `VisualSummary::Unavailable`, but `visual_summary_text` discards every reason except “No images are indexed”. `visual_search_status` reads only `window.visual_search_status()` from `visual_index`, not the searcher's result. Static error-routing finding.

**Suggested fix:** Present the actual concise query error in the status line or a toast, and retain diagnostic details where Preferences can show them.

### 17. The committed checkpoint contradicts its deployment and completed work

**Low · UI/docs · CONFIRMED**

**Location:** `CURRENT_TASKS.md:8`, `:9`, `:14`, `:20`, `:73`, `:75`; `ROADMAP.md:327` (all at `bd250fb`).

The current-task headline says all Phase 5 units are deployed, while the runtime/tag units still say “not deployed”. It calls the 1,000-image parity run open, then records its successful completion below; the waiting section says Phase 4 has yet to start. It also identifies the Flatpak server as `/app/bin/llama-server`, while the manifest and discovery code use `/app/libexec/sharpr-llama/llama-server`. The roadmap's Phase 5 status still says not deployed.

**Evidence:** `git show bd250fb:CURRENT_TASKS.md` and `git show bd250fb:ROADMAP.md` establish the contradictions within the requested range. Later concurrent documentation commits have already corrected the roadmap's deployment status, but stale unit/parity/Phase 4 text and the runtime path remained when last inspected. No newer feature scope was reviewed.

**Suggested fix:** Reconcile the remaining checkpoint statements with committed/deployed/accepted state and the executable's actual path. Keep pending acceptance measurements separate from completed parity work.

## 3. Checked and fine

- Successful tag conversion only reads tag rows; the unit and GTK conversion tests preserve every stored `(path, tag)` value, including missing-file rows.
- New GSettings key is additive; retired keys remain declared. The notice adds a library-index `schema_meta` row without changing its schema or compatibility requirements.
- Vector-store schema is separate, additive, version 1, and checked read-only for newer versions before setup; model-key filtering in `Store` itself works.
- Fixed-scope searches use the original folder/bucket snapshot; pending bucket opens defer search until their members arrive.
- Disabled paths are filtered during candidate selection and again immediately before loading search results; chips and dimension filters narrow the candidates before ranking.
- Filename matches precede visual matches, remain in the requested sort order, and are excluded from the visual output to avoid duplicate paths.
- Ranking breaks equal-score ties by path; strength uses the calibrated specific fifth-best score and the 50-image guard as documented, apart from the candidate-accounting issue above.
- Image preparation reads originals only; the JPEG/PNG unit test checks original bytes and bounded output size. JPEG reduction and orientation run on workers.
- Model hashing, decoding, vector SQLite bulk reads/writes, inference, and managed-server waits run off GTK; new result application checks query, scope, and generation and bumps the thumbnail generation.
- The image and query servers are spawned and retained on their respective long-lived `visual-index` and `visual-search` threads, so thread-scoped PDEATHSIG does not prematurely kill them.
- Managed-server Drop kills and waits; startup failure drops the child, query idle timeout drops its server, and indexing drops its server when a pass ends.
- GPU-death recursion is bounded by setting `gpu_failed` before retry; CPU crash handling stops at three crashes per pass, rather than recursively restarting forever.
- Missing model/projector/runtime and non-GGUF magic are explained; wrong vector length and nonfinite/zero runtime vectors are rejected.
- Successful vectors are persisted individually; unchanged fingerprints and recorded image failures are skipped on later passes, as the resumability unit tests verify.
- The app's HTTP client connects only to 127.0.0.1, uses a random 192-bit bearer key, and launches servers with an explicit loopback host; it has no redirect or external URL path.
- The Flatpak module pins its source hash, disables UI/npm/HF asset fetching and OpenMP fetching, and builds only the server target. The existing b11457 CMake cache/source confirmed the relevant switches; no Flatpak build was run.
- CPU server discovery matches the packaged executable path and sets adjacent shared-library lookup; GPU absence/startup failure takes the CPU path.
- Normal launches use a unique Gio application ID, so a second native launch on the same session activates the existing application; separate test instances intentionally use NON_UNIQUE and isolated data.
- Installer copies only explicitly supplied files into its separate model directory, verifies regular-file bytes, preserves shared-library symlinks, and skips identical files; it contains no network code.
- Viewer tag popover/pills, Ctrl+T shortcut/help section, metadata tag label, tag promotion drop target, and extra-tag dialog fields are removed; bucket/legacy tag persistence remains. The hidden filmstrip `Search tags…` placeholder and old internal tag-oriented names are dead wording, not a remaining reachable tag editor.

## 4. Test runs

Commands below are the commands executed, with their working directory identified. All real-model jobs ran serially with `MemoryMax=8G` and `MemorySwapMax=0`. No private benchmark, real library, deployment, or Flatpak build ran.

From the repository root, schema setup:

```bash
mkdir -p .tmp/audit/schemas .tmp/audit/data .tmp/audit/config .tmp/audit/cache
command cp -f sharpr/data/io.github.hebbihebb.Sharpr.gschema.xml .tmp/audit/schemas/
glib-compile-schemas .tmp/audit/schemas
```

From `sharpr/`, full checks, first inside the sandbox and then outside after its loopback bind denial:

```bash
I=$PWD/../.tmp/audit env CARGO_BUILD_JOBS=4 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/config XDG_CACHE_HOME=$PWD/../.tmp/audit/cache ./check.sh > ../.tmp/audit/check.log 2>&1
I=$PWD/../.tmp/audit env CARGO_BUILD_JOBS=4 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/config XDG_CACHE_HOME=$PWD/../.tmp/audit/cache ./check.sh > ../.tmp/audit/check-unsandboxed.log 2>&1
```

First: **180 passed, 1 failed, 6 skipped**, exit 100; failure was `requests_carry_the_key_and_body`, `PermissionDenied: Operation not permitted` at temporary listener bind. Rerun: **181 passed, 0 failed, 6 skipped**, exit 0; fmt, strict all-target/all-feature Clippy, deny, and machete passed. Deny emitted warnings for license parsing/allowances, duplicate transitive crates, and yanked `chacha20`; Cargo.lock was unchanged by this range.

From `sharpr/`, GTK/real-model runs:

```bash
env CARGO_BUILD_JOBS=4 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/gtk/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/gtk/config XDG_CACHE_HOME=$PWD/../.tmp/audit/gtk/cache SHARPR_GTK_FIXTURE=$PWD/../.tmp/audit/gtk/fixtures SHARPR_TEST_MODEL_DIR=$PWD/../.tmp/native-benchmark/models SHARPR_LLAMA_SERVER=$PWD/../.tmp/native-benchmark/runtime/llama-b11457/llama-server systemd-run --user --scope --quiet -p MemoryMax=8G -p MemorySwapMax=0 cargo nextest run --run-ignored ignored-only -E 'test(gtk_tests)' > ../.tmp/audit/gtk.log 2>&1
env CARGO_BUILD_JOBS=4 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/gtk2/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/gtk2/config XDG_CACHE_HOME=$PWD/../.tmp/audit/gtk2/cache SHARPR_GTK_FIXTURE=$PWD/../.tmp/audit/gtk2/fixtures SHARPR_TEST_MODEL_DIR=$PWD/../.tmp/native-benchmark/models SHARPR_LLAMA_SERVER=$PWD/../.tmp/native-benchmark/runtime/llama-b11457/llama-server systemd-run --user --scope --quiet -p MemoryMax=8G -p MemorySwapMax=0 cargo nextest run --run-ignored ignored-only -E 'test(gtk_tests)' --no-fail-fast > ../.tmp/audit/gtk2.log 2>&1
env CARGO_BUILD_JOBS=4 RUST_BACKTRACE=1 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/gtk3/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/gtk3/config XDG_CACHE_HOME=$PWD/../.tmp/audit/gtk3/cache SHARPR_GTK_FIXTURE=$PWD/../.tmp/audit/gtk3/fixtures SHARPR_TEST_MODEL_DIR=$PWD/../.tmp/native-benchmark/models SHARPR_LLAMA_SERVER=$PWD/../.tmp/native-benchmark/runtime/llama-b11457/llama-server systemd-run --user --scope --quiet -p MemoryMax=8G -p MemorySwapMax=0 cargo nextest run --run-ignored ignored-only -E 'test(gtk_tests::visual_search_ranks_by_content)' > ../.tmp/audit/gtk3.log 2>&1
```

- First GTK run: **0 passed, 1 failed, 3 selected tests not run**; rapid navigation observed 0 rows after first seeing the requested 40. The test predates this range and passed on rerun; no Phase 5 cause was established.
- Full rerun: **3 passed, 1 failed, 183 skipped**; rapid navigation, scoped search/crop, and tag conversion passed. Visual search timed out.
- Targeted backtrace rerun: **0 passed, 1 failed, 186 skipped**. The timeout is `render_node` → `screenshot_widget` → `visual_search_ranks_by_content:746`. All indexing, green/red ranking, filename-first, coverage, selection, and paging-button assertions preceding the screenshot completed. This is not a full GTK pass; screenshot/render failure remains unresolved.

From the repository root, two scratch model symlinks were created under `.tmp/audit/service/data/sharpr-models/embeddinggemma-2/`, targeting only the permitted `.tmp/native-benchmark/models/` files. From `sharpr/`:

```bash
env CARGO_BUILD_JOBS=4 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/service/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/service/config XDG_CACHE_HOME=$PWD/../.tmp/audit/service/cache SHARPR_LLAMA_SERVER=$PWD/../.tmp/native-benchmark/runtime/llama-b11457/llama-server SHARPR_LLAMA_SERVER_GPU=$PWD/../.tmp/native-benchmark/runtime/vulkan/llama-b11457/llama-server systemd-run --user --scope --quiet -p MemoryMax=8G -p MemorySwapMax=0 cargo nextest run --run-ignored ignored-only -E 'test(service_indexes_a_library_with_the_installed_model)' > ../.tmp/audit/service.log 2>&1
```

**1 passed, 0 failed, 186 skipped** in 25.052 s: two synthetic images indexed, one corrupt file recorded, vectors readable afterward. The command permits GPU selection/fallback; this assertion does not establish which device was selected.

Temporary reproduction helpers, from the repository root:

```bash
env CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/sharpr/target cargo build --offline --manifest-path .tmp/audit/probe/Cargo.toml > .tmp/audit/probe-build.log 2>&1
command cp -f sharpr/Cargo.lock .tmp/audit/probe/Cargo.lock
env CARGO_BUILD_JOBS=4 CARGO_TARGET_DIR=$PWD/sharpr/target cargo build --offline --manifest-path .tmp/audit/probe/Cargo.toml > .tmp/audit/probe-build.log 2>&1
env GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/.tmp/audit/schemas XDG_DATA_HOME=$PWD/.tmp/audit/probe-data XDG_CONFIG_HOME=$PWD/.tmp/audit/probe-config XDG_CACHE_HOME=$PWD/.tmp/audit/probe-cache SHARPR_LLAMA_SERVER=$PWD/.tmp/audit/probe/fake-server ./sharpr/target/debug/phase5-audit-probe $PWD/.tmp/audit/probe-state > .tmp/audit/probe.log 2>&1
python3 .tmp/audit/sql-probe.py > .tmp/audit/sql.log 2>&1
env GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/.tmp/audit/schemas XDG_DATA_HOME=$PWD/.tmp/audit/offline-data XDG_CONFIG_HOME=$PWD/.tmp/audit/offline-config XDG_CACHE_HOME=$PWD/.tmp/audit/offline-cache systemd-run --user --scope --quiet -p MemoryMax=8G -p MemorySwapMax=0 python3 .tmp/audit/offline-probe.py > .tmp/audit/offline.log 2>&1
env GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/.tmp/audit/schemas XDG_DATA_HOME=$PWD/.tmp/audit/offline-data XDG_CONFIG_HOME=$PWD/.tmp/audit/offline-config XDG_CACHE_HOME=$PWD/.tmp/audit/offline-cache systemd-run --user --scope --quiet -p MemoryMax=8G -p MemorySwapMax=0 python3 .tmp/audit/offline-probe.py > .tmp/audit/offline2.log 2>&1
rustc --edition=2021 .tmp/audit/decode-probe.rs -L dependency=sharpr/target/debug/deps --extern image=sharpr/target/debug/deps/libimage-f42af03e362c8e6d.rlib -o .tmp/audit/decode-probe
.tmp/audit/decode-probe
```

The first harness build failed because its initially resolved libc could not be unpacked into the sandboxed Cargo cache. After copying the repository lockfile, an approved offline build passed, with three unused-variable warnings caused by the harness's no-op logging macros. The harness imports unchanged production files/runtime/store/search/index modules and fingerprint hash/stamp routines; scanner/index/preparation dependencies are deliberately stubbed. Its five observations reproduce findings 1, 2, 7, 9, and 12; they are not application regression tests. The SQL helper reproduces transaction boundaries and tag-row selection for findings 4–5. The first offline helper invocation had a wrong scratch binary path and failed with FileNotFoundError; after correcting that helper path, both comparison cases completed with the output in finding 3. The AVIF probe compiled successfully and confirmed the unsupported decoder before payload decoding.

Scratch installer checks, from the repository root, after creating fake GGUF files, a shell runtime, and a relative shared-library symlink under `.tmp/audit/installer-source/`:

```bash
env XDG_DATA_HOME=$PWD/.tmp/audit/installer-data python3 tools/install-visual-search-model.py --models .tmp/audit/installer-source/models --runtime .tmp/audit/installer-source/runtime --gpu-runtime .tmp/audit/installer-source/runtime
env XDG_DATA_HOME=$PWD/.tmp/audit/installer-data python3 tools/install-visual-search-model.py --models .tmp/audit/installer-source/models --runtime .tmp/audit/installer-source/runtime --gpu-runtime .tmp/audit/installer-source/runtime
```

Both exited 0: **8 files updated**, then **0 files updated**. A Python comparison of both model files and both runtime trees returned **8/8 byte/symlink checks passed**. No actual runtime or model was installed.

Final required build, from `sharpr/`:

```bash
env CARGO_BUILD_JOBS=4 GSETTINGS_BACKEND=memory GSETTINGS_SCHEMA_DIR=$PWD/../.tmp/audit/schemas XDG_DATA_HOME=$PWD/../.tmp/audit/data XDG_CONFIG_HOME=$PWD/../.tmp/audit/config XDG_CACHE_HOME=$PWD/../.tmp/audit/cache cargo build > ../.tmp/audit/build.log 2>&1
```

**Passed**, exit 0, 2.07 s. This does not update the deployed launcher. Report validation also ran `git diff --check` and `git diff --cached --check`, and checked referenced tracked paths.

## 5. Open questions for the maintainer

- Should indexing freeze a content snapshot while external tools edit the library, or skip/retry every file whose stamp changes? Either policy must prevent finding 1.
- Should “Show More” retain the first query's coverage/ranking until inputs change, with an explicit refresh for newly indexed images? The roadmap currently requires stable results.
- Are disabled-folder tag rows meant to remain eligible for conversion despite the blanket write-path exclusion? Current code provides neither a scoped exclusion nor an explicit exception.
- What decoder/preparation support is intended for AVIF and HEIC/HEIF, which the library already accepts?
- Can the GTK screenshot helper be run on a consistently drawable test display, and can the rapid-navigation timing failure be reproduced independently? The current runs do not establish their cause.
- Has the deployed build's browsing impact been measured against the 10% gate? The concurrent checkpoint records that browsing feels smooth, but this audit did not reproduce a quantitative before/after measurement or the private 1,000-image parity run.
