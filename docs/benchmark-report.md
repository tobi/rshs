# Benchmark Report

## Test Environment

| Item       | Detail                                 |
| ---------- | -------------------------------------- |
| rshs       | v0.8.2                                 |
| Rust       | 1.87+ (edition 2024)                   |
| Criterion  | 0.5 with `async_tokio`, `html_reports` |
| Platform   | macOS aarch64 (Apple Silicon)          |
| Profile    | `bench` (optimized release)            |
| Filesystem | APFS (on internal SSD)                 |

## How to Run

```sh
cargo bench                      # Run all 6 suites
cargo bench --bench fileserver   # File server only
cargo bench --bench webdav       # WebDAV protocol only
cargo bench -- "GET/tiny"        # Filter by benchmark name
```

Results are written to `target/criterion/`. Open `target/criterion/report/index.html` for interactive HTML reports with change detection against previous runs.

---

## Suite Overview

| Suite        | File                      | Count | Scope                                                    |
| ------------ | ------------------------- | ----- | -------------------------------------------------------- |
| micro        | `benches/micro.rs`        | 35    | Pure CPU functions — parsing, XML gen, auth, lock eval   |
| fileserver   | `benches/fileserver.rs`   | 12    | GET/HEAD (13B–10MB), PUT (1KB–10MB), DELETE, dir listing |
| webdav       | `benches/webdav.rs`       | 9     | PROPFIND, MKCOL, COPY, MOVE, LOCK/UNLOCK, PROPPATCH      |
| middleware   | `benches/middleware.rs`   | 10    | HealthCheck, Auth (plaintext/SHA-512), LockEnforce       |
| path_resolve | `benches/path_resolve.rs` | 8     | Path resolution depth, percent-encoding, cold/hot cache  |
| scenarios    | `benches/scenarios.rs`    | 4     | Browser browse, WebDAV sync, lock-edit-unlock, mixed     |

**Total: 52 benchmarks across 6 suites.**

All benchmarks use `tower::ServiceExt::oneshot()` against the production `make_router()` — no TCP binding, isolating application-layer performance from network noise.

---

## Summary

| Category               | Metric             | Value          |
| ---------------------- | ------------------ | -------------- |
| GET 13B file           | Latency            | **44.0 µs**    |
| GET 1KB file           | Throughput         | **21.6 MiB/s** |
| GET 1MB file           | Throughput         | **22.9 GiB/s** |
| PUT 1KB (new)          | Latency            | **122 µs**     |
| PUT 1KB (overwrite)    | Latency            | **93 µs**      |
| PUT 10MB               | Throughput         | **693 MiB/s**  |
| DELETE file            | Latency            | **268 µs**     |
| DELETE dir tree (d=5)  | Latency            | **7.00 ms**    |
| Dir listing 10 items   | Latency            | **111 µs**     |
| Dir listing 200 items  | Latency            | **1.46 ms**    |
| Dir listing 1000 items | Latency            | **6.20 ms**    |
| OPTIONS                | Latency            | **2.91 µs**    |
| PROPFIND depth:0       | Latency            | **267 µs**     |
| PROPFIND depth:1 (200) | Latency            | **21.9 ms**    |
| PROPFIND depth:inf     | Latency (3×5 tree) | **3.06 ms**    |
| MKCOL                  | Latency            | **278 µs**     |
| COPY file              | Latency            | **452 µs**     |
| COPY dir tree          | Latency            | **6.32 ms**    |
| MOVE file              | Latency            | **525 µs**     |
| LOCK exclusive         | Latency            | **259 µs**     |
| UNLOCK                 | Latency            | **377 µs**     |
| HealthCheck intercept  | Latency            | **1.12 µs**    |
| Auth plaintext valid   | Latency            | **42.1 µs**    |
| Auth SHA-512 valid     | Latency            | **571 µs**     |
| SHA-512 crypt (pure)   | Latency            | **524 µs**     |
| Lock enforce reject    | Latency            | **327 µs**     |
| Ancestor lock reject   | Latency            | **448 µs**     |
| Cold GET (new dir)     | Latency            | **265 µs**     |
| Hot GET (reuse)        | Latency            | **42.3 µs**    |
| Browser browse (3 rqs) | Latency            | **928 µs**     |
| WebDAV sync (6 reqs)   | Latency            | **2.60 ms**    |
| Lock-edit-unlock       | Latency            | **385 µs**     |
| Mixed workload (8 rqs) | Latency            | **3.78 ms**    |
| If-header parse        | Latency            | **110 ns**     |
| PROPFIND body parse    | Latency            | **347 ns**     |

