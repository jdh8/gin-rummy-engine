// Thin renderer over the wasm engine: draw a snapshot, send clicks back.
// The engine runs the whole game synchronously inside each method call, so a
// returned snapshot already reflects the bot's reply.
import init, { WebGame } from './pkg/gin_rummy_web.js';

const BOT = 'mc:64';
const RULES = 'modern';

let game;

// ['glyph', 'colour class'] per suit letter from the engine.
const SUITS = { C: ['♣', 'black'], D: ['♦', 'red'], H: ['♥', 'red'], S: ['♠', 'black'] };
const RANKS = ['', 'A', '2', '3', '4', '5', '6', '7', '8', '9', '10', 'J', 'Q', 'K'];
const label = (c) => RANKS[c.rank] + SUITS[c.suit][0];

async function main() {
  await init();
  newGame();
}

function newGame() {
  // A JS Number seed keeps determinism per game; passed as a decimal string so
  // the engine reads it as an exact u64 (matching the terminal example).
  const seed = String(Math.floor(Math.random() * 2 ** 53));
  game = new WebGame(BOT, RULES, seed);
  render(JSON.parse(game.snapshot()));
}

// Call an engine method and render the snapshot it returns.
function act(method, ...args) {
  render(JSON.parse(game[method](...args)));
}

function cardEl(card, { clickable = false, onclick } = {}) {
  const [glyph, colour] = SUITS[card.suit];
  const el = document.createElement('div');
  el.className = `card ${colour}` + (card.taken ? ' taken' : '') + (clickable && !card.taken ? ' clickable' : '');
  el.innerHTML = `<span class="rank">${RANKS[card.rank]}</span><span class="suit">${glyph}</span>`;
  if (clickable && !card.taken) el.onclick = () => onclick(card);
  return el;
}

function row(cards, opts) {
  const el = document.createElement('div');
  el.className = 'row';
  cards.forEach((c) => el.appendChild(cardEl(c, opts)));
  return el;
}

function button(text, onclick) {
  const b = document.createElement('button');
  b.textContent = text;
  b.onclick = onclick;
  return b;
}

function section(id, heading, node) {
  const el = document.getElementById(id);
  el.innerHTML = '';
  const h = document.createElement('h2');
  h.textContent = heading;
  el.append(h, node);
}

function render(s) {
  document.getElementById('score').textContent =
    `You ${s.you_score} : ${s.bot_score} Bot — round ${s.round_no}`;

  // Opponent's revealed cards.
  const opp = s.opponent_known.length ? row(s.opponent_known) : text('(nothing revealed)');
  section('opponent', 'Bot is holding', opp);

  // Pile top and counts.
  const table = document.createElement('div');
  table.className = 'row';
  if (s.upcard) {
    table.append(text('Pile top:'), cardEl(s.upcard));
  } else {
    table.append(text('Pile empty'));
  }
  table.append(text(`pile ${s.pile_len} · stock ${s.stock_len}`, 'counts'));
  section('table', 'Table', table);

  // Your hand: melds grouped, then loose deadwood. Any card is a legal discard.
  const clickable = s.your_turn && s.phase === 'discard';
  const onclick = (c) => act('discard', c.code);
  const hand = document.createElement('div');
  hand.className = 'row hand';
  s.melds.forEach((meld) => {
    const g = row(meld, { clickable, onclick });
    g.classList.add('meld');
    hand.appendChild(g);
  });
  const loose = row(s.loose, { clickable, onclick });
  loose.classList.add('loose');
  hand.appendChild(loose);
  section('hand', `Your hand — ${s.deadwood} deadwood`, hand);

  renderActions(s);

  const log = document.getElementById('log');
  log.innerHTML = '<h2>Log</h2>' + s.log.map((l) => `<div>${escape(l)}</div>`).join('');
  log.scrollTop = log.scrollHeight;
}

function renderActions(s) {
  const box = document.getElementById('actions');
  box.innerHTML = '';

  if (s.game_over) {
    box.append(text(s.winner === 'you' ? 'You win! 🎉' : 'Bot wins.', 'banner'));
    box.appendChild(button('New game', newGame));
    return;
  }
  if (!s.your_turn) {
    box.append(text('Bot is thinking…'));
    return;
  }
  if (s.phase === 'upcard') {
    box.appendChild(button(`Take ${s.upcard ? label(s.upcard) : 'upcard'}`, () => act('take_upcard')));
    box.appendChild(button('Pass', () => act('pass')));
  } else if (s.phase === 'draw') {
    box.appendChild(button('Draw from stock', () => act('draw_stock')));
    if (s.upcard) box.appendChild(button(`Take ${label(s.upcard)}`, () => act('take_discard')));
  } else if (s.phase === 'discard') {
    box.append(text('Click a card to discard, or'));
    box.appendChild(button('Knock', () => act('knock')));
  }
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
