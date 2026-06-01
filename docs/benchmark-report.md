# Benchmark Report

## Test Environment

| Item       | Detail                        |
| ---------- | ----------------------------- |
| rshs       | v0.9.0                        |
| Rust       | 1.87+ (edition 2024)          |
| Criterion  | 0.8                           |
| Platform   | macOS aarch64 (Apple Silicon) |
| Profile    | `bench` (optimized release)   |
| Filesystem | APFS (on internal SSD)        |

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
| middleware   | `benches/middleware.rs`   | 11    | HealthCheck, Auth (plaintext/SHA-512/cached), LockEnforce        |
| path_resolve | `benches/path_resolve.rs` | 8     | Path resolution depth, percent-encoding, cold/hot cache          |
| scenarios    | `benches/scenarios.rs`    | 4     | Browser browse, WebDAV sync, lock-edit-unlock, mixed             |

**Total: 56 benchmarks across 6 suites.**

All benchmarks use `tower::ServiceExt::oneshot()` against the production `make_router()` — no TCP binding, isolating application-layer performance from network noise.

---

## Summary

| Category               | Metric             | Value         |
| ---------------------- | ------------------ | ------------- |
| GET 1MB dispatch       | Latency            | **42 µs**     |
| GET 64KB body-drain    | Latency            | **118 µs**    |
| GET 1MB body-drain     | Latency            | **1.13 ms**   |
| GET 10MB body-drain    | Latency            | **12.0 ms**   |
| GET 10MB read          | Throughput         | **835 MiB/s** |
| PUT 1KB (overwrite)    | Latency            | **70 µs**     |
| PUT 1KB (new)          | Latency            | **96 µs**     |
| PUT 10MB               | Throughput         | **720 MiB/s** |
| DELETE file            | Latency            | **271 µs**    |
| DELETE dir tree (d=5)  | Latency            | **7.07 ms**   |
| Dir listing 10 items   | Latency            | **67 µs**     |
| Dir listing 200 items  | Latency            | **426 µs**    |
| Dir listing 1000 items | Latency            | **2.04 ms**   |
| OPTIONS                | Latency            | **2.93 µs**   |
| PROPFIND depth:0       | Latency            | **34 µs**     |
| PROPFIND depth:1 (200) | Latency            | **1.15 ms**   |
| PROPFIND depth:inf     | Latency (3×5 tree) | **309 µs**    |
| MKCOL                  | Latency            | **260 µs**    |
| COPY file              | Latency            | **442 µs**    |
| COPY dir tree          | Latency            | **6.21 ms**   |
| MOVE file              | Latency            | **514 µs**    |
| LOCK exclusive         | Latency            | **252 µs**    |
| UNLOCK                 | Latency            | **369 µs**    |
| HealthCheck intercept  | Latency            | **1.10 µs**   |
| Auth plaintext valid   | Latency            | **42 µs**     |
| Auth SHA-512 cached    | Latency            | **42.0 µs**   |
| Auth SHA-512 cold miss | Latency            | **~536 µs**   |
| Auth SHA-512 invalid   | Latency            | **536 µs**    |
| SHA-512 crypt (pure)   | Latency            | **523 µs**    |
| Lock enforce reject    | Latency            | **297 µs**    |
| Ancestor lock reject   | Latency            | **432 µs**    |
| Cold GET (new dir)     | Latency            | **268 µs**    |
| Hot GET (reuse)        | Latency            | **42.2 µs**   |
| Browser browse (3 rqs) | Latency            | **904 µs**    |
| WebDAV sync (6 reqs)   | Latency            | **2.48 ms**   |
| Lock-edit-unlock       | Latency            | **355 µs**    |
| Mixed workload (8 rqs) | Latency            | **3.75 ms**   |
| If-header parse        | Latency            | **110 ns**    |
| PROPFIND body parse    | Latency            | **351 ns**    |

---

## Fileserver Core

### GET — File Serving

GET benchmarks measure two independent dimensions of read performance:

| File Size | Dispatch Latency | Body-Drain Latency | Read Throughput |
| --------- | ---------------- | ------------------ | --------------- |
| 13 B      | **42.5 µs**      | —                  | —               |
| 1 KB      | **42.3 µs**      | —                  | —               |
| 64 KB     | **42.4 µs**      | **119 µs**         | **528 MiB/s**   |
| 1 MB      | **42.3 µs**      | **1.13 ms**        | **888 MiB/s**   |
| 10 MB     | **42.4 µs**      | **12.1 ms**        | **828 MiB/s**   |

> **Dispatch latency** (~42µs, constant regardless of file size): The time from
> request arrival to the handler returning control. This reflects the server's
> concurrency ceiling — it can accept ~24,000 requests/second. Measured via
> `oneshot()` against the router (headers-only, representing when the async
> handler releases back to the runtime).
>
> **Body-drain latency** (119µs–12.1ms): The time to fully read the file from
> disk and stream all bytes through the response. Scales linearly with file
> size. Measured by draining the response body via `to_bytes()`.
>
> **Read vs write comparison** (10MB):
>
> | Direction        | Latency     | Throughput    |
> | ---------------- | ----------- | ------------- |
> | GET (body-drain) | **12.1 ms** | **828 MiB/s** |
> | PUT              | **14.4 ms** | **693 MiB/s** |
>
> Read is ~12% faster than write — expected on APFS, where writes incur
> additional flush overhead.

### PUT — File Upload

| Scenario        | Latency     | Throughput    |
| --------------- | ----------- | ------------- |
| Overwrite 1KB   | **63 µs**   | 15.4 MiB/s    |
| New file 1KB    | **94 µs**   | 10.4 MiB/s    |
| Large file 10MB | **14.4 ms** | **693 MiB/s** |

### DELETE — File and Directory Trees

| Scenario               | Files Removed | Latency     |
| ---------------------- | ------------- | ----------- |
| Single file            | 1             | **283 µs**  |
| Depth 2 directory tree | ~20           | **2.87 ms** |
| Depth 3 directory tree | ~30           | **4.63 ms** |
| Depth 5 directory tree | ~50           | **7.06 ms** |

> Scaling is approximately linear with file count. `remove_dir_all` is the dominant cost.

### Directory Listing (HTML)

| Items | Latency     | Throughput (items/s) |
| ----- | ----------- | -------------------- |
| 10    | **67 µs**   | 148 K/s              |
| 50    | **143 µs**  | 349 K/s              |
| 200   | **426 µs**  | 469 K/s              |
| 1000  | **2.04 ms** | 491 K/s              |

> Per-entry cost has dropped from ~6µs to ~2µs. This is the result of
> `batch_read_dir_entries` — all entries' metadata is collected in a single
> `spawn_blocking` call instead of one per entry. On Linux, io_uring further
> reduces the per-statx syscall overhead (see §Linux / io_uring).

---

## WebDAV Protocol

### PROPFIND — Property Retrieval

| Scenario                  | Entries | Latency     | Per-entry |
| ------------------------- | ------- | ----------- | --------- |
| Depth:0 single file       | 1       | **34 µs**   | 34 µs     |
| Depth:1 dir (10 files)    | 11      | **108 µs**  | 9.8 µs    |
| Depth:1 dir (50 files)    | 51      | **332 µs**  | 6.5 µs    |
| Depth:1 dir (200 files)   | 201     | **1.15 ms** | 5.7 µs    |
| Depth:1 dir (1000 files)  | 1001    | **5.96 ms** | 6.0 µs    |
| Depth:infinity (3×5 tree) | ~20     | **309 µs**  | 15.5 µs   |

> Per-entry cost is now dominated by XML generation (~3µs/entry) and lock/dead-property
> lookups — not filesystem traversal. The `batch_read_dir_entries` optimization
> (single `spawn_blocking` for all entries) has moved the bottleneck elsewhere.
> On Linux with io_uring, per-entry cost drops further (see §Linux / io_uring).

### XML Generation — Micro-benchmark