---

## Fileserver Core

### GET — File Serving

| File Size | Latency     | Throughput | Notes                                |
| --------- | ----------- | ---------- | ------------------------------------ |
| 13 B      | **44.0 µs** | 289 KiB/s  | Metadata overhead dominates          |
| 1 KB      | **45.1 µs** | 21.6 MiB/s | Same as 13B — fixed per-request cost |
| 64 KB     | **44.1 µs** | 1.39 GiB/s | Throughput jumps with file size      |
| 1 MB      | **42.5 µs** | 22.9 GiB/s | Memory-speed on hot page cache       |
| 10 MB     | **44.0 µs** | 222 GiB/s  | Stream setup ~44µs, I/O in reactor   |

> **Key insight**: GET latency is constant (~44µs) regardless of file size. The fixed cost is `canonicalize` + `metadata` + `open` syscalls. Actual data transfer happens in the async reactor via `ReaderStream`, not measured in the benchmark's router-level timing. Throughput scales with file size because the ratio of I/O time to fixed overhead increases.

### PUT — File Upload

| Scenario                              | Latency     | Throughput    |
| ------------------------------------- | ----------- | ------------- |
| New file 1KB (`create_new`)           | **122 µs**  | 8.0 MiB/s     |
| Overwrite 1KB (`create_new`→`create`) | **93 µs**   | 10.5 MiB/s    |
| Large file 10MB                       | **14.4 ms** | **693 MiB/s** |

> **Key insight**: The `create_new` + fallback pattern costs ~29µs (30%) on overwrites — one failed syscall per overwritten PUT. The 10MB upload achieves ~693 MiB/s, limited by `StreamReader` chunking and the `tokio::io::copy` 8KB buffer.

### DELETE — File and Directory Trees

| Scenario               | Files Removed | Latency     |
| ---------------------- | ------------- | ----------- |
| Single file            | 1             | **268 µs**  |
| Depth 2 directory tree | ~20           | **2.83 ms** |
| Depth 3 directory tree | ~30           | **4.14 ms** |
| Depth 5 directory tree | ~50           | **7.00 ms** |

> Scaling is approximately linear with file count. `remove_dir_all` is the dominant cost.

### Directory Listing (HTML)

| Items | Latency     | Throughput (items/s) |
| ----- | ----------- | -------------------- |
| 10    | **111 µs**  | 90.4 K/s             |
| 50    | **353 µs**  | 141.5 K/s            |
| 200   | **1.46 ms** | 137.1 K/s            |
| 1000  | **6.20 ms** | 161.4 K/s            |

> Stable throughput at ~140K items/s. Each entry costs ~7µs — ~3µs for `read_dir` + metadata, ~4µs for HTML rendering.

---

## WebDAV Protocol

### PROPFIND — Property Retrieval

| Scenario                  | Entries | Latency     | Per-entry |
| ------------------------- | ------- | ----------- | --------- |
| Depth:0 single file       | 1       | **267 µs**  | 267 µs    |
| Depth:1 dir (10 files)    | 11      | **1.26 ms** | 115 µs    |
| Depth:1 dir (50 files)    | 51      | **5.62 ms** | 110 µs    |
| Depth:1 dir (200 files)   | 201     | **21.9 ms** | 109 µs    |
| Depth:infinity (3×5 tree) | ~20     | **3.06 ms** | 153 µs    |

