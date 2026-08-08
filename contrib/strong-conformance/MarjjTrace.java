package ginrummy;

import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.util.ArrayList;
import java.util.Comparator;
import java.util.Random;

/** A minimal, line-oriented trace driver for the pinned public MARJJ v5. */
public final class MarjjTrace {
    private MarjjTrace() {}

    private static Card[] cards(String field) {
        if (field.isEmpty()) return new Card[0];
        String[] ids = field.split(",");
        Card[] cards = new Card[ids.length];
        for (int i = 0; i < ids.length; i++) {
            cards[i] = Card.getCard(Integer.parseInt(ids[i]));
        }
        return cards;
    }

    private static String spread(ArrayList<ArrayList<Card>> spread) {
        if (spread == null) return "null";
        ArrayList<Long> bits = new ArrayList<>();
        for (ArrayList<Card> meld : spread) {
            bits.add(GinRummyUtil.cardsToBitstring(meld));
        }
        bits.sort(Long::compareUnsigned);
        StringBuilder result = new StringBuilder();
        for (int i = 0; i < bits.size(); i++) {
            if (i != 0) result.append(',');
            result.append(Long.toUnsignedString(bits.get(i), 16));
        }
        return result.toString();
    }

    private static String allMinimumMelds(String[] fields) {
        if (fields.length != 2) throw new IllegalArgumentException("melds needs 2 fields");
        ArrayList<Card> hand = new ArrayList<>();
        for (Card card : cards(fields[1])) hand.add(card);
        ArrayList<ArrayList<ArrayList<Card>>> partitions =
            GinRummyUtil.cardsToBestMeldSets(hand);
        if (partitions.isEmpty()) return "-";
        ArrayList<String> canonical = new ArrayList<>();
        for (ArrayList<ArrayList<Card>> partition : partitions) {
            canonical.add(spread(partition));
        }
        canonical.sort(String::compareTo);
        return String.join("/", canonical);
    }

    private static String doubleBits(double value) {
        return String.format("%016x", Double.doubleToRawLongBits(value));
    }

    private static void seed(MARJJ_v5 player, long value) throws Exception {
        Field random = MARJJ_v5.class.getDeclaredField("random");
        random.setAccessible(true);
        random.set(player, new Random(value));
    }

    @SuppressWarnings("unchecked")
    private static String candidateTrace(MARJJ_v5 player, int nextTurn) throws Exception {
        Field cardsField = MARJJ_v5.class.getDeclaredField("cards");
        cardsField.setAccessible(true);
        ArrayList<Card> hand = (ArrayList<Card>) cardsField.get(player);

        Class<?> helper = Class.forName("ginrummy.MARJJ_v5$Helper");
        Method candidates = helper.getDeclaredMethod(
            "getUnmeldedCards", ArrayList.class, Card.class, int.class);
        candidates.setAccessible(true);
        ArrayList<MARJJ_v5.DiscardStat> stats =
            (ArrayList<MARJJ_v5.DiscardStat>) candidates.invoke(
                null, hand, player.faceUpCard, 5);
        Method future = helper.getDeclaredMethod(
            "getFutureDeadwoodImprovement", ArrayList.class,
            MARJJ_v5.GameState.class, ArrayList.class, int.class);
        Method opponent = helper.getDeclaredMethod(
            "getOpponentDeadwoodImprovement", ArrayList.class, ArrayList.class,
            MARJJ_v5.GameState.class, int.class);
        future.invoke(null, hand, player.gameState, stats, nextTurn);
        opponent.invoke(null, hand, stats, player.gameState, nextTurn);

        ArrayList<MARJJ_v5.DiscardStat> canonical = new ArrayList<>(stats);
        canonical.sort(Comparator.comparingInt(stat -> stat.card.getId()));
        StringBuilder result = new StringBuilder();
        for (int i = 0; i < canonical.size(); i++) {
            MARJJ_v5.DiscardStat stat = canonical.get(i);
            if (i != 0) result.append(';');
            result.append(stat.card.getId()).append(',')
                .append(stat.deadwoodPointsAfterDiscard).append(',')
                .append(doubleBits(stat.myEV)).append(',')
                .append(doubleBits(stat.oppEV)).append(',')
                .append(doubleBits(stat.myEV + stat.oppEV));
        }
        return result.toString();
    }

