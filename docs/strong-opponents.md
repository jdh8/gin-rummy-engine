# Strong-opponent evaluation

This fixed panel compares `greedy`, `mc:64`, and `mc:128` with two benchmark-only native adaptations: `gold-paper` and `marjj-v5-surrogate`. These are controlled host-engine comparisons—not executions of the original agents and not reproductions of their published tournaments.

Each matchup used seeds 7 and 8, 4000 mirrored round pairs per seed and 3000 mirrored game pairs per seed, with no optional stopping, extension, or seed replacement. Games use the corrected EAAI protocol: after a scored hand the dealer flips; after a dead hand the same dealer redeals. Rules are target 100, knock limit 10, gin/undercut bonuses 25, undercut on ties, no Big Gin, and no boxes, game bonus, or shutout bonus. Scores below are the raw totals that reached target.

An edge is declared only when both seed estimates point in the same nonzero direction and the pooled exact pair-sweep sign-test p-value remains below .05 after Holm correction across all six matchups. Everything else is **inconclusive**, never “equal.”

## Game results

Game win share and its 95% interval use mirrored pairs as clusters. The score margin is candidate minus opponent raw target-reaching score per game. The exact sign test counts only 2–0 pair sweeps; 1–1 splits are ties. Holm adjustment applies to the six pooled rows.

| Matchup | Seed | Win share (pair-cluster 95% CI) | Raw score margin/game | Sweeps | Raw p | Holm p | Finding |
|---|---:|---:|---:|---:|---:|---:|---|
| `greedy` vs `gold-paper` | 7 | 62.2% (61.2%–63.2%) | +15.38 | 952–219 | <0.001 | — | diagnostic |
| `greedy` vs `gold-paper` | 8 | 62.1% (61.1%–63.1%) | +15.47 | 940–215 | <0.001 | — | diagnostic |
| `greedy` vs `gold-paper` | pooled | 62.2% (61.4%–62.9%) | +15.43 | 1892–434 | <0.001 | <0.001 | **candidate edge** |
| `greedy` vs `marjj-v5-surrogate` | 7 | 28.8% (27.7%–29.9%) | -36.02 | 198–1472 | <0.001 | — | diagnostic |
| `greedy` vs `marjj-v5-surrogate` | 8 | 29.7% (28.6%–30.8%) | -34.34 | 215–1435 | <0.001 | — | diagnostic |
| `greedy` vs `marjj-v5-surrogate` | pooled | 29.2% (28.4%–30.0%) | -35.18 | 413–2907 | <0.001 | <0.001 | **opponent edge** |
| `mc:64` vs `gold-paper` | 7 | 70.0% (68.9%–71.0%) | +29.82 | 1342–144 | <0.001 | — | diagnostic |
| `mc:64` vs `gold-paper` | 8 | 69.1% (68.1%–70.1%) | +28.51 | 1285–138 | <0.001 | — | diagnostic |
| `mc:64` vs `gold-paper` | pooled | 69.5% (68.8%–70.3%) | +29.17 | 2627–282 | <0.001 | <0.001 | **candidate edge** |
| `mc:64` vs `marjj-v5-surrogate` | 7 | 42.2% (41.0%–43.4%) | -13.79 | 425–893 | <0.001 | — | diagnostic |
| `mc:64` vs `marjj-v5-surrogate` | 8 | 42.5% (41.4%–43.7%) | -13.74 | 455–902 | <0.001 | — | diagnostic |
| `mc:64` vs `marjj-v5-surrogate` | pooled | 42.4% (41.6%–43.2%) | -13.77 | 880–1795 | <0.001 | <0.001 | **opponent edge** |
| `mc:128` vs `gold-paper` | 7 | 75.1% (74.1%–76.1%) | +37.57 | 1593–85 | <0.001 | — | diagnostic |
| `mc:128` vs `gold-paper` | 8 | 73.9% (72.9%–74.9%) | +35.93 | 1521–86 | <0.001 | — | diagnostic |
| `mc:128` vs `gold-paper` | pooled | 74.5% (73.8%–75.2%) | +36.75 | 3114–171 | <0.001 | <0.001 | **candidate edge** |
| `mc:128` vs `marjj-v5-surrogate` | 7 | 46.7% (45.6%–47.9%) | -7.14 | 558–754 | <0.001 | — | diagnostic |
| `mc:128` vs `marjj-v5-surrogate` | 8 | 46.7% (45.5%–47.8%) | -7.66 | 563–764 | <0.001 | — | diagnostic |
| `mc:128` vs `marjj-v5-surrogate` | pooled | 46.7% (45.9%–47.5%) | -7.40 | 1121–1518 | <0.001 | <0.001 | **opponent edge** |

## Single-round diagnostics

These pooled single-round results diagnose tactics; they are not the game-strength declaration. Points and paired differential are per individual round. Finish counts are attributed to the bot that won that outcome.