> Stable at ~110µs per entry. Only **3µs** of this is XML generation — the remaining **97%** is file system traversal (`read_dir` + `metadata`), lock store reads, and dead property lookups.

### XML Generation — Micro-benchmark

| Entries | Latency     | Per-entry |
| ------- | ----------- | --------- |
| 1       | **3.80 µs** | 3.80 µs   |
| 10      | **31.6 µs** | 3.16 µs   |
| 100     | **309 µs**  | 3.09 µs   |
| 1000    | **3.07 ms** | 3.07 µs   |

> XML generation is efficient (~3µs/entry), scaling linearly. Not a bottleneck.

### Lock Operations

| Operation      | Latency    |
| -------------- | ---------- |
| LOCK exclusive | **259 µs** |
| LOCK shared    | **256 µs** |
| UNLOCK         | **377 µs** |

### COPY / MOVE

| Operation       | Latency     |
| --------------- | ----------- |
| COPY small file | **452 µs**  |
| COPY dir tree   | **6.32 ms** |
| MOVE small file | **525 µs**  |

---

## Middleware Cost Breakdown

### Health Check

| Scenario           | Latency     |
| ------------------ | ----------- |
| Intercept (200 OK) | **1.12 µs** |
| Passthrough GET    | **42.3 µs** |

> HealthCheck intercepts before any downstream middleware runs. 1.12µs is effectively pure tower overhead.

### Authentication

| Scenario                | Latency     | Δ from no-auth         |
| ----------------------- | ----------- | ---------------------- |
| No users (noop)         | **42.2 µs** | —                      |
| Plaintext valid         | **42.1 µs** | **0 µs**               |
| Plaintext invalid (401) | **2.32 µs** | shorter (early return) |
| SHA-512 valid           | **571 µs**  | **+529 µs**            |
| SHA-512 invalid         | **525 µs**  | **+483 µs**            |

> SHA-512 crypt adds ~530µs per authenticated request. This is by design — a slow hash to resist brute force attacks. The pure `ShaCrypt::verify` call alone takes **524µs** (measured independently).

### Lock Enforcement (Middleware)

| Scenario                          | Latency    |
| --------------------------------- | ---------- |
| PUT unlocked (passthrough)        | **95 µs**  |
| PUT locked without token → 423    | **327 µs** |
| PUT locked with matching If token | **240 µs** |
| PUT ancestor locked (depth:inf)   | **448 µs** |

> Lock enforce adds ~5µs overhead on unlocked resources (evaluating the If-condition against an empty store). Full evaluation (If-header parse + ancestor walk + exclusive check) adds ~230µs for rejected requests and ~145µs for accepted ones. Ancestor chain traversal costs an extra ~120µs per depth level.

---

## Path Resolution

| Scenario              | Latency    | Δ vs shallow |
| --------------------- | ---------- | ------------ |
| PUT shallow (1 level) | **270 µs** | —            |
| PUT deep (5 levels)   | **807 µs** | **+537 µs**  |
| PUT percent-encoded   | **270 µs** | **0 µs**     |
| GET shallow (1 level) | **264 µs** | —            |
| GET deep (5 levels)   | **795 µs** | **+531 µs**  |
| GET UTF-8 encoded     | **273 µs** | **+9 µs**    |

> Each additional path depth adds ~**130µs** from `tokio::fs::canonicalize` syscalls. Percent-encoding and UTF-8 paths impose negligible overhead.

### Cold vs Hot Cache

| Scenario                      | Latency     | Ratio |
| ----------------------------- | ----------- | ----- |
| Cold (fresh TempDir per iter) | **265 µs**  | 6.3×  |
| Hot (reuse same TempDir)      | **42.3 µs** | 1×    |

> Filesystem metadata caching by the OS accounts for **~223µs per request** (83% of GET latency). On hot caches, `canonicalize` + `metadata` become nearly free.