    private static void selfCheck() throws Exception {
        Class<?> helper = Class.forName("ginrummy.MARJJ_v5$Helper");
        Field initial = helper.getDeclaredField("INIT_WEIGHT");
        Field decay = helper.getDeclaredField("DECAY");
        Field count = helper.getDeclaredField("MAX_CARDS_TO_CONSIDER");
        initial.setAccessible(true);
        decay.setAccessible(true);
        count.setAccessible(true);
        if (initial.getDouble(null) != 18.0
                || decay.getDouble(null) != 0.9
                || count.getInt(null) != 7) {
            throw new AssertionError("unexpected MARJJ v5 future-value constants");
        }
        System.out.println("ok|self-check|18.0|0.9|7");
    }

    private static String offer(String[] fields) throws Exception {
        if (fields.length != 4) throw new IllegalArgumentException("offer needs 4 fields");
        MARJJ_v5 player = new MARJJ_v5();
        seed(player, Long.parseUnsignedLong(fields[1]));
        Card[] hand = cards(fields[2]);
        if (hand.length != 10) throw new IllegalArgumentException("offer needs 10 cards");
        player.startGame(0, 0, hand);
        boolean take = player.willDrawFaceUpCard(Card.getCard(Integer.parseInt(fields[3])));
        return take ? "take" : "pass";
    }

    private static String firstTurn(String[] fields) throws Exception {
        if (fields.length != 5) {
            throw new IllegalArgumentException("first-turn needs 5 fields");
        }
        MARJJ_v5 player = new MARJJ_v5();
        seed(player, Long.parseUnsignedLong(fields[1]));
        Card[] hand = cards(fields[2]);
        if (hand.length != 10) throw new IllegalArgumentException("first-turn needs 10 cards");
        player.startGame(0, 0, hand);
        Card upcard = Card.getCard(Integer.parseInt(fields[3]));
        boolean take = player.willDrawFaceUpCard(upcard);
        Card drawn = Card.getCard(Integer.parseInt(fields[4]));
        player.reportDraw(0, drawn);
        String candidates = candidateTrace(player, 1);
        Card discard = player.getDiscard();
        player.reportDiscard(0, discard);
        ArrayList<ArrayList<Card>> finalMelds = player.getFinalMelds();
        return (take ? "take" : "pass")
            + "|discard=" + discard.getId()
            + "|melds=" + spread(finalMelds)
            + "|candidates=" + candidates;
    }

    private static String knock(String[] fields) throws Exception {
        if (fields.length != 5) throw new IllegalArgumentException("knock needs 5 fields");
        MARJJ_v5 player = new MARJJ_v5();
        seed(player, Long.parseUnsignedLong(fields[1]));
        Card[] hand = cards(fields[2]);
        if (hand.length != 10) throw new IllegalArgumentException("knock needs 10 cards");
        player.startGame(0, 0, hand);
        Field turns = MARJJ_v5.class.getDeclaredField("turns");
        turns.setAccessible(true);
        turns.setInt(player, Integer.parseInt(fields[3]));
        Field opponentKnocked = MARJJ_v5.class.getDeclaredField("opponentKnocked");
        opponentKnocked.setAccessible(true);
        opponentKnocked.setBoolean(player, Boolean.parseBoolean(fields[4]));
        return spread(player.getFinalMelds());
    }

    public static void main(String[] args) throws Exception {
        if (args.length == 1 && args[0].equals("--self-check")) {
            selfCheck();
            return;
        }
        BufferedReader input = new BufferedReader(new InputStreamReader(System.in));
        String raw;
        int line = 0;
        while ((raw = input.readLine()) != null) {
            line++;
            if (raw.isEmpty()) continue;
            String[] fields = raw.split("\\|", -1);
            String result;
            switch (fields[0]) {
                case "offer": result = offer(fields); break;
                case "first-turn": result = firstTurn(fields); break;
                case "melds": result = allMinimumMelds(fields); break;
                case "knock": result = knock(fields); break;
                default: throw new IllegalArgumentException("unknown command: " + fields[0]);
            }
            System.out.println("ok|" + line + "|" + fields[0] + "|" + result);
        }
    }
}
