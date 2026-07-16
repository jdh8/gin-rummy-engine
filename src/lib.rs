#![doc = include_str!("../README.md")]
#![warn(missing_docs)]

pub use gin_rummy;

mod action;
mod driver;
#[cfg(feature = "rand")]
mod eaai;
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
#[cfg(feature = "rand")]
pub use eaai::EaaiSimpleBot;
pub use heuristic::{HeuristicBot, HeuristicConfig};
#[cfg(feature = "rand")]
pub use mc::{Assessment, MonteCarloBot};
pub use strategy::Strategy;
pub use view::View;
