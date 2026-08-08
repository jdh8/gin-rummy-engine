//! Opt-in checks against user-supplied, pinned upstream opponent sources.

#![cfg(feature = "rand")]

// The support tree keeps meld enumeration private; this second path import is
// intentional so conformance can compare complete partitions without making
// benchmark diagnostics public.
#[allow(clippy::duplicate_mod, dead_code)]
#[path = "../examples/support/strong/melds.rs"]
mod marjj_melds;
#[allow(dead_code)]
#[path = "../examples/support/strong/mod.rs"]
mod strong;

use gin_rummy::{Card, Hand, Melds, Rank, Suit, deadwood};
use gin_rummy_engine::TurnAction;
use rand::SeedableRng as _;
use rand::rngs::StdRng;
use rand::seq::SliceRandom as _;
use std::env;
use std::io::Write as _;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use strong::GoldPaperBot;

fn card(text: &str) -> Card {
    text.parse().expect("a valid fixture card")
}

fn hand(text: &str) -> Hand {
    text.parse().expect("a valid fixture hand")
}

fn card_hand(cards: &[&str]) -> Hand {
    cards
        .iter()
        .map(|text| card(text))
        .fold(Hand::EMPTY, |hand, card| hand | card.into())
}

fn gold_id(card: Card) -> u8 {
    let suit = match card.suit {
        Suit::Spades => 0,
        Suit::Hearts => 1,
        Suit::Diamonds => 2,
        Suit::Clubs => 3,
    };
    suit * 13 + card.rank.get() - 1
}

fn eaai_id(card: Card) -> u8 {
    let suit = match card.suit {
        Suit::Clubs => 0,
        Suit::Hearts => 1,
        Suit::Spades => 2,
        Suit::Diamonds => 3,
    };
    suit * 13 + card.rank.get() - 1
}

fn from_eaai_id(id: u8) -> Card {
    let suit = match id / 13 {
        0 => Suit::Clubs,
        1 => Suit::Hearts,
        2 => Suit::Spades,
        3 => Suit::Diamonds,
        _ => panic!("an EAAI card id is below 52"),
    };
    Card {
        suit,
        rank: Rank::new(id % 13 + 1),
    }
}

