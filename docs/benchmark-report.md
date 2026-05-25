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

| Suite        | File                      | Count | Scope                                                            |
| ------------ | ------------------------- | ----- | ---------------------------------------------------------------- |
| micro        | `benches/micro.rs`        | 35    | Pure CPU functions — parsing, XML gen, auth, lock eval           |
| fileserver   | `benches/fileserver.rs`   | 15    | GET (dispatch + body-drain), PUT (1KB–10MB), DELETE, dir listing |
| webdav       | `benches/webdav.rs`       | 9     | PROPFIND, MKCOL, COPY, MOVE, LOCK/UNLOCK, PROPPATCH              |
| middleware   | `benches/middleware.rs`   | 10    | HealthCheck, Auth (plaintext/SHA-512), LockEnforce               |
| path_resolve | `benches/path_resolve.rs` | 8     | Path resolution depth, percent-encoding, cold/hot cache          |
| scenarios    | `benches/scenarios.rs`    | 4     | Browser browse, WebDAV sync, lock-edit-unlock, mixed             |

**Total: 55 benchmarks across 6 suites.**

All benchmarks use `tower::ServiceExt::oneshot()` against the production `make_router()` — no TCP binding, isolating application-layer performance from network noise.

---

## Summary

| Category               | Metric             | Value         |
| ---------------------- | ------------------ | ------------- |
| GET 1MB dispatch       | Latency            | **42 µs**     |
| GET 64KB body-drain    | Latency            | **117 µs**    |
| GET 1MB body-drain     | Latency            | **1.10 ms**   |
| GET 10MB body-drain    | Latency            | **11.8 ms**   |
| GET 10MB read          | Throughput         | **846 MiB/s** |
| PUT 1KB (overwrite)    | Latency            | **64 µs**     |
| PUT 1KB (new)          | Latency            | **95 µs**     |
| PUT 10MB               | Throughput         | **749 MiB/s** |
| DELETE file            | Latency            | **287 µs**    |
| DELETE dir tree (d=5)  | Latency            | **7.04 ms**   |
| Dir listing 10 items   | Latency            | **106 µs**    |
| Dir listing 200 items  | Latency            | **1.24 ms**   |
| Dir listing 1000 items | Latency            | **5.97 ms**   |
| OPTIONS                | Latency            | **2.86 µs**   |
| PROPFIND depth:0       | Latency            | **267 µs**    |
| PROPFIND depth:1 (200) | Latency            | **21.5 ms**   |
| PROPFIND depth:inf     | Latency (3×5 tree) | **3.07 ms**   |
| MKCOL                  | Latency            | **259 µs**    |
| COPY file              | Latency            | **433 µs**    |
| COPY dir tree          | Latency            | **6.27 ms**   |
| MOVE file              | Latency            | **486 µs**    |
| LOCK exclusive         | Latency            | **248 µs**    |
| UNLOCK                 | Latency            | **358 µs**    |
| HealthCheck intercept  | Latency            | **1.12 µs**   |
| Auth plaintext valid   | Latency            | **42 µs**     |
| Auth SHA-512 valid     | Latency            | **572 µs**    |
| SHA-512 crypt (pure)   | Latency            | **524 µs**    |
| Lock enforce reject    | Latency            | **310 µs**    |
| Ancestor lock reject   | Latency            | **430 µs**    |
| Cold GET (new dir)     | Latency            | **266 µs**    |
| Hot GET (reuse)        | Latency            | **42.5 µs**   |
| Browser browse (3 rqs) | Latency            | **932 µs**    |
| WebDAV sync (6 reqs)   | Latency            | **2.58 ms**   |
| Lock-edit-unlock       | Latency            | **356 µs**    |
| Mixed workload (8 rqs) | Latency            | **3.74 ms**   |
| If-header parse        | Latency            | **109 ns**    |
| PROPFIND body parse    | Latency            | **347 ns**    |

---

## Fileserver Core

### GET — File Serving

GET benchmarks measure two independent dimensions of read performance:

| File Size | Dispatch Latency | Body-Drain Latency | Read Throughput |
| --------- | ---------------- | ------------------ | --------------- |
| 13 B      | **42.8 µs**      | —                  | —               |
| 1 KB      | **42.4 µs**      | —                  | —               |
| 64 KB     | **42.3 µs**      | **117 µs**         | **536 MiB/s**   |
| 1 MB      | **41.9 µs**      | **1.10 ms**        | **907 MiB/s**   |
| 10 MB     | **42.3 µs**      | **11.8 ms**        | **846 MiB/s**   |

> **Dispatch latency** (~42µs, constant regardless of file size): The time from
> request arrival to the handler returning control. This reflects the server's
> concurrency ceiling — it can accept ~24,000 requests/second. Measured via
> `oneshot()` against the router (headers-only, representing when the async
> handler releases back to the runtime).
>
> **Body-drain latency** (117µs–11.8ms): The time to fully read the file from
> disk and stream all bytes through the response. Scales linearly with file
> size. Measured by draining the response body via `to_bytes()`.
>
> **Read vs write comparison** (10MB):
>
> | Direction        | Latency     | Throughput    |
> | ---------------- | ----------- | ------------- |
> | GET (body-drain) | **11.8 ms** | **846 MiB/s** |
> | PUT              | **13.4 ms** | **749 MiB/s** |
>
> Read is ~12% faster than write — expected on APFS, where writes incur
> additional flush overhead.

### PUT — File Upload

| Scenario        | Latency     | Throughput    |
| --------------- | ----------- | ------------- |
| Overwrite 1KB   | **64 µs**   | 15.2 MiB/s    |
| New file 1KB    | **95 µs**   | 10.2 MiB/s    |
| Large file 10MB | **13.4 ms** | **749 MiB/s** |

### DELETE — File and Directory Trees

| Scenario               | Files Removed | Latency     |
| ---------------------- | ------------- | ----------- |
| Single file            | 1             | **287 µs**  |
| Depth 2 directory tree | ~20           | **2.81 ms** |
| Depth 3 directory tree | ~30           | **4.21 ms** |
| Depth 5 directory tree | ~50           | **7.04 ms** |

> Scaling is approximately linear with file count. `remove_dir_all` is the dominant cost.

### Directory Listing (HTML)

| Items | Latency     | Throughput (items/s) |
| ----- | ----------- | -------------------- |
| 10    | **106 µs**  | 94 K/s               |
| 50    | **338 µs**  | 148 K/s              |
| 200   | **1.24 ms** | 161 K/s              |
| 1000  | **5.97 ms** | 168 K/s              |

> Stable throughput at ~150K items/s. Each entry costs ~6µs — ~3µs for `read_dir` + metadata, ~3µs for HTML rendering.

---

## WebDAV Protocol

### PROPFIND — Property Retrieval

| Scenario                  | Entries | Latency     | Per-entry |
| ------------------------- | ------- | ----------- | --------- |
| Depth:0 single file       | 1       | **267 µs**  | 267 µs    |
| Depth:1 dir (10 files)    | 11      | **1.26 ms** | 115 µs    |
| Depth:1 dir (50 files)    | 51      | **5.55 ms** | 109 µs    |
| Depth:1 dir (200 files)   | 201     | **21.5 ms** | 107 µs    |
| Depth:infinity (3×5 tree) | ~20     | **3.07 ms** | 153 µs    |

> Stable at ~110µs per entry. Only **3µs** of this is XML generation — the remaining **97%** is file system traversal (`read_dir` + `metadata`), lock store reads, and dead property lookups.

### XML Generation — Micro-benchmark

| Entries | Latency     | Per-entry |
| ------- | ----------- | --------- |
| 1       | **3.64 µs** | 3.64 µs   |
| 10      | **30.9 µs** | 3.09 µs   |
| 100     | **308 µs**  | 3.08 µs   |
| 1000    | **3.07 ms** | 3.07 µs   |

