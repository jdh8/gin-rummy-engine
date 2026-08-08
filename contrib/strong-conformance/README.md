# Optional strong-opponent conformance checks

This directory contains an opt-in, network-free check of the benchmark-only
Gold and MARJJ adaptations.  It is deliberately outside the normal test
suite: neither upstream project publishes a dependency lock file, and the
MARJJ artifact also needs the EAAI Java framework to compile.

The checker never clones or downloads anything.  Supply local checkouts:

```sh
scripts/check-strong-conformance.sh \
  --gold-root /path/to/adversarial-coevolution \
  --marjj-root /path/to/MARJJ \
  --eaai-root /path/to/gin-rummy-eaai
```

Either opponent may be checked by itself.  Gold additionally requires Python
3.11 with `PettingZoo==1.24.3`, `rlcard==1.0.5`, and NumPy installed.  MARJJ
requires `javac` and `java`.  Rust dependencies must already be cached; the
checker invokes Cargo with both `--locked` and `--offline`.

The script rejects source files whose SHA-256 digests differ from the pinned
artifacts, including the small Python package files needed to import Gold.
When a supplied directory is a Git checkout it also requires the exact
commit; an unpacked source archive is accepted only after all required file
hashes match.  The pins are:

| Artifact | Commit | SHA-256 |
| --- | --- | --- |
| `agents/gold_standard_agent.py` | `3b2f5b7866d27234647c5833497c12ca1a2afde9` | `88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca` |
| `MARJJ_v5-1.java` | `5d1f00c1dff5380021785c8146d039a11efcabc3` | `df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f` |
| EAAI framework | `559c712516e3b0fd6b908864acd141e254d94f39` | Per-file hashes in the checker |

## What is compared

The Gold probe imports the pinned agent and checks generated, uniquely
determined draw, ordinary-discard, and non-gin-knock decisions against the
native adapter.  Gin is checked only as a category because RLCard exposes a
global gin action while this engine requires a discard-and-knock action.

Both probes stage only their hash-verified source files in a temporary
directory.  The MARJJ staging step adds only the missing `package ginrummy;`
declaration and corrects the filename to match `public class MARJJ_v5`.  Its
line-oriented trace driver exposes opening offers, first-turn choices,
complete minimum-deadwood meld sets, candidate sets, component scores (as
exact raw IEEE-754 hexadecimal values), tied minima, knock/null timing, and
knock spreads.  The Rust ignored test compares the decisions and diagnostics
that can be represented through both public strategy interfaces.
The temporary tree is removed on exit; no upstream source is copied into this
repository.

The following are classified and reported, not treated as parity failures:

- Java collection/insertion ordering where the public artifact does not
  specify a stable order;
- random choice among exactly tied MARJJ candidates (the native adapter uses
  the arena RNG, not Java's `Random` stream);
- final-bit floating-point presentation differences when the minimizing set
  is unchanged;
- opening-offer, Big-Gin, defender-meld, layoff, and stock-dead behavior that
  one of the two hosts cannot represent;
- Gold gin's action id versus the native discard-to-gin action.

Passing this check establishes decision parity only for the emitted,
representable cases.  It does not establish whole-game identity, reproduce
the historical EAAI tournament, or prove that public `MARJJ_v5` is the
submitted `MARJJ_Player` binary.

The checked [receipt](receipt.json) records the pinned-source run used for
the published strong-opponent panel.  Re-run the workflow after changing an
adapter, then update the receipt only from a fully passing run.
