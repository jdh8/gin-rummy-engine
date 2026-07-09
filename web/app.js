// Thin renderer over the wasm engine, paced so each move is a separate,
// animated step.  The engine runs the game synchronously; while it is the
// bot's turn (or a forced human step) JS ticks it on a timer and flies one
// card per step between the deck, the discard pile, your hand, and the
// opponent's fan.
import init, { WebGame } from './pkg/gin_rummy_web.js';

const RULES = 'modern';
const PACE_MS = 650; // pause between the bot's steps, so they can be followed
const FLY_MS = 350; // card glide duration — keep in sync with `.ghost` in style.css

// Adaptive hint depth: everyone starts at a cheap 128 worlds for a fast first
// read, then N drifts toward whatever fits HINT_BUDGET_MS on this device — up
// on fast phones, down on slow ones.  Only decisions that did real work (took a
// meaningful slice of the budget) move N, so easy near-instant reads don't
// inflate it and hard ones back it off fast.
const HINT_BUDGET_MS = 250;
const HINT_MIN = 128;
// The budget, not this ceiling, caps hint time — N only reaches HINT_MAX on
// hardware fast enough to still fit the budget there.  Past ~1024 the pick
// almost never changes; the extra samples just steady the displayed equities on
// a quick device (desktop browsers, mostly).  4096 is another √2 less jitter,
// below the point you'd notice.
const HINT_MAX = 2048;
let hintSamples = 128;

// Four-colour deck, poker convention: clubs green, diamonds blue.
const SUITS = { C: ['♣', 'green'], D: ['♦', 'blue'], H: ['♥', 'red'], S: ['♠', 'black'] };
const SUIT_ORDER = { C: 0, D: 1, H: 2, S: 3 }; // clubs first, as the engine iterates
const RANKS = ['', 'A', '2', '3', '4', '5', '6', '7', '8', '9', '10', 'J', 'Q', 'K'];

let game;
let state; // the snapshot currently on screen (the "before" state during a step)
let busy = false; // blocks input while the bot's turn is animating
let bySuit = false; // loose-card ordering: false = by rank, true = by suit (like the CLI's `v`)

const id = (x) => document.getElementById(x);
const delay = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  await init();
  // A purely cosmetic re-sort of the loose cards, like the terminal's `v`.
  const toggleSort = () => {
    bySuit = !bySuit;
    updateSortButton();
    if (state) render(state);
  };
  id('sort').onclick = toggleSort;
  // The log is hidden by default (grows into a mildly disturbing wall); this
  // reveals it, mirroring the sort toggle.
  const toggleLog = () => {
    document.body.classList.toggle('log-hidden');
    updateLogButton();
  };
  id('logtoggle').onclick = toggleLog;
  document.addEventListener('keydown', (e) => {
    if (e.metaKey || e.ctrlKey) return;
    if (e.key === 'v') toggleSort();
    if (e.key === 'l') toggleLog();
    if (e.key === 'h') showHint();
  });
  updateSortButton();
  updateLogButton();
  // Difficulty picks the bot spec (`greedy` or `mc:samples`); changing it
  // mid-game just starts a fresh one, the simplest way to apply it. Stored
  // across visits so the choice sticks instead of resetting to Medium.
  const saved = localStorage.getItem('difficulty');
  if (saved) id('difficulty').value = saved;
  id('difficulty').onchange = () => {
    localStorage.setItem('difficulty', id('difficulty').value);
    newGame();
  };
  await newGame();
}

function updateSortButton() {
  id('sort').textContent = bySuit ? 'Sort: suit' : 'Sort: rank';
}

function updateLogButton() {
  id('logtoggle').textContent =
    document.body.classList.contains('log-hidden') ? 'Show log' : 'Hide log';
}

