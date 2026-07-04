// Thin renderer over the wasm engine, paced so each move is a separate,
// animated step.  The engine runs the game synchronously; while it is the
// bot's turn (or a forced human step) JS ticks it on a timer and flies one
// card per step between the deck, the discard pile, your hand, and the
// opponent's fan.
import init, { WebGame } from './pkg/gin_rummy_web.js';

const BOT = 'mc:64';
const RULES = 'modern';
const PACE_MS = 650; // pause between the bot's steps, so they can be followed
const FLY_MS = 350; // card glide duration — keep in sync with `.ghost` in style.css

const SUITS = { C: ['♣', 'black'], D: ['♦', 'red'], H: ['♥', 'red'], S: ['♠', 'black'] };
const RANKS = ['', 'A', '2', '3', '4', '5', '6', '7', '8', '9', '10', 'J', 'Q', 'K'];

let game;
let state; // the snapshot currently on screen (the "before" state during a step)
let busy = false; // blocks input while the bot's turn is animating

const id = (x) => document.getElementById(x);
const delay = (ms) => new Promise((r) => setTimeout(r, ms));

async function main() {
  await init();
  await newGame();
}

async function newGame() {
  // A JS Number seed keeps each game deterministic; passed as a decimal string
  // so the engine reads it as an exact u64 (matching the terminal example).
  const seed = String(Math.floor(Math.random() * 2 ** 53));
  game = new WebGame(BOT, RULES, seed);
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
  while (state && !state.your_turn && !state.game_over) {
    await delay(PACE_MS);
    await step(JSON.parse(game.tick()));
  }
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
  opp.append(text(`Bot — ${s.bot_score}`, 'seat'));
  const fan = document.createElement('div');
  fan.className = 'fan';
  for (let i = 0; i < s.bot_hand_len; i++) fan.appendChild(backEl());
  opp.appendChild(fan);

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
  h.appendChild(group(s.loose, clickable, false));
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
  if (s.game_over) {
    box.append(
      text(s.winner === 'you' ? 'You win! 🎉' : 'Bot wins.', 'banner'),
      button('New game', newGame),
    );
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
