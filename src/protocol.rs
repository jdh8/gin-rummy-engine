//! Game-level protocol choices shared by tables and score-aware strategies.

use gin_rummy::{Rules, Shutout};

/// How the dealer for the next round is selected.
///
/// A dead round keeps the current dealer under both supported protocols.
/// The distinction is what happens after a scored round: ordinary gin
/// rummy gives the deal to its winner, while the EAAI challenge alternated
/// the starting player (and therefore the dealer role) after each scored
/// round.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DealerRotation {
    /// The winner of a scored round deals the next round.
    #[default]
    WinnerDeals,
    /// A scored round flips the dealer; a dead round is redealt unchanged.
    AlternateAfterScoredRound,
}

/// The rules used by the 2021 EAAI Gin Rummy Challenge.
///
/// The challenge played to 100 with 25-point gin and undercut bonuses,
/// undercut on equal deadwood, and no Big Gin, boxes, game bonus, or
/// shutout bonus.
#[must_use]
pub const fn eaai_rules() -> Rules {
    let mut rules = Rules::new();
    rules.big_gin_bonus = None;
    rules.box_bonus = 0;
    rules.game_bonus = 0;
    rules.shutout = Shutout::Flat(0);
    rules
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eaai_preset_has_only_round_scoring() {
        let rules = eaai_rules();
        assert_eq!(rules.game_target, 100);
        assert_eq!(rules.knock_limit, 10);
        assert_eq!(rules.gin_bonus, 25);
        assert_eq!(rules.undercut_bonus, 25);
        assert!(rules.undercut_on_tie);
        assert_eq!(rules.big_gin_bonus, None);
        assert_eq!(rules.box_bonus, 0);
        assert!(!rules.immediate_boxes);
        assert_eq!(rules.game_bonus, 0);
        assert_eq!(rules.shutout, Shutout::Flat(0));
    }
}
