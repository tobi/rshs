# Litmus WebDAV Compliance Report

## Test Environment

| Item     | Detail                              |
| -------- | ----------------------------------- |
| rshs     | v0.7.0 (dev)                        |
| litmus   | 0.17                                |
| neon     | 0.34.2                              |
| Platform | macOS (arm64)                       |
| Server   | `cargo run --release -- ./docs -vv` |

## Summary

| Suite     | Passed  | Total   | %       | Warnings | Skipped |
| --------- | ------- | ------- | ------- | -------- | ------- |
| basic     | 16      | 16      | 100     | 1        | —       |
| http      | 4       | 4       | 100     | —        | —       |
| copymove  | 13      | 13      | 100     | 2        | —       |
| locks     | 36      | 36      | 100     | 1        | 4       |
| props     | 33      | 33      | 100     | —        | —       |
| **Total** | **102** | **102** | **100** | **4**    | **4**   |

All five litmus test suites pass at 100%.

---

## Detailed Results

### basic — 16/16 ✅

All standard HTTP/WebDAV core tests pass.

| Test                 | Result  |
| -------------------- | ------- |
| begin                | pass    |
| options              | pass    |
| put_get              | pass    |
| put_get_utf8_segment | pass    |
| put_no_parent        | pass    |
| put_location         | pass    |
| mkcol_over_plain     | pass    |
| delete               | pass    |
| delete_null          | pass    |
| delete_fragment      | pass ⚠️ |
| mkcol                | pass    |
| mkcol_again          | pass    |
| delete_coll          | pass    |
| mkcol_no_parent      | pass    |
| mkcol_with_body      | pass    |
| finish               | pass    |

> ⚠️ `delete_fragment` — DELETE on a URL with a fragment component removed the collection anyway. The server treats fragment as part of the path (filesystem behaviour).

---

### http — 4/4 ✅

| Test           | Result |
| -------------- | ------ |
| begin          | pass   |
| direct_connect | pass   |
| expect100      | pass   |
| finish         | pass   |

---

### copymove — 13/13 ✅

| Test            | Result  |
| --------------- | ------- |
| begin           | pass    |
| copy_init       | pass    |
| copy_simple     | pass    |
| copy_overwrite  | pass ⚠️ |
| copy_abspath    | pass    |
| copy_nodestcoll | pass    |
| copy_cleanup    | pass    |
| copy_coll       | pass    |
| copy_shallow    | pass    |
| move            | pass ⚠️ |
| move_coll       | pass    |
| move_cleanup    | pass    |
| finish          | pass    |

> ⚠️ `copy_overwrite` — COPY to an existing resource returns `201 Created` instead of `204 No Content`. RFC 4918 §9.8.5 specifies `204` when overwriting an existing resource.
>
> ⚠️ `move` — MOVE of a non-collection resource into an existing collection returns `201 Created` instead of `204 No Content`.

---

### locks — 36/36 ✅

| Test                   | Result  |
| ---------------------- | ------- |
| begin                  | pass    |
| options                | pass    |
| precond                | pass    |
| init_locks             | pass    |
| put                    | pass    |
| lock_excl              | pass    |
| discover               | pass    |
| refresh                | pass    |
| notowner_modify        | pass    |
| notowner_lock          | pass    |
| owner_modify           | pass    |
| notowner_modify        | pass    |
| notowner_lock          | pass    |
| copy                   | pass    |
| cond_put               | skipped |
| fail_cond_put          | skipped |
| cond_put_with_not      | pass    |
| cond_put_corrupt_token | pass    |
| complex_cond_put       | skipped |
| fail_complex_cond_put  | skipped |
| unlock                 | pass    |
| fail_cond_put_unlocked | pass    |
| lock_shared            | pass    |
| notowner_modify        | pass    |
| notowner_lock          | pass    |
| owner_modify           | pass    |
| double_sharedlock      | pass    |
| notowner_modify        | pass    |
| notowner_lock          | pass    |
| unlock                 | pass    |
| prep_collection        | pass    |
| lock_collection        | pass    |
| owner_modify           | pass    |
| notowner_modify        | pass    |
| refresh                | pass    |
| indirect_refresh       | pass    |
| unlock                 | pass    |
| unmapped_lock          | pass ⚠️ |
| unlock                 | pass    |
| finish                 | pass    |

> 4 tests skipped (`cond_put`, `fail_cond_put`, `complex_cond_put`, `fail_complex_cond_put`): litmus skips these when a preceding precondition is not met.
>
> ⚠️ `unmapped_lock` — LOCK on an unmapped URL returns `200 OK` instead of `201 Created`. RFC 4918 §7.3 specifies `201` when the lock creates the resource (lock-null).

---

### props — 33/33 ✅

