# gin-rummy-web

Play gin rummy against the [gin-rummy-engine](..) bots in the browser.  The
whole game runs client-side as WebAssembly; there is no server.

## Build

You need the `wasm32-unknown-unknown` target and a `wasm-bindgen` CLI whose
version matches the `wasm-bindgen` crate in `Cargo.lock` (currently 0.2.126):

```console
rustup target add wasm32-unknown-unknown          # once; or distro pkg (below)
cargo install wasm-bindgen-cli --version 0.2.126  # once; match Cargo.lock

cargo build --release --target wasm32-unknown-unknown
wasm-bindgen target/wasm32-unknown-unknown/release/gin_rummy_web.wasm \
    --out-dir pkg --target web                      # writes ./pkg/
```

`wasm-pack build --target web` (`cargo install wasm-pack`) collapses both steps
into one command and picks a matching `wasm-bindgen` for you, if you prefer it.

Notes:

- `.cargo/config.toml` clears a global `-Ctarget-cpu=native` for the wasm build.
  Left in place, that flag (harmless on native builds, meaningless for wasm)
  corrupts the module's target features and `wasm-bindgen` then fails with
  `failed to find intrinsics to enable "clone_ref"`.
- `getrandom = { features = ["wasm_js"] }` in `Cargo.toml` names getrandom's
  browser backend so the wasm target compiles — we never call it (the RNG is
  seeded from JS), but the crate still has to name a backend.
- Distro-packaged (non-rustup) Rust can't `rustup target add`; install the
  target's std via the package manager instead — on Fedora,
  `sudo dnf install rust-std-static-wasm32-unknown-unknown` — or use a rustup
  toolchain.

## Play

Serve this directory over HTTP — ES modules and wasm won't load from `file://`:

```console
python3 -m http.server
# open http://localhost:8000/
```

Pick the bot's difficulty (Easy/Medium/Hard) from the dropdown in the header;
it takes effect on the next new game.  Edit `RULES` at the top of `app.js` to
change ruleset (`modern`/`classic`/`palace`).

Click `Hint` (or press `h`) on your turn for the Monte Carlo solver's read on
the decision: every candidate move with its equity (your chance to win the
game) and expected points, the bot's own pick highlighted.

## Deploy

`pkg/`, `index.html`, `app.js`, and `style.css` are all static — push them to
GitHub Pages or any static host.

## Test

The game logic is native-testable without a browser:

```console
cargo test
```