async function newGame() {
  // A JS Number seed keeps each game deterministic; passed as a decimal string
  // so the engine reads it as an exact u64 (matching the terminal example).
  const seed = String(Math.floor(Math.random() * 2 ** 53));
  game = new WebGame(id('difficulty').value, RULES, seed);
  state = JSON.parse(game.snapshot());
  render(state);
  busy = true;
  await run(); // pace from the deal to the first human decision
  busy = false;
}

// Apply a human decision, then pace the bot's reply.
async function act(method, ...args) {
  if (busy) return;
  busy = true;
  await step(JSON.parse(game[method](...args)));
  await run();
  busy = false;
}

// Tick the engine while it is the bot's turn or a forced human step, animating
// and pacing each move.
async function run() {
  while (state && !state.your_turn && !state.game_over && !state.round_over) {
    await delay(PACE_MS);
    await step(JSON.parse(game.tick()));
  }
}

// Clear the showdown and deal the next round, then pace to the next decision.
async function continueGame() {
  if (busy) return;
  busy = true;
  state = JSON.parse(game.next_round());
  render(state);
  await run();
  busy = false;
}

// Animate the move that produced `s` over the current view, then render `s`.
async function step(s) {
  const mv = s.last_move;
  if (mv && (mv.kind === 'draw_stock' || mv.kind === 'take' || mv.kind === 'discard')) {
    const { from, to } = endpoints(mv);
    const face = mv.card ? cardByCode(mv.card, s) : null; // null → fly face down
    await flyCard(from, to, face);
  }
  render(s);
  // Point out the human's freshly drawn card in the now-sorted hand.
  if (mv && mv.kind === 'draw_stock' && mv.actor === 'you' && mv.card) {
    id('hand').querySelector(`[data-code="${mv.card}"]`)?.classList.add('justdrawn');
  }
  state = s;
}

// The from/to DOM anchors for a move, read from the current (pre-render) view.
function endpoints(mv) {
  const dest = mv.actor === 'you' ? id('hand') : id('opp');
  if (mv.kind === 'draw_stock') return { from: id('deck'), to: dest };
  if (mv.kind === 'take') return { from: id('discard'), to: dest };
  // discard: the human's card flies from where it sat; the bot's from its fan.
  const from =
    mv.actor === 'you'
      ? id('hand').querySelector(`[data-code="${mv.card}"]`) || id('hand')
      : id('opp');
  return { from, to: id('discard') };
}

// Find the CardJson for a code in this or the previous snapshot, to face the ghost.
function cardByCode(code, s) {
  const pool = (x) =>
    x ? [x.upcard, ...x.melds.flat(), ...x.loose, ...x.opponent_known].filter(Boolean) : [];
  return pool(s).find((c) => c.code === code) || pool(state).find((c) => c.code === code) || null;
}

// Fly a ghost card from one element's centre to another's, then resolve.
function flyCard(fromEl, toEl, face) {
  return new Promise((resolve) => {
    if (!fromEl || !toEl) return resolve();
    const a = fromEl.getBoundingClientRect();
    const b = toEl.getBoundingClientRect();
    const ghost = face ? cardEl(face) : backEl();
    ghost.classList.add('ghost');
    ghost.style.left = `${a.left}px`;
    ghost.style.top = `${a.top}px`;
    document.body.appendChild(ghost);
    const dx = b.left + b.width / 2 - (a.left + a.width / 2);
    const dy = b.top + b.height / 2 - (a.top + a.height / 2);
    requestAnimationFrame(() => {
      ghost.style.transform = `translate(${dx}px, ${dy}px)`;
    });
    let done = false;
    const finish = () => {
      if (!done) {
        done = true;
        ghost.remove();
        resolve();
      }
    };
    ghost.addEventListener('transitionend', finish, { once: true });
    setTimeout(finish, FLY_MS + 150); // fallback if the transition never fires
  });
}

// --- rendering -------------------------------------------------------------

