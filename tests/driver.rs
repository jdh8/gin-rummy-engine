//! Driving rounds and games end to end

use gin_rummy::{Game, Hand, Phase, Player, Round, Rules};
use gin_rummy_engine::{
    DealerRotation, DrawAction, EngineError, HeuristicBot, Layoff, Strategy, Table, TurnAction,
    UpcardAction, View, play_round,
};

/// A deterministic deal: the sorted deck dealt round-robin
fn fixed_deal(rules: Rules, dealer: Player) -> Round {
    let deck: Vec<_> = Hand::ALL.iter().collect();
    let hands = [
        deck.iter().step_by(2).take(10).copied().collect::<Hand>(),
        deck.iter().skip(1).step_by(2).take(10).copied().collect(),
    ];
    let upcard = deck[20];
    let stock = deck[21..].to_vec();
    Round::from_deal(rules, dealer, hands, upcard, stock).expect("a partitioned deck")
}

#[test]
fn heuristic_round_finishes_and_records() {
    for dealer in Player::ALL {
        let round = fixed_deal(Rules::default(), dealer);
        let result = play_round(round, [&mut HeuristicBot::new(), &mut HeuristicBot::new()])
            .expect("heuristic bots always choose legal actions");

        let mut game = Game::new(Rules::default(), dealer);
        game.record(result).expect("a round result records");
        if let Some(winner) = result.winner() {
            assert!(game.score(winner) > 0);
        }
    }
}

#[test]
fn deterministic_bots_replay_identically() {
    let play = || {
        play_round(
            fixed_deal(Rules::default(), Player::One),
            [&mut HeuristicBot::new(), &mut HeuristicBot::new()],
        )
        .expect("legal play")
    };
    assert_eq!(play(), play());
}

/// Takes the upcard, then tries to shed it the same turn — illegal
struct Cheater;

impl Strategy for Cheater {
    fn offer_upcard(&mut self, _: &View<'_>) -> UpcardAction {
        UpcardAction::Take
    }
    fn choose_draw(&mut self, _: &View<'_>) -> DrawAction {
        DrawAction::TakeDiscard
    }
    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        TurnAction::Discard(view.taken_discard().expect("this bot always takes"))
    }
    fn choose_layoff(&mut self, _: &View<'_>) -> Option<Layoff> {
        None
    }
}

#[test]
fn illegal_action_reports_the_offending_seat() {
    // Player Two is the non-dealer, so the cheater acts first.
    let round = fixed_deal(Rules::default(), Player::One);
    let error = play_round(round, [&mut HeuristicBot::new(), &mut Cheater])
        .expect_err("shedding the just-taken discard is illegal");
    let EngineError::IllegalAction { seat, .. } = error else {
        panic!("unexpected error kind");
    };
    assert_eq!(seat, Player::Two);
}

#[test]
fn table_retries_after_an_illegal_action() {
    let mut table = Table::new(fixed_deal(Rules::default(), Player::One));
    let mut cheater = Cheater;
    let mut honest = HeuristicBot::new();

    // Upcard take, then the illegal shed: the table must stay usable.
    assert_eq!(table.turn(), Some(Player::Two));
    table
        .step(&mut cheater)
        .expect("taking the upcard is legal");
    assert_eq!(table.round().phase(), Phase::Discard);
    assert!(table.step(&mut cheater).is_err());
    assert_eq!(table.round().phase(), Phase::Discard);
    assert_eq!(table.turn(), Some(Player::Two));

    // An honest strategy completes the same decision.
    table.step(&mut honest).expect("an honest turn is accepted");
    assert_eq!(table.turn(), Some(Player::One));
}

/// Wraps a strategy, logging `'B'` for each round announcement and `'D'`
/// for each decision
#[derive(Default)]
struct Recorder<S> {
    inner: S,
    log: String,
}

impl<S: Strategy> Strategy for Recorder<S> {
    fn begin_round(&mut self, view: &View<'_>) {
        assert_eq!(view.stock_len(), 31, "announced before any stock draw");
        self.log.push('B');
    }
    fn offer_upcard(&mut self, view: &View<'_>) -> UpcardAction {
        self.log.push('D');
        self.inner.offer_upcard(view)
    }
    fn choose_draw(&mut self, view: &View<'_>) -> DrawAction {
        self.log.push('D');
        self.inner.choose_draw(view)
    }
    fn play_turn(&mut self, view: &View<'_>) -> TurnAction {
        self.log.push('D');
        self.inner.play_turn(view)
    }
    fn choose_layoff(&mut self, view: &View<'_>) -> Option<Layoff> {
        self.log.push('D');
        self.inner.choose_layoff(view)
    }
}