| Entries | Latency     | Per-entry |
| ------- | ----------- | --------- |
| 1       | **3.63 µs** | 3.63 µs   |
| 10      | **31.4 µs** | 3.14 µs   |
| 100     | **312 µs**  | 3.12 µs   |
| 1000    | **3.11 ms** | 3.11 µs   |

> XML generation is efficient (~3µs/entry), scaling linearly. Not a bottleneck.

### Lock Operations

| Operation      | Latency    |
| -------------- | ---------- |
| LOCK exclusive | **264 µs** |
| LOCK shared    | **252 µs** |
| UNLOCK         | **373 µs** |

### COPY / MOVE

| Operation       | Latency     |
| --------------- | ----------- |
| COPY small file | **438 µs**  |
| COPY dir tree   | **6.16 ms** |
| MOVE small file | **490 µs**  |

---

## Middleware Cost Breakdown

### Health Check

| Scenario           | Latency     |
| ------------------ | ----------- |
| Intercept (200 OK) | **1.06 µs** |
| Passthrough GET    | **42.6 µs** |

> HealthCheck intercepts before any downstream middleware runs. 1.06µs is effectively pure tower overhead.

### Authentication

Auth caching reduces repeated SHA-512 crypt verification overhead:

| Scenario                | Latency     | Cache? | Δ from no-auth         |
| ----------------------- | ----------- | ------ | ---------------------- |
| No users (noop)         | **42.6 µs** | —      | —                      |
| Plaintext valid         | **42.8 µs** | —      | **+0.2 µs**            |
| Plaintext invalid (401) | **2.35 µs** | —      | shorter (early return) |
| SHA-512 valid (cached)  | **42.6 µs** | hit    | **±0 µs**              |
| SHA-512 valid (miss)    | **~572 µs** | miss   | **+530 µs**            |
| SHA-512 invalid (401)   | **537 µs**  | no     | **+494 µs**            |

> Cache hits complete in **~43µs** — identical to the no-auth baseline.
> Cache misses fall through to `spawn_blocking` SHA-512 crypt verification
> (**528µs** raw cost), isolating the expensive work onto the blocking thread
> pool so async worker threads remain free.
>
> Failed authentications are **never cached**, maintaining brute-force resistance.
> Default TTL is 60 seconds, configurable via `--auth-cache-ttl` (set to 0 to
> disable caching entirely). Password changes take effect after at most the TTL
> window.

### Lock Enforcement (Middleware)

| Scenario                          | Latency    |
| --------------------------------- | ---------- |
| PUT unlocked (passthrough)        | **63 µs**  |
| PUT locked without token → 423    | **305 µs** |
| PUT locked with matching If token | **254 µs** |
| PUT ancestor locked (depth:inf)   | **425 µs** |

> Lock enforce adds ~2µs overhead on unlocked resources (evaluating the If-condition against an empty store via lock-count shortcut). Full evaluation (If-header parse + ancestor walk + exclusive check) adds ~242µs for rejected requests. Ancestor chain traversal costs an extra ~120µs per depth level.

---

## Path Resolution

| Scenario              | Latency    | Δ vs shallow |
| --------------------- | ---------- | ------------ |
| PUT shallow (1 level) | **261 µs** | —            |
| PUT deep (5 levels)   | **790 µs** | **+529 µs**  |
| PUT percent-encoded   | **262 µs** | **0 µs**     |
| GET shallow (1 level) | **266 µs** | —            |
| GET deep (5 levels)   | **836 µs** | **+570 µs**  |
| GET UTF-8 encoded     | **265 µs** | **0 µs**     |

> Each additional path depth adds ~**130µs** from `tokio::fs::canonicalize` syscalls. Percent-encoding and UTF-8 paths impose negligible overhead.

### Cold vs Hot Cache

| Scenario                      | Latency     | Ratio |
| ----------------------------- | ----------- | ----- |
| Cold (fresh TempDir per iter) | **267 µs**  | 6.3×  |
| Hot (reuse same TempDir)      | **42.6 µs** | 1×    |