fn ids(cards: Hand, convert: fn(Card) -> u8) -> String {
    let mut ids: Vec<_> = cards.iter().map(convert).collect();
    ids.sort_unstable();
    ids.into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

fn run_lines(mut command: Command, input: &str) -> Vec<String> {
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start the upstream trace process");
    child
        .stdin
        .take()
        .expect("the trace process has piped stdin")
        .write_all(input.as_bytes())
        .expect("write trace requests");
    let output = child.wait_with_output().expect("wait for trace process");
    assert!(
        output.status.success(),
        "upstream trace failed:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("trace output is UTF-8")
        .lines()
        .map(str::to_owned)
        .collect()
}

fn java_double(bits: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(bits, 16).expect("raw Java double bits"))
}

fn assert_float_parity(source: f64, native: f64, component: &str) {
    if source.to_bits() == native.to_bits() {
        return;
    }
    let same_sign = source.is_sign_negative() == native.is_sign_negative();
    let ulps = source.to_bits().abs_diff(native.to_bits());
    assert!(
        same_sign && ulps <= 1,
        "MARJJ {component} differs materially: source={source:?}, native={native:?}, ulps={ulps}",
    );
    eprintln!(
        "classified MARJJ {component}: one-ULP host floating-point tail ({source:?} vs {native:?})"
    );
}

fn spread_trace(spread: Melds) -> String {
    let mut melds: Vec<_> = spread
        .iter()
        .map(|meld| marjj_melds::eaai_bits(meld.cards()))
        .collect();
    melds.sort_unstable();
    melds
        .into_iter()
        .map(|meld| format!("{meld:x}"))
        .collect::<Vec<_>>()
        .join(",")
}

fn all_meld_trace(hand: Hand) -> Vec<String> {
    let mut partitions: Vec<_> = marjj_melds::all_minimum_melds(hand)
        .into_iter()
        .map(spread_trace)
        .map(|spread| {
            if spread.is_empty() {
                "-".to_owned()
            } else {
                spread
            }
        })
        .collect();
    partitions.sort();
    partitions.dedup();
    partitions
}

fn gold_discard_request(cards: Hand, taken: Option<Card>) -> (String, u16, bool) {
    let mut legal = Vec::new();
    for card in cards.iter().filter(|&card| Some(card) != taken) {
        legal.push(6 + u16::from(gold_id(card)));
    }
    for card in cards.iter().filter(|&card| Some(card) != taken) {
        if deadwood(cards - card.into()) <= 10 {
            legal.push(58 + u16::from(gold_id(card)));
        }
    }
    legal.sort_unstable();

    let action = GoldPaperBot::turn_action(cards, taken, 10);
    let (expected, unique) = match action {
        TurnAction::Discard(discard) => {
            let chosen = deadwood(cards - discard.into());
            let pip = discard.rank.deadwood();
            let tied = cards
                .iter()
                .filter(|&candidate| Some(candidate) != taken)
                .filter(|&candidate| {
                    deadwood(cards - candidate.into()) == chosen && candidate.rank.deadwood() == pip
                })
                .count();
            (6 + u16::from(gold_id(discard)), tied == 1)
        }
        TurnAction::Knock { discard, melds } if melds.deadwood() != 0 => {
            let chosen = deadwood(cards - discard.into());
            let tied = cards
                .iter()
                .filter(|&candidate| Some(candidate) != taken)
                .filter(|&candidate| deadwood(cards - candidate.into()) == chosen)
                .count();
            (58 + u16::from(gold_id(discard)), tied == 1)
        }
        TurnAction::Knock { .. } | TurnAction::BigGin(_) => return (String::new(), 5, false),
    };
    let legal = legal
        .into_iter()
        .map(|id| id.to_string())
        .collect::<Vec<_>>()
        .join(",");
    (
        format!("discard|{}|-|{}\n", ids(cards, gold_id), legal),
        expected,
        unique,
    )
}

#[test]
fn conformance_assets_document_pins_and_exclusions() {
    let readme = include_str!("../contrib/strong-conformance/README.md");
    assert!(readme.contains("88a5ed62638de8c45c0a679c42cd2b05656b93336af9760905d77af04d1e7bca"));
    assert!(readme.contains("df6d4db2476ea35ee193258eec12f4925e1ea4d0fb703283fea3b1d4f82b9a4f"));
    assert!(readme.contains("classified and reported, not treated as parity failures"));
}

#[test]
#[ignore = "requires a pinned user-supplied Gold checkout and Python environment"]
fn gold_upstream_unique_decisions() {
    let root = PathBuf::from(
        env::var_os("GOLD_UPSTREAM_ROOT")
            .expect("run scripts/check-strong-conformance.sh --gold-root PATH"),
    );
    let python = env::var_os("GOLD_PYTHON").unwrap_or_else(|| "python3.11".into());
    let probe =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("contrib/strong-conformance/gold_probe.py");

    let mut requests = Vec::new();
    let mut expected = Vec::new();
    let mut classes = Vec::new();

    let improving = hand("A2.456.789.5K");
    requests.push(format!(
        "draw|{}|{}|2,3\n",
        ids(improving, gold_id),
        gold_id(card("C3"))
    ));
    expected.push(3_u16);
    classes.push("unique draw: strict improvement");

    let equal = hand("A2.456.789.5K");
    requests.push(format!(
        "draw|{}|{}|2,3\n",
        ids(equal, gold_id),
        gold_id(card("DT"))
    ));
    expected.push(2);
    classes.push("unique draw: equal deadwood passes");

    // Generated positions exercise both meld solvers without relying on a
    // source-order tie.  Gin is counted separately below because the hosts
    // expose different action shapes for it.
    let mut deck: Vec<_> = Hand::ALL.iter().collect();
    let mut rng = StdRng::seed_from_u64(0x474f_4c44_434f_4e46);
    let mut ordinary_discards = 0;
    let mut non_gin_knocks = 0;
    let mut gin_cases = 0;
    for iteration in 0..100_000 {
        deck.shuffle(&mut rng);
        let position: Hand = deck.iter().take(11).copied().collect();
        let taken = (iteration % 2 == 0).then_some(deck[0]);
        let (request, action, unique) = gold_discard_request(position, taken);
        if request.is_empty() {
            gin_cases += 1;
            continue;
        }
        if !unique {
            continue;
        }
        let class = if action >= 58 {
            if non_gin_knocks == 32 {
                continue;
            }
            non_gin_knocks += 1;
            "unique non-gin knock"
        } else {
            if ordinary_discards == 128 {
                continue;
            }
            ordinary_discards += 1;
            "unique ordinary discard"
        };
        requests.push(request);
        expected.push(action);
        classes.push(class);
        if ordinary_discards == 128 && non_gin_knocks == 32 {
            break;
        }
    }
    assert_eq!(
        (ordinary_discards, non_gin_knocks),
        (128, 32),
        "the fixed corpus should cover unique ordinary discards and non-gin knocks"
    );

    // The upstream source returns a global gin action (5).  Verify that
    // category, but deliberately do not compare its missing discard id to
    // the native host's discard-and-knock representation.
    let gin = hand("456.TJQ..6789T");
    let mut gin_legal: Vec<String> = gin
        .iter()
        .flat_map(|card| {
            let id = u16::from(gold_id(card));
            [6 + id, 58 + id]
        })
        .map(|id| id.to_string())
        .collect();
    gin_legal.push("5".to_owned());
    requests.push(format!(
        "discard|{}|-|{}\n",
        ids(gin, gold_id),
        gin_legal.join(",")
    ));
    expected.push(5);
    classes.push("classified host adaptation: global gin versus discard-to-gin");

    let input = requests.concat();
    let mut command = Command::new(python);
    command.arg(probe).arg("--root").arg(root);
    let output = run_lines(command, &input);
    assert_eq!(output.len(), expected.len());
    for (index, ((line, expected), class)) in output
        .iter()
        .zip(expected.iter())
        .zip(classes.iter())
        .enumerate()
    {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 3, "case {} ({class})", index + 1);
        assert_eq!(fields.first(), Some(&"ok"), "case {} ({class})", index + 1);
        assert_eq!(
            fields[1].parse::<usize>().expect("a response line number"),
            index + 1,
            "case {} ({class})",
            index + 1,
        );
        let actual: u16 = fields
            .get(2)
            .expect("an action field")
            .parse()
            .expect("a numeric action id");
        assert_eq!(actual, *expected, "case {} ({class})", index + 1);
    }
    eprintln!(
        "Gold parity: 2 draws, {ordinary_discards} ordinary discards, and {non_gin_knocks} non-gin knocks; one explicit gin-category case classified/excluded ({} generated gin positions skipped)",
        gin_cases
    );
}