function render(s) {
  id('score').textContent = `You ${s.you_score} : ${s.bot_score} Bot · round ${s.round_no}`;

  const opp = id('opp');
  opp.innerHTML = '';
  opp.append(text('Bot', 'seat'));
  if (s.bot_melds.length || s.bot_loose.length) {
    renderReveal(opp, s); // showdown: the bot's hand, face up
  } else {
    const fan = document.createElement('div');
    fan.className = 'fan';
    for (let i = 0; i < s.bot_hand_len; i++) fan.appendChild(backEl());
    opp.appendChild(fan);
  }

  renderDeck(s);
  renderDiscard(s);
  renderHand(s);
  renderActions(s);
  renderLog(s);
}

function renderDeck(s) {
  const deck = id('deck');
  deck.innerHTML = '';
  if (s.stock_len > 0) {
    const back = backEl();
    if (s.your_turn && s.phase === 'draw') {
      back.classList.add('clickable');
      back.onclick = () => act('draw_stock');
    }
    deck.append(back, text(String(s.stock_len), 'badge'));
  } else {
    deck.appendChild(slotEl());
  }
}

function renderDiscard(s) {
  const d = id('discard');
  d.innerHTML = '';
  if (s.upcard) {
    const c = cardEl(s.upcard);
    if (s.your_turn && (s.phase === 'draw' || s.phase === 'upcard')) {
      c.classList.add('clickable');
      c.onclick = () => act(s.phase === 'upcard' ? 'take_upcard' : 'take_discard');
    }
    d.appendChild(c);
  } else {
    d.appendChild(slotEl());
  }
}

function renderHand(s) {
  const h = id('hand');
  h.innerHTML = '';
  const clickable = s.your_turn && s.phase === 'discard';
  s.melds.forEach((meld) => h.appendChild(group(meld, clickable, true)));
  // Melds keep their grouped order; only the loose deadwood re-sorts.
  const loose = [...s.loose].sort(
    bySuit
      ? (a, b) => SUIT_ORDER[a.suit] - SUIT_ORDER[b.suit] || a.rank - b.rank
      : (a, b) => a.rank - b.rank || SUIT_ORDER[a.suit] - SUIT_ORDER[b.suit],
  );
  h.appendChild(group(loose, clickable, false));
}

// The bot's hand laid face up at the showdown: melds tinted, loose plain, with
// any laid-off cards shown apart.  Nothing here is clickable.
function renderReveal(opp, s) {
  const wrap = document.createElement('div');
  wrap.className = 'hand reveal';
  s.bot_melds.forEach((m) => wrap.appendChild(group(m, false, true)));
  if (s.bot_loose.length) wrap.appendChild(group(s.bot_loose, false, false));
  opp.appendChild(wrap);
  if (s.laid_off.length) {
    const lo = group(s.laid_off, false, false);
    lo.classList.add('laidoff');
    opp.append(text('laid off', 'seat'), lo);
  }
}

function group(cards, clickable, meld) {
  const g = document.createElement('div');
  g.className = meld ? 'group meld' : 'group';
  cards.forEach((c) => {
    const el = cardEl(c);
    el.dataset.code = c.code;
    if (c.taken) el.classList.add('taken');
    if (clickable && !c.taken) {
      el.classList.add('clickable');
      el.onclick = () => act('discard', c.code);
    }
    g.appendChild(el);
  });
  return g;
}

function renderActions(s) {
  const box = id('actions');
  box.innerHTML = '';
  hideHint(); // a solver read is for one decision; drop it when the view changes
  if (s.game_over) {
    box.append(
      text(s.winner === 'you' ? 'You win! 🎉' : 'Bot wins.', 'banner'),
      button('New game', newGame),
    );
    return;
  }
  if (s.round_over) {
    box.append(text(s.result || 'Round over', 'banner'), button('Continue', continueGame));
    return;
  }
  box.append(text(`Deadwood ${s.deadwood}`, 'dead'));
  if (!s.your_turn) {
    box.append(text('Bot is thinking…', 'muted'));
    return;
  }
  if (s.phase === 'upcard') {
    box.append(text('Click the upcard to take it, or'), button('Pass', () => act('pass')));
  } else if (s.phase === 'draw') {
    box.append(text('Click the deck or the discard pile.'));
  } else if (s.phase === 'discard') {
    box.append(text(s.can_knock ? 'Click a card to discard, or' : 'Click a card to discard.'));
    if (s.can_knock) box.append(button('Knock', () => act('knock')));
  }
  box.append(button('Hint', showHint));
}

