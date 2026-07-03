# gin-rummy-engine

This crate implements gameplay strategy for gin rummy on top of the
[gin-rummy](../gin-rummy) mechanics crate: a `Strategy` trait, an
information-hygienic `View`, a `Table` driver, a deterministic
`HeuristicBot`, and a determinized `MonteCarloBot` (feature `rand`).
Game mechanics — card types, the deadwood solver, the `Round` state machine,
scoring — live in gin-rummy and are out of scope here; only decision-making
belongs in this crate.

The central invariant is information hygiene: `Round` exposes both hands and
the stock order, but strategies only ever receive a `View`, whose accessors
are the whitelist of legally visible information.  The driver maintains the
per-seat `Knowledge` (opponent's taken/shed/passed cards, the just-taken
discard, the forced stock draw) that a `Round` snapshot cannot recover.
Until a knock reveals the spread, `View::unseen` must satisfy
`unseen.len() == stock_len + opponent_hand_len − opponent_known.len()` —
exactly the cards a determinizing bot distributes between the stock and the
hidden part of the opponent's hand.

The Monte Carlo rollout uses a crate-private forward model (`sim.rs`) because
a `Round` cannot be constructed mid-game.  Any rules change upstream must be
mirrored there; the `Sim` ⇔ `Round` equivalence proptest guards the pairing.

After updating the codebase, please

- Format the code with `cargo fmt`.
- Run the tests with `cargo test --all-features`.
- Update [CHANGELOG.md](CHANGELOG.md) with a summary of the changes and their impact on users.
- Propose a clear and descriptive commit message.