#[test]
fn play_announces_each_seat_once_before_deciding() {
    for dealer in Player::ALL {
        for rotation in [
            DealerRotation::WinnerDeals,
            DealerRotation::AlternateAfterScoredRound,
        ] {
            let mut one = Recorder::<HeuristicBot>::default();
            let mut two = Recorder::<HeuristicBot>::default();
            Table::new(fixed_deal(Rules::default(), dealer))
                .dealer_rotation(rotation)
                .play([&mut one, &mut two])
                .expect("heuristic bots always choose legal actions");
            for log in [one.log, two.log] {
                assert!(log.starts_with("BD"), "announced first: {log}");
                assert_eq!(log.matches('B').count(), 1, "announced once: {log}");
            }
        }
    }
}

#[test]
fn step_announces_lazily_and_not_on_retry() {
    // Player Two, the non-dealer, takes the upcard, so the dealer's first
    // decision is a draw: no upcard offer marks the round for it.
    let mut table = Table::new(fixed_deal(Rules::default(), Player::One));
    let mut cheater = Recorder {
        inner: Cheater,
        log: String::new(),
    };
    table
        .step(&mut cheater)
        .expect("taking the upcard is legal");
    assert!(table.step(&mut cheater).is_err());
    assert_eq!(
        cheater.log, "BDD",
        "a rejected decision is not re-announced"
    );
    table
        .step(&mut HeuristicBot::new())
        .expect("an honest turn is accepted");

    let mut dealer = Recorder::<HeuristicBot>::default();
    table.step(&mut dealer).expect("a draw is legal");
    assert_eq!(dealer.log, "BD");
}

#[cfg(feature = "rand")]
mod dealt {
    use super::*;
    use gin_rummy::RoundResult;
    use gin_rummy_engine::{MonteCarloBot, play_game};
    use rand::SeedableRng as _;
    use rand::rngs::StdRng;

    #[test]
    fn seeded_games_settle() {
        for seed in 0..5 {
            let mut rng = StdRng::seed_from_u64(seed);
            let mut game = Game::new(Rules::default(), Player::One);
            let score = play_game(
                &mut game,
                [&mut HeuristicBot::new(), &mut HeuristicBot::new()],
                &mut rng,
            )
            .expect("heuristic bots always choose legal actions");
            assert!(game.is_over());
            assert!(score.totals[score.winner as usize] > 0);
        }
    }

    #[test]
    fn play_game_announces_every_round() {
        let mut rng = StdRng::seed_from_u64(3);
        let mut game = Game::new(Rules::default(), Player::One);
        let mut one = Recorder::<HeuristicBot>::default();
        let mut two = Recorder::<HeuristicBot>::default();
        play_game(&mut game, [&mut one, &mut two], &mut rng)
            .expect("heuristic bots always choose legal actions");
        let rounds = one.log.matches('B').count();
        assert!(rounds > 1, "a game spans several rounds");
        assert_eq!(two.log.matches('B').count(), rounds);
        for log in [one.log, two.log] {
            assert!(log.starts_with('B') && !log.contains("BB"), "{log}");
        }
    }

    #[test]
    fn monte_carlo_plays_legally() {
        let mut rng = StdRng::seed_from_u64(9);
        let mut mc = MonteCarloBot::new(StdRng::seed_from_u64(10)).samples(4);
        for _ in 0..3 {
            let mut table = Table::deal(Rules::default(), Player::One, &mut rng);
            let result = table
                .play([&mut HeuristicBot::new(), &mut mc])
                .expect("the Monte Carlo bot chooses legal actions");
            assert!(matches!(
                result,
                RoundResult::Dead
                    | RoundResult::Knock { .. }
                    | RoundResult::Undercut { .. }
                    | RoundResult::Gin { .. }
                    | RoundResult::BigGin { .. }
            ));
        }
    }
}
