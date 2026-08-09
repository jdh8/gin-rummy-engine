# Strong-opponent evaluation

This fixed panel compares `greedy`, `mc:64`, and `mc:128` with two benchmark-only native adaptations: `gold-paper` and `marjj-v5-surrogate`. These are controlled host-engine comparisons—not executions of the original agents and not reproductions of their published tournaments.

Each matchup used seeds 7 and 8, 4000 mirrored round pairs per seed and 3000 mirrored game pairs per seed, with no optional stopping, extension, or seed replacement. Games use the corrected EAAI protocol: after a scored hand the dealer flips; after a dead hand the same dealer redeals. Rules are target 100, knock limit 10, gin/undercut bonuses 25, undercut on ties, no Big Gin, and no boxes, game bonus, or shutout bonus. Scores below are the raw totals that reached target.

An edge is declared only when both seed estimates point in the same nonzero direction and the pooled exact pair-sweep sign-test p-value remains below .05 after Holm correction across all six matchups. Everything else is **inconclusive**, never “equal.”

All three candidates beat `gold-paper` over games (62.0%–67.1% candidate win share) and lost to `marjj-v5-surrogate` (29.2%–34.2%); all six Holm-adjusted p-values were below .001 and both seeds agreed in direction. `mc:128` had the highest observed share against both opponents, but the predeclared tests compare each candidate with its opponent—not candidates with one another.

All three candidates beat `gold-paper` over games (62.0%–67.1% candidate win share) and lost to `marjj-v5-surrogate` (29.2%–34.2%); all six Holm-adjusted p-values were below .001 and both seeds agreed in direction. `mc:128` had the highest observed share against both opponents, but the predeclared tests compare each candidate with its opponent—not candidates with one another.

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
| `mc:64` vs `gold-paper` | 7 | 62.0% (60.9%–63.0%) | +17.97 | 964–246 | <0.001 | — | diagnostic |
| `mc:64` vs `gold-paper` | 8 | 62.0% (60.9%–63.0%) | +17.29 | 957–240 | <0.001 | — | diagnostic |
| `mc:64` vs `gold-paper` | pooled | 62.0% (61.2%–62.7%) | +17.63 | 1921–486 | <0.001 | <0.001 | **candidate edge** |
| `mc:64` vs `marjj-v5-surrogate` | 7 | 29.2% (28.2%–30.3%) | -33.89 | 160–1405 | <0.001 | — | diagnostic |
| `mc:64` vs `marjj-v5-surrogate` | 8 | 31.1% (30.1%–32.2%) | -31.50 | 180–1311 | <0.001 | — | diagnostic |
| `mc:64` vs `marjj-v5-surrogate` | pooled | 30.2% (29.4%–31.0%) | -32.69 | 340–2716 | <0.001 | <0.001 | **opponent edge** |
| `mc:128` vs `gold-paper` | 7 | 66.7% (65.6%–67.8%) | +25.25 | 1203–201 | <0.001 | — | diagnostic |
| `mc:128` vs `gold-paper` | 8 | 67.5% (66.4%–68.5%) | +25.26 | 1220–173 | <0.001 | — | diagnostic |
| `mc:128` vs `gold-paper` | pooled | 67.1% (66.3%–67.8%) | +25.26 | 2423–374 | <0.001 | <0.001 | **candidate edge** |
| `mc:128` vs `marjj-v5-surrogate` | 7 | 33.7% (32.6%–34.8%) | -26.87 | 224–1200 | <0.001 | — | diagnostic |
| `mc:128` vs `marjj-v5-surrogate` | 8 | 34.6% (33.5%–35.7%) | -25.24 | 254–1177 | <0.001 | — | diagnostic |
| `mc:128` vs `marjj-v5-surrogate` | pooled | 34.2% (33.4%–35.0%) | -26.06 | 478–2377 | <0.001 | <0.001 | **opponent edge** |

## Single-round diagnostics

These pooled single-round results diagnose tactics; they are not the game-strength declaration. Points and paired differential are per individual round. Finish counts are attributed to the bot that won that outcome.

| Matchup | Decisive win share (pair-cluster 95% CI) | Points/round | Paired point differential | Dead rate | Candidate K/U/G | Opponent K/U/G |
|---|---:|---:|---:|---:|---:|---:|
| `greedy` vs `gold-paper` | 40.2% (39.6%–40.8%) | 9.28 vs 8.22 | +1.06 | 0.1% | 3284/2658/485 | 9282/18/257 |
| `greedy` vs `marjj-v5-surrogate` | 53.8% (53.1%–54.6%) | 7.87 vs 14.43 | -6.56 | 0.6% | 6678/32/1850 | 1310/2627/3402 |
| `mc:64` vs `gold-paper` | 53.8% (53.2%–54.4%) | 10.15 vs 8.10 | +2.05 | 0.1% | 7720/519/364 | 6976/246/163 |
| `mc:64` vs `marjj-v5-surrogate` | 57.4% (56.7%–58.1%) | 9.15 vs 13.47 | -4.32 | 0.6% | 8358/16/760 | 1361/4007/1400 |
| `mc:128` vs `gold-paper` | 55.1% (54.4%–55.8%) | 10.90 vs 7.93 | +2.96 | 0.1% | 7730/644/437 | 6781/258/142 |
| `mc:128` vs `marjj-v5-surrogate` | 57.7% (57.0%–58.4%) | 9.72 vs 13.45 | -3.73 | 0.9% | 8131/16/1002 | 1393/3758/1556 |

Decisive-round win share alone is misleading here: `greedy` won only 40.2% of decisive rounds against Gold yet led by 1.06 points/round and won 62.2% of games. Conversely, candidates won 53.8%–57.7% of decisive rounds against MARJJ yet trailed by 3.73–6.56 points/round and won only 29.2%–34.2% of games.

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

- Measured arena source SHA-256: `5f44b626c229306d4b0879351258a08e39f0259eea604cbd1202e84bd2f4f9ac`
- Cargo.lock SHA-256: `a0037359bca064781c6673725500d84f538f17c71ccd73478c8f84321f6d06d6`
- Git commit: `99bad5f4bfe14622aee3b2b07d7f9a7fb42fc489`; dirty worktree: `false`
- Compiler: `rustc 1.97.1 (8bab26f4f 2026-07-14) (Fedora 1.97.1-1.fc44); binary: rustc; commit-hash: 8bab26f4f68e0e26f0bb7960be334d5b520ea452; commit-date: 2026-07-14; host: x86_64-unknown-linux-gnu; release: 1.97.1; LLVM version: 22.1.8`
- Platform: `Linux 7.1.6-201.fc44.x86_64 #1 SMP PREEMPT_DYNAMIC Tue Aug  4 00:23:39 UTC 2026 x86_64 GNU/Linux` / `x86_64`
- CPU: `AMD Ryzen 7 8700F 8-Core Processor`; logical threads: `16`
- Sum of measured leg runtimes: 17334.3 seconds
- Upstream conformance: `passed`; receipt: `contrib/strong-conformance/receipt.json`
- Raw machine-readable evidence: [strong-opponents.json](strong-opponents.json)
- Report encoding: exact-sign numeric and scientific-decimal fields were recomputed after measurement solely from the validated sweep counts; no rounds or games were rerun. The raw JSON records the report-helper hashes.

Runtime caveat: an accidentally retained duplicate benchmark shared the CPU during part of this panel. Contention inflated elapsed times and depressed throughput, so the 17334.3-second leg sum and raw throughput fields are provenance only, not clean speed measurements. Seeded outcomes and statistical estimates remain valid because decisions do not depend on wall time and no player timer was enforced.
