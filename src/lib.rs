#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub use gin_rummy;

mod action;
mod driver;
mod heuristic;
#[cfg(feature = "rand")]
mod mc;
#[cfg(feature = "rand")]
mod sim;
mod strategy;
mod view;

pub use action::{DrawAction, Layoff, TurnAction, UpcardAction};
#[cfg(feature = "rand")]
pub use driver::play_game;
pub use driver::{EngineError, Table, play_round};
pub use heuristic::{HeuristicBot, HeuristicConfig};
#[cfg(feature = "rand")]
pub use mc::MonteCarloBot;
pub use strategy::Strategy;
pub use view::View;