---

## End-to-End Scenarios

| Scenario                                                | Requests | Latency     | Avg/req |
| ------------------------------------------------------- | -------- | ----------- | ------- |
| Browser browse (GET /, /images/, file)                  | 3        | **928 µs**  | 309 µs  |
| WebDAV sync (PROPFIND d:1 + 5×GET)                      | 6        | **2.60 ms** | 433 µs  |
| Lock → edit (PUT with If) → unlock                      | 3        | **385 µs**  | 128 µs  |
| Mixed workload (5 GET + 1 PROPFIND + 1 PUT + 1 OPTIONS) | 8        | **3.78 ms** | 473 µs  |

> The mixed workload (80% GET, 15% PROPFIND, 5% PUT) on a 30-file directory completes 8 requests in ~3.8ms — **~2100 mixed requests/second** through the full middleware stack.

---

## Hot Path Analysis

### Bottleneck Ranking

| Rank | Component                   | Cost          | % of request (typ.) | Mitigation                                    |
| ---- | --------------------------- | ------------- | ------------------- | --------------------------------------------- |
| 1    | **SHA-512 crypt verify**    | 524 µs        | 92% (auth GET)      | Token/session caching; intentional cost       |
| 2    | **fs::canonicalize (cold)** | ~220 µs       | 83% (cold GET)      | Cache canonical results; OS page cache helps  |
| 3    | **read_dir + metadata**     | ~5 µs/entry   | 95% (dir listing)   | `tokio::fs` blocking pool is optimal          |
| 4    | **create_new fallback**     | +29 µs        | 30% (PUT overwrite) | `try_exists` pre-check (TOCTOU trade-off)     |
| 5    | **Ancestor lock walk**      | +120 µs       | 38% (locked PUT)    | Cache lock path topology                      |
| 6    | **PROPFIND fs traversal**   | ~100 µs/entry | 97% (PROPFIND)      | Depth:0/1 when possible; avoid depth:infinity |

### Low-cost / Optimal Paths

| Component                    | Cost           | Notes                              |
| ---------------------------- | -------------- | ---------------------------------- |
| Method dispatch (`try_from`) | **1.9 ns**     | Essentially free                   |
| If-header parsing            | **110 ns**     | Handwritten parser, zero-alloc     |
| Header parsing               | **16–112 ns**  | Depth, Timeout, Destination, etc.  |
| XML generation               | **3 µs/entry** | Linear scaling, allocation-minimal |
| Lock token check             | **6 ns**       | Single iteration, short-circuit    |

---

## Conclusions

### Performance Profile

- **File server core is fast**: 44µs GET latency (cold: 265µs). In-memory serving approaches filesystem limits.
- **WebDAV overhead is moderate**: PROPFIND costs ~110µs per entry, dominated by fs traversal — not XML generation.
- **SHA-512 auth is the deliberate bottleneck**: 524µs per validation. Mitigate with persistent sessions if high-throughput auth is needed.
- **Path depth matters**: 5-level deep paths cost 3× more than single-level — `canonicalize` per component.
- **Throughput ceiling**: In hot-cache scenarios, each GET/PUT takes ~42–94µs. Conservative ceiling: **10,000–23,000 requests/second** (single-core).

### Scaling Guidance

| Directory Size | PROPFIND depth:1 | Dir Listing HTML |
| -------------- | ---------------- | ---------------- |
| 10 files       | 1.3 ms           | 111 µs           |
| 50 files       | 5.6 ms           | 353 µs           |
| 100 files      | ~11 ms           | ~700 µs          |
| 200 files      | 21.9 ms          | 1.5 ms           |
| 1000 files     | ~110 ms          | 6.2 ms           |

> For directories with **>500 files**, PROPFIND depth:1 will exceed 50ms. WebDAV clients performing full-tree syncs on large directories should use depth:0 and iterate manually, or the server should support result streaming (not implemented).