#[test]
#[ignore = "requires staged pinned MARJJ and EAAI Java sources"]
fn marjj_upstream_unique_decisions() {
    let classpath = env::var_os("MARJJ_TRACE_CLASSPATH")
        .expect("run scripts/check-strong-conformance.sh --marjj-root PATH --eaai-root PATH");
    let improving = hand("A2.456.789.5K");
    let useless = hand("A2.456.789.5K");
    let input = format!(
        "offer|7|{}|{}\noffer|8|{}|{}\n",
        ids(improving, eaai_id),
        eaai_id(card("C3")),
        ids(useless, eaai_id),
        eaai_id(card("DT")),
    );
    let mut command = Command::new("java");
    command.arg("-cp").arg(classpath).arg("ginrummy.MarjjTrace");
    let output = run_lines(command, &input);
    assert_eq!(output.len(), 2);
    let improving_top = card("C3");
    let improving_seen = improving | improving_top.into();
    let expected_improving = strong::MarjjV5Surrogate::<StdRng>::would_take(
        improving,
        improving_top,
        improving_seen,
        Hand::EMPTY,
    );
    let useless_top = card("DT");
    let useless_seen = useless | useless_top.into();
    let expected_useless = strong::MarjjV5Surrogate::<StdRng>::would_take(
        useless,
        useless_top,
        useless_seen,
        Hand::EMPTY,
    );
    assert_eq!(
        output[0],
        format!(
            "ok|1|offer|{}",
            if expected_improving { "take" } else { "pass" }
        )
    );

    // Compare all minimum-deadwood partitions, not only the partition used
    // by one action.  Canonicalizing meld bitsets removes Java collection
    // order from this exact set comparison.
    let mut meld_requests = Vec::new();
    let mut meld_expected = Vec::new();
    let ambiguous = card_hand(&["C4", "C5", "C6", "H5", "S5"]);
    meld_requests.push(format!("melds|{}\n", ids(ambiguous, eaai_id)));
    meld_expected.push(all_meld_trace(ambiguous));
    let mut deck: Vec<_> = Hand::ALL.iter().collect();
    let mut rng = StdRng::seed_from_u64(0x4d41_524a_4a4d_454c);
    for _ in 0..64 {
        deck.shuffle(&mut rng);
        let cards: Hand = deck.iter().take(10).copied().collect();
        meld_requests.push(format!("melds|{}\n", ids(cards, eaai_id)));
        meld_expected.push(all_meld_trace(cards));
    }
    let mut command = Command::new("java");
    command
        .arg("-cp")
        .arg(env::var_os("MARJJ_TRACE_CLASSPATH").expect("a staged Java classpath"))
        .arg("ginrummy.MarjjTrace");
    let meld_output = run_lines(command, &meld_requests.concat());
    assert_eq!(meld_output.len(), meld_expected.len());
    for (index, (line, expected)) in meld_output.iter().zip(&meld_expected).enumerate() {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0], "ok");
        assert_eq!(
            fields[1].parse::<usize>().expect("a response line number"),
            index + 1,
        );
        assert_eq!(fields[2], "melds");
        let actual: Vec<_> = fields[3].split('/').map(str::to_owned).collect();
        assert_eq!(&actual, expected, "minimum-meld set case {}", index + 1);
    }
    assert_eq!(
        output[1],
        format!(
            "ok|2|offer|{}",
            if expected_useless { "take" } else { "pass" }
        )
    );

    // Compare a complete first-discard candidate trace from the public v5
    // source.  This is its embedded "scenario 2" hand, made into a legal
    // first stock turn by holding HJ back as the draw and exposing C2.
    let initial = card_hand(&["CQ", "DQ", "SQ", "SA", "H3", "D5", "H8", "S8", "ST", "SJ"]);
    let upcard = card("C2");
    let drawn = card("HJ");
    assert_eq!(initial.len(), 10);
    assert!(!initial.contains(upcard));
    assert!(!initial.contains(drawn));
    let first_turn = format!(
        "first-turn|2021|{}|{}|{}\n",
        ids(initial, eaai_id),
        eaai_id(upcard),
        eaai_id(drawn),
    );
    let mut command = Command::new("java");
    command
        .arg("-cp")
        .arg(env::var_os("MARJJ_TRACE_CLASSPATH").expect("a staged Java classpath"))
        .arg("ginrummy.MarjjTrace");
    let trace = run_lines(command, &first_turn);
    assert_eq!(trace.len(), 1);
    let fields: Vec<_> = trace[0].split('|').collect();
    assert_eq!(fields.len(), 7);
    assert_eq!(&fields[..4], &["ok", "1", "first-turn", "pass"]);
    let source_discard: u8 = fields[4]
        .strip_prefix("discard=")
        .expect("a discard field")
        .parse()
        .expect("a source discard id");
    let source_candidates = fields[6]
        .strip_prefix("candidates=")
        .expect("a candidate trace");

    let position = initial | drawn.into();
    let seen = position | upcard.into();
    let native =
        strong::MarjjV5Surrogate::<StdRng>::evaluate_discards(position, None, seen, Hand::EMPTY, 1);
    let source: Vec<_> = source_candidates
        .split(';')
        .map(|candidate| {
            let values: Vec<_> = candidate.split(',').collect();
            assert_eq!(values.len(), 5);
            (
                values[0].parse::<u8>().expect("a candidate id"),
                values[1].parse::<u8>().expect("candidate deadwood"),
                java_double(values[2]),
                java_double(values[3]),
                java_double(values[4]),
            )
        })
        .collect();
    assert_eq!(source.len(), native.len());
    for ((id, post, my_value, danger, total), native) in source.iter().zip(&native) {
        assert_eq!(*id, eaai_id(native.card));
        assert_eq!(*post, native.post_deadwood);
        let native_my = f64::from(native.post_deadwood) + 18.0 * native.future_mean;
        assert_float_parity(*my_value, native_my, "future component");
        assert_float_parity(*danger, native.danger, "danger component");
        assert_float_parity(*total, native.total, "total score");
    }

    let best = native
        .iter()
        .map(|candidate| candidate.total)
        .fold(f64::INFINITY, f64::min);
    let tied: Vec<_> = native
        .iter()
        .filter(|candidate| candidate.total == best)
        .map(|candidate| eaai_id(candidate.card))
        .collect();
    let source_best = source
        .iter()
        .map(|candidate| candidate.4)
        .fold(f64::INFINITY, f64::min);
    let source_tied: Vec<_> = source
        .iter()
        .filter(|candidate| candidate.4 == source_best)
        .map(|candidate| candidate.0)
        .collect();
    assert_eq!(source_tied, tied);
    if tied.len() == 1 {
        assert_eq!(source_discard, tied[0]);
    } else {
        eprintln!(
            "classified MARJJ first-turn choice: {} exact minima require host-specific RNG/order",
            tied.len()
        );
    }

    let source_melds = fields[5]
        .strip_prefix("melds=")
        .expect("a final-meld field");
    let remaining = position - from_eaai_id(source_discard).into();
    let native_knocks =
        strong::MarjjV5Surrogate::<StdRng>::should_knock(1, deadwood(remaining), 10);
    assert_eq!(source_melds != "null", native_knocks);
    if native_knocks {
        assert!(
            all_meld_trace(remaining)
                .iter()
                .any(|spread| spread == source_melds)
        );
    }

    // Pin the public source's effective three-turn voluntary knock window,
    // its deadwood boundary, and gin-after-the-window behavior.
    let ten = hand("A23.456.789.K");
    let eleven = hand("A23.456.A4.A5");
    let gin = hand("A234.456.789.");
    assert_eq!(deadwood(ten), 10);
    assert_eq!(deadwood(eleven), 11);
    assert_eq!(deadwood(gin), 0);
    let knock_cases = [(ten, 1_u32), (ten, 3), (ten, 4), (eleven, 1), (gin, 4)];
    let knock_input = knock_cases
        .iter()
        .enumerate()
        .map(|(index, (hand, turns))| {
            format!(
                "knock|{}|{}|{}|false\n",
                index + 1,
                ids(*hand, eaai_id),
                turns
            )
        })
        .collect::<String>();
    let mut command = Command::new("java");
    command
        .arg("-cp")
        .arg(env::var_os("MARJJ_TRACE_CLASSPATH").expect("a staged Java classpath"))
        .arg("ginrummy.MarjjTrace");
    let knock_output = run_lines(command, &knock_input);
    assert_eq!(knock_output.len(), knock_cases.len());
    for (index, (line, (hand, turns))) in knock_output.iter().zip(knock_cases).enumerate() {
        let fields: Vec<_> = line.split('|').collect();
        assert_eq!(fields.len(), 4);
        assert_eq!(fields[0], "ok");
        assert_eq!(
            fields[1].parse::<usize>().expect("a response line number"),
            index + 1,
        );
        assert_eq!(fields[2], "knock");
        let expected = strong::MarjjV5Surrogate::<StdRng>::should_knock(turns, deadwood(hand), 10);
        assert_eq!(fields[3] != "null", expected, "knock case {}", index + 1);
        if expected {
            assert!(
                all_meld_trace(hand)
                    .iter()
                    .any(|spread| spread == fields[3])
            );
        }
    }
    eprintln!(
        "MARJJ parity: two unique opening decisions, {} complete meld-set cases, {} first-turn component records, and {} knock-window cases verified",
        meld_expected.len(),
        native.len(),
        knock_cases.len()
    );
}
