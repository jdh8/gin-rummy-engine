//! Benchmark-only adaptations of externally described opponents.

mod gold;
mod marjj;
mod melds;

use anyhow::Result;
use gin_rummy::{Hand, Meld, best_melds};
use gin_rummy_engine::{Layoff, Strategy};
use rand::SeedableRng as _;
use rand::rngs::StdRng;

pub use gold::GoldPaperBot;
pub use marjj::MarjjV5Surrogate;

/// Bot specifications recognized by [`make_bot`].
pub const BOT_SPECS: &[&str] = &["gold-paper", "marjj-v5-surrogate"];

/// Construct a benchmark-only opponent, leaving ordinary arena bot names to
/// the caller.
pub fn make_bot(spec: &str, seed: u64) -> Result<Option<Box<dyn Strategy>>> {
    match spec {
        "gold-paper" => Ok(Some(Box::new(GoldPaperBot::new()))),
        "marjj-v5-surrogate" => Ok(Some(Box::new(MarjjV5Surrogate::new(
            StdRng::seed_from_u64(seed),
        )))),
        _ => Ok(None),
    }
}

/// The engine's greedy layoff policy, duplicated here because benchmark
/// support deliberately sits outside the library's public API.
pub(crate) fn greedy_layoff(hand: Hand, spread: impl Iterator<Item = Meld>) -> Option<Layoff> {
    let dead = best_melds(hand).deadwood_cards();
    spread
        .enumerate()
        .flat_map(|(meld, spread)| {
            dead.iter()
                .filter(move |&card| spread.extended(card).is_some())
                .map(move |card| Layoff { card, meld })
        })
        .max_by_key(|layoff| layoff.card.rank.deadwood())
}