> Filesystem metadata caching by the OS accounts for **~224µs per request** (83% of GET latency). On hot caches, `canonicalize` + `metadata` become nearly free.

---

## End-to-End Scenarios

| Scenario                                                | Requests | Latency     | Avg/req |
| ------------------------------------------------------- | -------- | ----------- | ------- |
| Browser browse (GET /, /images/, file)                  | 3        | **947 µs**  | 316 µs  |
| WebDAV sync (PROPFIND d:1 + 5×GET)                      | 6        | **2.69 ms** | 448 µs  |
| Lock → edit (PUT with If) → unlock                      | 3        | **367 µs**  | 122 µs  |
| Mixed workload (5 GET + 1 PROPFIND + 1 PUT + 1 OPTIONS) | 8        | **3.85 ms** | 481 µs  |

> The mixed workload (80% GET, 15% PROPFIND, 5% PUT) on a 30-file directory completes 8 requests in ~3.9ms — **~2100 mixed requests/second** through the full middleware stack.

---

## Hot Path Analysis

### Bottleneck Ranking

| Rank | Component                   | Cost              | % of request (typ.) | Status            |
| ---- | --------------------------- | ----------------- | ------------------- | ----------------- |
| 1    | **fs::canonicalize (cold)** | ~224 µs           | 83% (cold GET)      | OS cache          |
| 2    | **SHA-512 crypt verify**    | 528 µs            | 92% (first auth)    | ✅ Cached         |
| 3    | **read_dir + metadata**     | ~2 µs/entry       | ~60% (dir listing)  | ✅ Batched        |
| 4    | **Ancestor lock walk**      | → 63µs (was 95µs) | passthrough         | ✅ Improved       |
| 5    | **PROPFIND fs traversal**   | → batched         | (was 97%)           | ✅ io_uring batch |

> - **PROPFIND fs traversal (#5)**: Replaced per-entry `tokio::fs::metadata()` (serial,
>   one `spawn_blocking` per entry) with a single `spawn_blocking` call that
>   enumerates the directory via `std::fs::read_dir` and batches all `statx`
>   metadata calls. On Linux, uses `io_uring` (`IORING_OP_STATX`) to submit all
>   `statx` calls in a single `io_uring_enter` syscall. Non-Linux platforms fall
>   back to serial `std::fs::metadata()` calls inside the single `spawn_blocking` —
>   still a significant improvement by eliminating per-entry tokio scheduling
>   overhead. The same optimization applies to HTML directory listing.
>   See `src/scandir.rs` for implementation details.

### Low-cost / Optimal Paths

| Component                    | Cost           | Notes                              |
| ---------------------------- | -------------- | ---------------------------------- |
| Method dispatch (`try_from`) | **1.9 ns**     | Essentially free                   |
| If-header parsing            | **111 ns**     | Handwritten parser, zero-alloc     |
| Header parsing               | **16–113 ns**  | Depth, Timeout, Destination, etc.  |
| XML generation               | **3 µs/entry** | Linear scaling, allocation-minimal |
| Lock token check             | **6 ns**       | Single iteration, short-circuit    |

---

## Linux / io_uring

io_uring batch statx was validated in a Linux VM (ext4). VM results carry
hypervisor overhead — the focus is on **relative scaling**, not absolute
numbers.

### Directory Listing — Pure `batch_read_dir_entries`

Since directory listing has no XML generation or lock lookups, it isolates
the `batch_read_dir_entries` cost.

| Items | macOS (native) | Linux (VM, io_uring) | Winner          |
| ----- | -------------- | -------------------- | --------------- |
| 10    | **67 µs**      | 171 µs               | macOS 2.6×      |
| 50    | **143 µs**     | 213 µs               | macOS 1.5×      |
| 200   | 426 µs         | **352 µs**           | **Linux 1.21×** |
| 1000  | 2.04 ms        | **1.14 ms**          | **Linux 1.79×** |

