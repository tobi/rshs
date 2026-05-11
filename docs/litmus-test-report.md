# Litmus WebDAV Conformance Test Report

rshs uses [litmus](http://www.webdav.org/neon/litmus/) to verify WebDAV protocol compliance.
Tests are run against a local server instance with Basic Auth.

<details>

<summary>Test Output (with `FakeLs` lock system)</summary>

```log
-> running `basic':
 0. init.................. pass
 1. begin................. pass
 2. options............... pass
 3. put_get............... pass
 4. put_get_utf8_segment.. pass
 5. put_no_parent......... pass
 6. mkcol_over_plain...... pass
 7. delete................ pass
 8. delete_null........... pass
 9. delete_fragment....... WARNING: DELETE removed collection resource with Request-URI including fragment; unsafe
    ...................... pass (with 1 warning)
10. mkcol................. pass
11. mkcol_again........... pass
12. delete_coll........... pass
13. mkcol_no_parent....... pass
14. mkcol_with_body....... pass
15. finish................ pass
<- summary for `basic': of 16 tests run: 16 passed, 0 failed. 100.0%
-> 1 warning was issued.
-> running `http':
 0. init.................. pass
 1. begin................. pass
 2. expect100............. pass
 3. finish................ pass
<- summary for `http': of 4 tests run: 4 passed, 0 failed. 100.0%
-> running `copymove':
 0. init.................. pass
 1. begin................. pass
 2. copy_init............. pass
 3. copy_simple........... pass
 4. copy_overwrite........ pass
 5. copy_nodestcoll....... pass
 6. copy_cleanup.......... pass
 7. copy_coll............. pass
 8. copy_shallow.......... pass
 9. move.................. pass
10. move_coll............. pass
11. move_cleanup.......... pass
12. finish................ pass
<- summary for `copymove': of 13 tests run: 13 passed, 0 failed. 100.0%
-> running `locks':
 0. init.................. pass
 1. begin................. pass
 2. options............... pass
 3. precond............... pass
 4. init_locks............ pass
 5. put................... pass
 6. lock_excl............. pass
 7. discover.............. pass
 8. refresh............... pass
 9. notowner_modify....... FAIL (DELETE of locked resource should fail)
10. notowner_lock......... FAIL (UNLOCK with bogus lock token)
11. owner_modify.......... FAIL (PROPPATCH on locked resouce on `/litmus/lockme': 501 Not Implemented)
12. notowner_modify....... FAIL (DELETE of locked resource should fail)
13. notowner_lock......... FAIL (UNLOCK with bogus lock token)
14. copy.................. FAIL (could not COPY locked resource:
404 Not Found)
15. cond_put.............. SKIPPED
16. fail_cond_put......... SKIPPED
17. cond_put_with_not..... pass
18. cond_put_corrupt_token FAIL (conditional PUT with invalid lock-token should fail: 204 No Content)
19. complex_cond_put...... SKIPPED
20. fail_complex_cond_put. SKIPPED
21. unlock................ pass
22. fail_cond_put_unlocked pass
23. lock_shared........... pass
24. notowner_modify....... FAIL (DELETE of locked resource should fail)
25. notowner_lock......... FAIL (UNLOCK with bogus lock token)
26. owner_modify.......... FAIL (PROPPATCH on locked resouce on `/litmus/lockme': 501 Not Implemented)
27. double_sharedlock..... pass
28. notowner_modify....... FAIL (DELETE of locked resource should fail)
29. notowner_lock......... FAIL (UNLOCK with bogus lock token)
30. unlock................ pass
31. prep_collection....... pass
32. lock_collection....... pass
33. owner_modify.......... FAIL (PROPPATCH on locked resouce on `/litmus/lockcoll/lockme.txt': 501 Not Implemented)
34. notowner_modify....... FAIL (DELETE of locked resource should fail)
35. refresh............... pass
36. indirect_refresh...... pass
37. unlock................ pass
38. unmapped_lock......... pass
39. unlock................ pass
40. finish................ pass
-> 4 tests were skipped.
<- summary for `locks': of 37 tests run: 23 passed, 14 failed. 62.2%
See debug.log for network/debug traces.
```

</details>

<details>

<summary>Test Output (with `MemLs` lock system)</summary>

```log
-> running `basic':
 0. init.................. pass
 1. begin................. pass
 2. options............... pass
 3. put_get............... pass
 4. put_get_utf8_segment.. pass
 5. put_no_parent......... pass
 6. mkcol_over_plain...... pass
 7. delete................ pass
 8. delete_null........... pass
 9. delete_fragment....... WARNING: DELETE removed collection resource with Request-URI including fragment; unsafe
    ...................... pass (with 1 warning)
10. mkcol................. pass
11. mkcol_again........... pass
12. delete_coll........... pass
13. mkcol_no_parent....... pass
14. mkcol_with_body....... pass
15. finish................ pass
<- summary for `basic': of 16 tests run: 16 passed, 0 failed. 100.0%
-> 1 warning was issued.
-> running `http':
 0. init.................. pass
 1. begin................. pass
 2. expect100............. pass
 3. finish................ pass
<- summary for `http': of 4 tests run: 4 passed, 0 failed. 100.0%
-> running `copymove':
 0. init.................. pass
 1. begin................. pass
 2. copy_init............. pass
 3. copy_simple........... pass
 4. copy_overwrite........ pass
 5. copy_nodestcoll....... pass
 6. copy_cleanup.......... pass
 7. copy_coll............. pass
 8. copy_shallow.......... pass
 9. move.................. pass
10. move_coll............. pass
11. move_cleanup.......... pass
12. finish................ pass
<- summary for `copymove': of 13 tests run: 13 passed, 0 failed. 100.0%
-> running `locks':
 0. init.................. pass
 1. begin................. pass
 2. options............... pass
 3. precond............... pass
 4. init_locks............ pass
 5. put................... pass
 6. lock_excl............. pass
 7. discover.............. pass
 8. refresh............... pass
 9. notowner_modify....... pass
10. notowner_lock......... pass
11. owner_modify.......... FAIL (PROPPATCH on locked resouce on `/litmus/lockme': 501 Not Implemented)
12. notowner_modify....... pass
13. notowner_lock......... pass
14. copy.................. pass
15. cond_put.............. SKIPPED
16. fail_cond_put......... SKIPPED
17. cond_put_with_not..... pass
18. cond_put_corrupt_token pass
19. complex_cond_put...... SKIPPED
20. fail_complex_cond_put. SKIPPED
21. unlock................ pass
22. fail_cond_put_unlocked pass
23. lock_shared........... pass
24. notowner_modify....... pass
25. notowner_lock......... pass
26. owner_modify.......... FAIL (PROPPATCH on locked resouce on `/litmus/lockme': 501 Not Implemented)
27. double_sharedlock..... pass
28. notowner_modify....... pass
29. notowner_lock......... pass
30. unlock................ pass
31. prep_collection....... pass
32. lock_collection....... pass
33. owner_modify.......... FAIL (PROPPATCH on locked resouce on `/litmus/lockcoll/lockme.txt': 501 Not Implemented)
34. notowner_modify....... pass
35. refresh............... pass
36. indirect_refresh...... pass
37. unlock................ pass
38. unmapped_lock......... pass
39. unlock................ pass
40. finish................ pass
-> 4 tests were skipped.
<- summary for `locks': of 37 tests run: 34 passed, 3 failed. 91.9%
See debug.log for network/debug traces.
```

</details>

## Results Summary

| Test Suite  | FakeLs (v0.5.1)    | MemLs (v0.6.0)     |
| ----------- | ------------------ | ------------------ |
| `http`      | 4/4 PASS (100%)    | 4/4 PASS (100%)    |
| `basic`     | 16/16 PASS (100%)  | 16/16 PASS (100%)  |
| `copymove`  | 13/13 PASS (100%)  | 13/13 PASS (100%)  |
| `locks`     | 23/37 PASS (62.2%) | 34/37 PASS (91.9%) |
| **Overall** | **56/70 (80.0%)**  | **67/70 (95.7%)**  |

> **Note:** `locks` has 4 skipped tests (`cond_put`, `fail_cond_put`, `complex_cond_put`,
> `fail_complex_cond_put`) that require `<D:owner>` in lock requests; litmus skips them
> automatically. These are not counted as failures in either run.

## Locks Test Improvements (MemLs)

The switch from `FakeLs` to `MemLs` resolved 11 previously-failing tests:

| Test (× occurrences)          | FakeLs | MemLs | Description                               |
| ----------------------------- | ------ | ----- | ----------------------------------------- |
| `notowner_modify` (×5)        | FAIL   | PASS  | DELETE on locked resource without token   |
| `notowner_lock` (×4)          | FAIL   | PASS  | UNLOCK with an invalid/bogus lock token   |
| `copy` (×1)                   | FAIL   | PASS  | COPY of a locked resource (cascading fix) |
| `cond_put_corrupt_token` (×1) | FAIL   | PASS  | Conditional PUT with corrupt lock token   |

### Before/After Debug Log Comparison

**notowner_modify (FakeLs) — DELETE succeeds incorrectly:**

```log
dav_server::davhandler: == END REQUEST result OK
tower_http::trace::on_response: status=204
```

**notowner_modify (MemLs) — DELETE correctly rejected:**

```log
dav_server::davhandler: == END REQUEST result StatusClose(423)
tower_http::trace::on_response: status=423
```

**copy (FakeLs) — cascading failure due to premature DELETE:**

```log
dav_server::davhandler: == END REQUEST result FsError(NotFound)
tower_http::trace::on_response: status=404
```

**copy (MemLs) — file preserved, copy succeeds:**

```log
dav_server::davhandler: == END REQUEST result OK
tower_http::trace::on_response: status=201
```

## Remaining Failures (3)

| Test (× occurrences) | Status | Root Cause                      |
| -------------------- | ------ | ------------------------------- |
| `owner_modify` (×3)  | FAIL   | PROPPATCH → 501 Not Implemented |

These failures are **not related to the lock system**. They occur because `LocalFs`
(the dav-server filesystem backend) does not support writing WebDAV properties
(PROPPATCH). Resolving these would require either:

- Using a different filesystem backend that supports custom properties
- Implementing property storage in `LocalFs` upstream

This limitation does not affect common WebDAV use cases (file upload/download,
directory listing, lock/unlock, copy/move).

## Test Configuration

```sh
# Server startup
cargo run -- . -u admin:secret123

# Litmus invocation
TESTS="basic http copymove locks props" TESTROOT=. ./litmus http://localhost:8080 admin pass
```