| Matchup | Decisive win share (pair-cluster 95% CI) | Points/round | Paired point differential | Dead rate | Candidate K/U/G | Opponent K/U/G |
|---|---:|---:|---:|---:|---:|---:|
| `greedy` vs `gold-paper` | 40.2% (39.6%–40.8%) | 9.28 vs 8.22 | +1.06 | 0.1% | 3284/2658/485 | 9282/18/257 |
| `greedy` vs `marjj-v5-surrogate` | 53.8% (53.1%–54.6%) | 7.87 vs 14.43 | -6.56 | 0.6% | 6678/32/1850 | 1310/2627/3402 |
| `mc:64` vs `gold-paper` | 48.6% (47.9%–49.2%) | 12.13 vs 8.16 | +3.97 | 0.1% | 4303/2443/1015 | 7877/140/204 |
| `mc:64` vs `marjj-v5-surrogate` | 53.3% (52.6%–54.0%) | 12.60 vs 14.41 | -1.81 | 3.3% | 4219/13/4014 | 1380/1927/3913 |
| `mc:128` vs `gold-paper` | 49.5% (48.9%–50.2%) | 13.00 vs 8.01 | +4.98 | 0.1% | 3915/2834/1165 | 7723/130/212 |
| `mc:128` vs `marjj-v5-surrogate` | 53.7% (53.0%–54.4%) | 13.48 vs 14.24 | -0.76 | 4.0% | 3670/17/4559 | 1394/1466/4252 |

## Provenance and adaptations

`gold-paper` follows the fixed heuristic in the [2026 paper](https://arxiv.org/html/2607.06854v1) and [pinned source](https://github.com/Nikelroid/adversarial-coevolution/blob/3b2f5b7866d27234647c5833497c12ca1a2afde9/agents/gold_standard_agent.py) (`88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca`). Its published 70–99% results came from a different simplified single-hand RLCard/PettingZoo reward environment. Despite the repository label, the paper does not claim game-theoretic optimality for full gin rummy; only meld decomposition is exact. Host-only opening, gin, Big Gin, meld-selection, defender, and layoff behavior is adapted as recorded in the raw JSON.

`marjj-v5-surrogate` independently implements the reachable path of the [later public MARJJ_v5 file](https://github.com/aqibahm/MARJJ/blob/5d1f00c1dff5380021785c8146d039a11efcabc3/MARJJ_v5-1.java) (`df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f`). The [official results](https://cs.gettysburg.edu/~tneller/games/ginrummy/eaai/gin-rummy-results.pdf) identify the separately named `MARJJ_Player` as the 2021 winner; they do not establish that this later public file is the submitted binary. The source uses initial future weight 18, discount 0.9, and best-seven selection, while the paper reports 20/0.9/six. Canonical ordering, seeded ties, optimized-defender settlement, and greedy layoffs are host adaptations.

Both adapters use host settlement and layoff semantics, which can differ from upstream environments. No EAAI 30-second player timer is enforced. They remain outside the library API and interactive player list. No upstream agent or GPL EAAI framework source is copied or vendored; both agent repositories lack explicit licenses, so benchmark-only placement does not eliminate distribution risk.

Because the host `View` has no round identifier, the MARJJ adaptation infers round reset from callbacks. If its opening callback is skipped and the same seat receives the exact same ten-card hand in consecutive rounds, stale surrogate history could theoretically survive. This pathological limitation is retained in the measured, conformed adapter.

Mirroring is common-random-number seat reversal, not a guarantee of identical later game histories. Orientation-dependent outcomes—including whether a hand is dead—can make later dealer sequences diverge under dead-hand retention.

Commands (release mode, without the `parallel` feature):

```console
scripts/bench-strong.sh --smoke
STRONG_CONFORMANCE_RECEIPT=contrib/strong-conformance/receipt.json \
  scripts/bench-strong.sh
```

## Reproducibility

- Measured arena source SHA-256: `fa84470015c06d6e3163d86e5a44cc3bec830eecaea4d4640a8d1b996b00fd72`
- Cargo.lock SHA-256: `a0037359bca064781c6673725500d84f538f17c71ccd73478c8f84321f6d06d6`
- Git commit: `099c38e859009f72503e25b25bf685b538630e76`; dirty worktree: `false`
- Compiler: `rustc 1.97.1 (8bab26f4f 2026-07-14) (Fedora 1.97.1-1.fc44); binary: rustc; commit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452; commit-date: 2026-07-14; host: x86_64-unknown-linux-gnu; release: 1.97.1; LLVM version: 22.1.8`
- Platform: `Linux 7.1.6-201.fc44.x86_64 #1 SMP PREEMPT_DYNAMIC Tue Aug  4 00:23:39 UTC 2026 x86_64 GNU/Linux` / `x86_64`
- CPU: `AMD Ryzen 7 8700F 8-Core Processor`; logical threads: `16`
- Sum of measured leg runtimes: 8285.7 seconds
- Upstream conformance: `passed`; receipt: `contrib/strong-conformance/receipt.json`
- Raw machine-readable evidence: [strong-opponents.json](strong-opponents.json)
- Report encoding: exact-sign numeric and scientific-decimal fields were recomputed after measurement solely from the validated sweep counts; no rounds or games were rerun. The raw JSON records the report-helper hashes.