> macOS numbers include the `std::fs::metadata(entry.path())` symlink-following
> overhead (~14–18% vs `entry.metadata()`). Despite this, per-entry
> performance is ~3× faster than v0.8.4.
>
> **Key observation**: the crossover is at ~200 entries in the VM — near
> `BATCH_SIZE` (256). Below that, io_uring setup cost exceeds the benefit
> of batching a few `statx` calls. Above it, the advantage grows: 1.21×
> at 200, 1.79× at 1000. The trend is unambiguous.

### PROPFIND — End-to-End with XML + Lock Lookups

| Scenario        | macOS      | Linux (VM)  | Winner          |
| --------------- | ---------- | ----------- | --------------- |
| depth:0         | **34 µs**  | 79 µs       | macOS 2.3×      |
| depth:1 10      | **108 µs** | 208 µs      | macOS 1.9×      |
| depth:1 50      | **332 µs** | 374 µs      | macOS 1.1×      |
| depth:1 200     | 1.15 ms    | **950 µs**  | **Linux 1.21×** |
| depth:1 1000    | 5.96 ms    | **4.13 ms** | **Linux 1.44×** |
| depth:inf (~20) | **309 µs** | 695 µs      | macOS 2.2×      |

> Same crossover pattern as directory listing. At depth:inf (~20 entries)
> macOS still leads — the tree is too small to benefit from batching on
> either platform.

### Practical Impact

At 10 entries — the typical case for most real-world directories — both
platforms deliver sub-millisecond PROPFIND (104 µs macOS, 208 µs Linux VM).
The client cannot perceive the difference.

io*uring batch statx targets the \_long tail*: directories with 200+
entries where serial `fstatat` overhead becomes multiplicative. A single
code path for all directory sizes avoids the complexity of a
threshold-based dispatch between serial and batch stat.

---

## Conclusions

### Performance Profile

- **Dispatch latency is flat**: GET ~42µs regardless of file size — `canonicalize` + `metadata` + `open` dominate.
- **Body-drain throughput is high**: 828 MiB/s read, 693 MiB/s write. Read is ~12% faster than write (expected: writes incur flush overhead).
- **WebDAV PROPFIND is fast**: After batch `spawn_blocking`, per-entry cost is ~5µs (macOS),
  dominated by XML generation (~3µs) and lock/dead-property lookups — not I/O.
- **SHA-512 auth with caching**: First request costs 528µs (blocking thread pool, not worker threads). Subsequent requests within the 60s TTL hit the auth cache: 528µs → <1µs per verification (43µs total dispatch). Configurable via `--auth-cache-ttl` (0 = disable).
- **Path depth matters**: 5-level deep paths cost 3× more than single-level — `canonicalize` per component.
- **Concurrency ceiling**: In hot-cache scenarios, each GET dispatch takes ~42µs. Ceiling: **~24,000 requests/second** (single-core).

### Understanding GET Latency

The benchmark report presents two GET latency numbers for the same file:

- **Dispatch latency (42µs)**: Measures the async handler's dispatch time — when the handler returns control to the runtime, free to accept the next request. This is the correct metric for server concurrency.
- **Body-drain latency (119µs–12.1ms)**: Measures full read + stream time including disk I/O. Comparable to PUT write latency and useful for throughput analysis.

Both numbers are accurate — they measure different phases of the same HTTP transaction.

### Scaling Guidance

| Directory Size | PROPFIND depth:1 | Dir Listing HTML |
| -------------- | ---------------- | ---------------- |
| 10 files       | 108 µs           | 67 µs            |
| 50 files       | 332 µs           | 143 µs           |
| 100 files      | ~600 µs          | ~280 µs          |
| 200 files      | 1.15 ms          | 426 µs           |
| 1000 files     | 5.96 ms          | 2.04 ms          |

> PROPFIND per-entry overhead over plain directory listing is now ~3-4µs,
> attributable to XML generation (~3µs) plus lock/dead-property lookups.
> For directories with >1000 files, PROPFIND depth:1 still completes in ~5ms —
> well within typical WebDAV client timeouts.