// Ask the engine's Monte Carlo solver to rate every candidate move and show
// its read for the current decision.
function showHint() {
  if (busy || !state || !state.your_turn) return;
  const t = performance.now();
  const rows = JSON.parse(game.hint(hintSamples));
  adaptHintSamples(performance.now() - t);
  if (!rows.length) return hideHint();
  renderHint(rows);
}

// Nudge the world count toward HINT_BUDGET_MS: back off proportionally when a
// read overruns, climb gently when one had room to spare.  Reads that finished
// in under half the budget are too easy to be a useful speed signal, so leave N
// alone rather than let them ratchet it up.
function adaptHintSamples(ms) {
  if (ms > HINT_BUDGET_MS) hintSamples *= HINT_BUDGET_MS / ms;
  else if (ms > HINT_BUDGET_MS / 2) hintSamples *= 1.3;
  else return;
  hintSamples = Math.round(Math.min(HINT_MAX, Math.max(HINT_MIN, hintSamples)));
}

function hideHint() {
  const panel = id('hint');
  panel.hidden = true;
  panel.innerHTML = '';
}

function renderHint(rows) {
  if (!rows.length) return hideHint();
  const head =
    '<div class="hint-row hint-head"><span>Move</span><span>Equity</span><span>EV</span></div>';
  const body = rows
    .map((r) => {
      const ev = (r.ev >= 0 ? '+' : '') + r.ev.toFixed(1);
      return (
        `<div class="hint-row${r.recommended ? ' best' : ''}">` +
        `<span>${escape(r.action)}</span>` +
        `<span>${(r.equity * 100).toFixed(1)}%</span>` +
        `<span>${ev}</span></div>`
      );
    })
    .join('');
  // The bot deviates from its greedy default only on a statistically clear
  // equity gain, so its pick can sit below a higher point-estimate that is
  // really just sampling noise.  Explain that only when it actually happens.
  const tied = rows[0].recommended
    ? ''
    : '<p class="hint-note">Equities within the sampling noise are ties; the ' +
      'bot holds its default move unless one clearly leads.</p>';
  const panel = id('hint');
  panel.innerHTML =
    '<h2>Solver</h2>' +
    '<p class="hint-note">Equity is your chance to win the game; EV your expected points.</p>' +
    tied +
    head +
    body;
  panel.hidden = false;
}

function renderLog(s) {
  const log = id('log');
  log.innerHTML = '<h2>Log</h2>' + s.log.map((l) => `<div>${escape(l)}</div>`).join('');
  log.scrollTop = log.scrollHeight;
}

// --- element helpers -------------------------------------------------------

function cardEl(card) {
  const [glyph, colour] = SUITS[card.suit];
  const el = document.createElement('div');
  el.className = `card ${colour}`;
  el.innerHTML = `<span class="rank">${RANKS[card.rank]}</span><span class="suit">${glyph}</span>`;
  return el;
}

function backEl() {
  const el = document.createElement('div');
  el.className = 'card back';
  return el;
}

function slotEl() {
  const el = document.createElement('div');
  el.className = 'card slot';
  return el;
}

function button(label, onclick) {
  const b = document.createElement('button');
  b.textContent = label;
  b.onclick = onclick;
  return b;
}

function text(str, cls) {
  const el = document.createElement('span');
  el.textContent = str;
  if (cls) el.className = cls;
  return el;
}

function escape(str) {
  const d = document.createElement('div');
  d.textContent = str;
  return d.innerHTML;
}

main();
