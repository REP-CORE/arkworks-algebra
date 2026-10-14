# msm-pool-exhaustion

Native repro for the `msm_bigint_wnaf` per-chunk pool fallback (branch `rep/ios-msm-pool-fallback`).

- **What upstream does.** `msm_bigint_wnaf` (`ec/src/scalar_mul/variable_base/mod.rs`) builds a rayon
  pool per chunk with `.build().unwrap()`. When `pthread_create` fails, the MSM panics.
- **Where it bit us.** On iOS, a Jolt proof's per-chunk pool churn alone takes the process to 1,024
  threads, which we read as iOS's per-process limit. Measured on an iPhone 15 Pro Max, iOS 26.6, 2026-10-14:
  - the first failed build was EAGAIN, at 1,024 process threads;
  - 408-566 failed builds per proof (the counter is cumulative per process; these are per-proof deltas);
  - every proof: 26 of 26 over three sittings.
- **The fix.** It runs the chunk on the current pool instead. Same result, no visible cost.

Run (macOS; on Linux, user-wide thread limits can trip before the process one):

    cargo test --release                   # exhaust the thread budget, MSMs stay correct, counter moves
    cargo run --release -- control         # no exhaustion
    cargo run --release -- exhaust 0       # zero free thread slots
    cargo run --release -- loop 4 256      # 300 back-to-back MSMs (n=256) with 4 free slots
    cargo run --release -- exhaust 0 4096 4 serial   # msm_serial reaches the pool too

The upstream follow-up (not in this branch): one shared pool, or the caller's pool, so no pool is
built per chunk, and honour `serial`.