> XML generation is efficient (~3µs/entry), scaling linearly. Not a bottleneck.

### Lock Operations

| Operation      | Latency    |
| -------------- | ---------- |
| LOCK exclusive | **248 µs** |
| LOCK shared    | **245 µs** |
| UNLOCK         | **358 µs** |

### COPY / MOVE

| Operation       | Latency     |
| --------------- | ----------- |
| COPY small file | **433 µs**  |
| COPY dir tree   | **6.27 ms** |
| MOVE small file | **486 µs**  |

---

## Middleware Cost Breakdown

### Health Check

| Scenario           | Latency     |
| ------------------ | ----------- |
| Intercept (200 OK) | **1.12 µs** |
| Passthrough GET    | **42 µs**   |

> HealthCheck intercepts before any downstream middleware runs. 1.12µs is effectively pure tower overhead.

### Authentication

| Scenario                | Latency     | Δ from no-auth         |
| ----------------------- | ----------- | ---------------------- |
| No users (noop)         | **42 µs**   | —                      |
| Plaintext valid         | **42 µs**   | **0 µs**               |
| Plaintext invalid (401) | **2.36 µs** | shorter (early return) |
| SHA-512 valid           | **572 µs**  | **+530 µs**            |
| SHA-512 invalid         | **530 µs**  | **+488 µs**            |

> SHA-512 crypt adds ~530µs per authenticated request. This is by design — a slow hash to resist brute force attacks. The pure `ShaCrypt::verify` call alone takes **524µs** (measured independently).

### Lock Enforcement (Middleware)

| Scenario                          | Latency    |
| --------------------------------- | ---------- |
| PUT unlocked (passthrough)        | **66 µs**  |
| PUT locked without token → 423    | **310 µs** |
| PUT locked with matching If token | **240 µs** |
| PUT ancestor locked (depth:inf)   | **430 µs** |

> Lock enforce adds ~2µs overhead on unlocked resources (evaluating the If-condition against an empty store via lock-count shortcut). Full evaluation (If-header parse + ancestor walk + exclusive check) adds ~244µs for rejected requests. Ancestor chain traversal costs an extra ~120µs per depth level.

---

## Path Resolution

| Scenario              | Latency    | Δ vs shallow |
| --------------------- | ---------- | ------------ |
| PUT shallow (1 level) | **264 µs** | —            |
| PUT deep (5 levels)   | **792 µs** | **+528 µs**  |
| PUT percent-encoded   | **263 µs** | **0 µs**     |
| GET shallow (1 level) | **266 µs** | —            |
| GET deep (5 levels)   | **790 µs** | **+524 µs**  |
| GET UTF-8 encoded     | **268 µs** | **+2 µs**    |

> Each additional path depth adds ~**130µs** from `tokio::fs::canonicalize` syscalls. Percent-encoding and UTF-8 paths impose negligible overhead.

### Cold vs Hot Cache

| Scenario                      | Latency     | Ratio |
| ----------------------------- | ----------- | ----- |
| Cold (fresh TempDir per iter) | **266 µs**  | 6.3×  |
| Hot (reuse same TempDir)      | **42.5 µs** | 1×    |

> Filesystem metadata caching by the OS accounts for **~223µs per request** (83% of GET latency). On hot caches, `canonicalize` + `metadata` become nearly free.

---

## End-to-End Scenarios

| Scenario                                                | Requests | Latency     | Avg/req |
| ------------------------------------------------------- | -------- | ----------- | ------- |
| Browser browse (GET /, /images/, file)                  | 3        | **932 µs**  | 311 µs  |
| WebDAV sync (PROPFIND d:1 + 5×GET)                      | 6        | **2.58 ms** | 430 µs  |
| Lock → edit (PUT with If) → unlock                      | 3        | **356 µs**  | 119 µs  |
| Mixed workload (5 GET + 1 PROPFIND + 1 PUT + 1 OPTIONS) | 8        | **3.74 ms** | 467 µs  |