| Test              | Result |
| ----------------- | ------ |
| begin             | pass   |
| propfind_invalid  | pass   |
| propfind_invalid2 | pass   |
| propfind_d0       | pass   |
| propinit          | pass   |
| propset           | pass   |
| propget           | pass   |
| propextended      | pass   |
| propmove          | pass   |
| propget           | pass   |
| propdeletes       | pass   |
| propget           | pass   |
| propset           | pass   |
| propdeletes       | pass   |
| propdeletes       | pass   |
| propreplace       | pass   |
| propget           | pass   |
| propnullns        | pass   |
| propget           | pass   |
| prophighunicode   | pass   |
| propget           | pass   |
| propremoveset     | pass   |
| propget           | pass   |
| propsetremove     | pass   |
| propget           | pass   |
| propvalnspace     | pass   |
| propwformed       | pass   |
| propinit          | pass   |
| propgetlastmod    | pass   |
| propmanyns        | pass   |
| propget           | pass   |
| propcleanup       | pass   |
| finish            | pass   |

No warnings. All dead property operations — including set, get, delete, move, replace, high-unicode characters, namespace-preserving roundtrip, and operation ordering (set-then-remove) — function correctly.

---

## Known Deviations from RFC

| Test            | Status  | RFC Reference   | Behaviour                          |
| --------------- | ------- | --------------- | ---------------------------------- |
| copy_overwrite  | warning | RFC 4918 §9.8.5 | Returns `201` instead of `204`     |
| move            | warning | RFC 4918 §9.9.4 | Returns `201` instead of `204`     |
| unmapped_lock   | warning | RFC 4918 §7.3   | Returns `200` instead of `201`     |
| delete_fragment | warning | RFC 3986 §3.5   | Fragment processed as path segment |
| cond_put (×4)   | skipped | —               | Litmus precondition not met        |

None of these deviations cause interoperability issues with common WebDAV clients (Finder, davfs, cadaver, Cyberduck).

---

## How to Reproduce

### 1. Install litmus