> The mixed workload (80% GET, 15% PROPFIND, 5% PUT) on a 30-file directory completes 8 requests in ~3.7ms — **~2100 mixed requests/second** through the full middleware stack.

---

## Hot Path Analysis

### Bottleneck Ranking

| Rank | Component                   | Cost              | % of request (typ.) | Status      |
| ---- | --------------------------- | ----------------- | ------------------- | ----------- |
| 1    | **SHA-512 crypt verify**    | 524 µs            | 92% (auth GET)      | Design      |
| 2    | **fs::canonicalize (cold)** | ~223 µs           | 83% (cold GET)      | OS cache    |
| 3    | **read_dir + metadata**     | ~5 µs/entry       | 95% (dir listing)   | Optimal     |
| 4    | **create_new fallback**     | → eliminated      | → 64µs (was 93µs)   | ✅ Solved   |
| 5    | **Ancestor lock walk**      | → 66µs (was 95µs) | passthrough         | ✅ Improved |
| 6    | **PROPFIND fs traversal**   | ~100 µs/entry     | 97% (PROPFIND)      | OS-bound    |

> Items 4 and 5 have been addressed in performance improvements:
>
> - **PUT overwrite**: Replaced `create_new`-fallback pattern with `try_exists` pre-check + single `create` — saved 31% (93µs → 64µs).
> - **Lock enforce**: Replaced per-ancestor `HashMap` walk with lock-count shortcut for depth:infinity locks — unlocked passthrough reduced 33% (95µs → 66µs).

### Low-cost / Optimal Paths

| Component                    | Cost           | Notes                              |
| ---------------------------- | -------------- | ---------------------------------- |
| Method dispatch (`try_from`) | **1.60 ns**    | Essentially free                   |
| If-header parsing            | **109 ns**     | Handwritten parser, zero-alloc     |
| Header parsing               | **16–116 ns**  | Depth, Timeout, Destination, etc.  |
| XML generation               | **3 µs/entry** | Linear scaling, allocation-minimal |
| Lock token check             | **6 ns**       | Single iteration, short-circuit    |

---

## Conclusions

### Performance Profile

- **Dispatch latency is flat**: GET ~42µs regardless of file size — `canonicalize` + `metadata` + `open` dominate.
- **Body-drain throughput is high**: 846 MiB/s read, 749 MiB/s write. Read is ~12% faster than write (expected: writes incur flush overhead).
- **WebDAV overhead is moderate**: PROPFIND costs ~110µs per entry, dominated by fs traversal — not XML generation.
- **SHA-512 auth is the deliberate bottleneck**: 524µs per validation. Mitigate with persistent sessions if high-throughput auth is needed.
- **Path depth matters**: 5-level deep paths cost 3× more than single-level — `canonicalize` per component.
- **Concurrency ceiling**: In hot-cache scenarios, each GET dispatch takes ~42µs. Ceiling: **~24,000 requests/second** (single-core).

### Understanding GET Latency

The benchmark report presents two GET latency numbers for the same file:

- **Dispatch latency (42µs)**: Measures the async handler's dispatch time — when the handler returns control to the runtime, free to accept the next request. This is the correct metric for server concurrency.
- **Body-drain latency (117µs–11.8ms)**: Measures full read + stream time including disk I/O. Comparable to PUT write latency and useful for throughput analysis.

Both numbers are accurate — they measure different phases of the same HTTP transaction.

### Scaling Guidance

| Directory Size | PROPFIND depth:1 | Dir Listing HTML |
| -------------- | ---------------- | ---------------- |
| 10 files       | 1.3 ms           | 106 µs           |
| 50 files       | 5.6 ms           | 338 µs           |
| 100 files      | ~11 ms           | ~700 µs          |
| 200 files      | 21.5 ms          | 1.2 ms           |
| 1000 files     | ~110 ms          | 6.0 ms           |

> For directories with **>500 files**, PROPFIND depth:1 will exceed 50ms. WebDAV clients performing full-tree syncs on large directories should use depth:0 and iterate manually, or the server should support result streaming (not implemented).