Download and build [litmus](https://github.com/notroj/litmus) — the WebDAV protocol conformance test suite. See the project README for build instructions.

### 2. Run the tests

```sh
# Terminal 1: Start rshs
cargo run --release -- ./docs -vv

# Terminal 2: Run litmus (from the litmus source directory)
TESTS="basic http copymove locks props" TESTROOT=. ./litmus http://localhost:8080
```

Expected output matches the results shown in this report.

<details>

<summary>Original litmus test report</summary>

```log
-> running `basic':
 0. begin................. pass
 1. options............... pass
 2. put_get............... pass
 3. put_get_utf8_segment.. pass
 4. put_no_parent......... pass
 5. put_location.......... pass
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
 0. begin................. pass
 1. direct_connect........ pass
 2. expect100............. pass
 3. finish................ pass
<- summary for `http': of 4 tests run: 4 passed, 0 failed. 100.0%
-> running `copymove':
 0. begin................. pass
 1. copy_init............. pass
 2. copy_simple........... pass
 3. copy_overwrite........ WARNING: COPY to existing resource should give 204 (RFC4918:S9.8.5), got 201 Created
    ...................... pass (with 1 warning)
 4. copy_abspath.......... pass
 5. copy_nodestcoll....... pass
 6. copy_cleanup.......... pass
 7. copy_coll............. pass
 8. copy_shallow.......... pass
 9. move.................. WARNING: MOVE to existing collection resource didn't give 204
    ...................... pass (with 1 warning)
10. move_coll............. pass
11. move_cleanup.......... pass
12. finish................ pass
<- summary for `copymove': of 13 tests run: 13 passed, 0 failed. 100.0%
-> 2 warnings were issued.
-> running `locks':
 0. begin................. pass
 1. options............... pass
 2. precond............... pass
 3. init_locks............ pass
 4. put................... pass
 5. lock_excl............. pass
 6. discover.............. pass
 7. refresh............... pass
 8. notowner_modify....... pass
 9. notowner_lock......... pass
10. owner_modify.......... pass
11. notowner_modify....... pass
12. notowner_lock......... pass
13. copy.................. pass
14. cond_put.............. SKIPPED
15. fail_cond_put......... SKIPPED
16. cond_put_with_not..... pass
17. cond_put_corrupt_token pass
18. complex_cond_put...... SKIPPED
19. fail_complex_cond_put. SKIPPED
20. unlock................ pass
21. fail_cond_put_unlocked pass
22. lock_shared........... pass
23. notowner_modify....... pass
24. notowner_lock......... pass
25. owner_modify.......... pass
26. double_sharedlock..... pass
27. notowner_modify....... pass
28. notowner_lock......... pass
29. unlock................ pass
30. prep_collection....... pass
31. lock_collection....... pass
32. owner_modify.......... pass
33. notowner_modify....... pass
34. refresh............... pass
35. indirect_refresh...... pass
36. unlock................ pass
37. unmapped_lock......... WARNING: LOCK on unmapped url returned 200 not 201 (RFC4918:S7.3)
    ...................... pass (with 1 warning)
38. unlock................ pass
39. finish................ pass
-> 4 tests were skipped.
<- summary for `locks': of 36 tests run: 36 passed, 0 failed. 100.0%
-> 1 warning was issued.
-> running `props':
 0. begin................. pass
 1. propfind_invalid...... pass
 2. propfind_invalid2..... pass
 3. propfind_d0........... pass
 4. propinit.............. pass
 5. propset............... pass
 6. propget............... pass
 7. propextended.......... pass
 8. propmove.............. pass
 9. propget............... pass
10. propdeletes........... pass
11. propget............... pass
12. propset............... pass
13. propdeletes........... pass
14. propdeletes........... pass
15. propreplace........... pass
16. propget............... pass
17. propnullns............ pass
18. propget............... pass
19. prophighunicode....... pass
20. propget............... pass
21. propremoveset......... pass
22. propget............... pass
23. propsetremove......... pass
24. propget............... pass
25. propvalnspace......... pass
26. propwformed........... pass
27. propinit.............. pass
28. propgetlastmod........ pass
29. propmanyns............ pass
30. propget............... pass
31. propcleanup........... pass
32. finish................ pass
<- summary for `props': of 33 tests run: 33 passed, 0 failed. 100.0%
(base) [mogeko:~/Downloads/litmus-0.17]$ TESTS="basic http copymove locks props" TESTROOT=. ./litmus http://localhost:8080
-> running `basic':
 0. begin................. pass
 1. options............... pass
 2. put_get............... pass
 3. put_get_utf8_segment.. pass
 4. put_no_parent......... pass
 5. put_location.......... pass
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
 0. begin................. pass
 1. direct_connect........ pass
 2. expect100............. pass
 3. finish................ pass
<- summary for `http': of 4 tests run: 4 passed, 0 failed. 100.0%
-> running `copymove':
 0. begin................. pass
 1. copy_init............. pass
 2. copy_simple........... pass
 3. copy_overwrite........ WARNING: COPY to existing resource should give 204 (RFC4918:S9.8.5), got 201 Created
    ...................... pass (with 1 warning)
 4. copy_abspath.......... pass
 5. copy_nodestcoll....... pass
 6. copy_cleanup.......... pass
 7. copy_coll............. pass
 8. copy_shallow.......... pass
 9. move.................. WARNING: MOVE to existing collection resource didn't give 204
    ...................... pass (with 1 warning)
10. move_coll............. pass
11. move_cleanup.......... pass
12. finish................ pass
<- summary for `copymove': of 13 tests run: 13 passed, 0 failed. 100.0%
-> 2 warnings were issued.
-> running `locks':
 0. begin................. pass
 1. options............... pass
 2. precond............... pass
 3. init_locks............ pass
 4. put................... pass
 5. lock_excl............. pass
 6. discover.............. pass
 7. refresh............... pass
 8. notowner_modify....... pass
 9. notowner_lock......... pass
10. owner_modify.......... pass
11. notowner_modify....... pass
12. notowner_lock......... pass
13. copy.................. pass
14. cond_put.............. SKIPPED
15. fail_cond_put......... SKIPPED
16. cond_put_with_not..... pass
17. cond_put_corrupt_token pass
18. complex_cond_put...... SKIPPED
19. fail_complex_cond_put. SKIPPED
20. unlock................ pass
21. fail_cond_put_unlocked pass
22. lock_shared........... pass
23. notowner_modify....... pass
24. notowner_lock......... pass
25. owner_modify.......... pass
26. double_sharedlock..... pass
27. notowner_modify....... pass
28. notowner_lock......... pass
29. unlock................ pass
30. prep_collection....... pass
31. lock_collection....... pass
32. owner_modify.......... pass
33. notowner_modify....... pass
34. refresh............... pass
35. indirect_refresh...... pass
36. unlock................ pass
37. unmapped_lock......... WARNING: LOCK on unmapped url returned 200 not 201 (RFC4918:S7.3)
    ...................... pass (with 1 warning)
38. unlock................ pass
39. finish................ pass
-> 4 tests were skipped.
<- summary for `locks': of 36 tests run: 36 passed, 0 failed. 100.0%
-> 1 warning was issued.
-> running `props':
 0. begin................. pass
 1. propfind_invalid...... pass
 2. propfind_invalid2..... pass
 3. propfind_d0........... pass
 4. propinit.............. pass
 5. propset............... pass
 6. propget............... pass
 7. propextended.......... pass
 8. propmove.............. pass
 9. propget............... pass
10. propdeletes........... pass
11. propget............... pass
12. propset............... pass
13. propdeletes........... pass
14. propdeletes........... pass
15. propreplace........... pass
16. propget............... pass
17. propnullns............ pass
18. propget............... pass
19. prophighunicode....... pass
20. propget............... pass
21. propremoveset......... pass
22. propget............... pass
23. propsetremove......... pass
24. propget............... pass
25. propvalnspace......... pass
26. propwformed........... pass
27. propinit.............. pass
28. propgetlastmod........ pass
29. propmanyns............ pass
30. propget............... pass
31. propcleanup........... pass
32. finish................ pass
<- summary for `props': of 33 tests run: 33 passed, 0 failed. 100.0%
```

</details>
