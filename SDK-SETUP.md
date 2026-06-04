# Liana Signer — SDK setup & next steps

## STATUS 2026-06-02 — SDK structure confirmed against the real CLI

Installed the official `foundation` CLI (github.com/Foundation-Devices/foundation-cli,
`cargo install --path .`) and ran `foundation new liana-signer-sdk --template
multi-page-app`. The generated template is **structurally identical to this app**:
`manifest.toml` (NOT `app-config.toml` — the docs are ahead of the code), the same
`slint-keyos-platform` path deps, the same `src/main.rs` + `ui/pages/*` + `build.rs`
+ `resources/icon.svg` + `i18n/en.json` layout, the `app!()` macro, and `@ui`
widgets. **So this app already IS an SDK-conformant project.** The speculative
`app-config.toml` was removed; `manifest.toml` is the real SDK manifest.

**CLI maturity (important):** only `foundation new` + `foundation develop` are
implemented. `build`/`sign`/`package` are stubs; **`sim`, `sideload`, `cert` do
not exist yet.** So the CLI can scaffold + open the Nix shell, but cannot build
or install to hardware. Use `cargo xtask run --hosted` for the sim (works today).

A reference scaffold from the CLI lives at `../../../../liana-signer-sdk/`.

## Where this landed (done autonomously)

The app was rebuilt onto the **SDK primitives** and is now **device-portable**:

- **SDK primitives:** `slint_keyos_platform::app!`, the `@ui` widget library,
  `security` (app seed → key), `fs` (file I/O), and the file-picker overlay
  (`navigation::select_file`) — exactly what the SDK's own `main.rs` uses.
- **SDK project metadata:** `app-config.toml` (SDK source of truth — identity,
  publisher, permissions), `resources/icon.svg`, `i18n/en.json`.
- **Bitcoin/miniscript on the device-safe stack:** the descriptor/policy/PSBT/
  signing logic now lives in `src/liana/` built on `ngwallet::bdk_wallet`
  (no_std/secp-on-device), instead of a host-only crates.io `miniscript`.
- **Both targets compile:** `cargo xtask check gui-app-liana-signer` passes for
  **armv7a-unknown-xous-elf (device)** and the **simulator** with 0 errors.
- **Tested:** app logic 7/7 (`cargo test -p gui-app-liana-signer`), reference
  crate 8/8 (`liana-signer-core`), real Liana descriptor parses + checksum
  matches, sim boots with no panic.
- Design intent, features, branding, and miniscript standards all preserved.

## What I could NOT do here (needs you)

The official **`foundation` CLI** is not installed and its `build/sim/sideload`
require **Nix**, which I couldn't install — **`sudo` needs a password** and you
were away (Nix's installer creates the `/nix` volume + daemon via sudo).

Good news: **you don't strictly need the foundation CLI** — KeyOS's own
`cargo xtask` flow builds the app for both the simulator and the device, and
that toolchain is already installed here (the ARM/xous toolchain downloaded via
`cargo xtask install-toolchain`, no sudo).

## Next steps — two tracks

### Track A — simulator NOW (no Nix, works today)

From `KeyOS/`:

```bash
cargo xtask run --hosted          # or: just sim   → opens the Passport window
```

The app shows in the dev **Secret Menu** (registered in
`os/gui-app-launcher/src/main.rs`). For real Signet testing with Liana, see
`SIGNET-TEST.md`.

**App device-compilation is proven without Nix:**
```bash
cargo xtask check gui-app-liana-signer   # ARM (xous) + sim, 0 errors
```

But the **full flashable firmware image needs Nix.** `cargo xtask build` pulls
the whole OS — including the `rfal-sys` (NFC) crate, whose `build.rs` needs the
Nix-provided headers and fails outside it (it's unrelated to this app). So a
real on-hardware install requires Track B (or running the device build inside
`nix develop` on the KeyOS flake). Once in Nix:
```bash
cargo xtask build && cargo xtask flash   # signed image + flash over USB (SAM-BA)
```

### Track B — adopt the official Foundation SDK (the `foundation` CLI)

1. **Install Nix** (Terminal, ~3 min — needs your password):
   ```bash
   curl --proto '=https' --tlsv1.2 -sSf -L https://install.determinate.systems/nix | sh -s -- install
   ```
   Open a fresh terminal; `nix --version` to confirm.

2. **Get the `foundation` CLI.** It's a public-beta bundle and is not on this
   machine — it isn't a documented `cargo install`. Obtain it from Foundation
   (you have the contacts) or the developer portal, then `foundation doctor` to
   verify the environment.

3. **Scaffold a clean SDK project and move the code in:**
   ```bash
   foundation new liana-signer --template multi-page-app
   ```
   Then copy into it: `src/liana/`, `src/main.rs`, `src/master_key.rs`,
   `ui/`, `resources/icon.svg`, `i18n/en.json`, and merge this app's
   `app-config.toml` (already in SDK format). The Cargo deps map 1:1 — the SDK
   provides `slint_keyos_platform`, `@ui`, `fs`, `security`, `ngwallet`.

4. **Run + install via the SDK:**
   ```bash
   foundation develop      # enter the SDK Nix shell
   foundation sim          # build + launch the simulator
   foundation cert gen "Foundation"
   foundation sideload     # build, sign, copy to the PRIME USB volume, launch
   ```

> Note: I also tried building just the app's device `.elf` with a minimal
> service set to dodge `rfal-sys` — it compiles the app (device `.rmeta`
> artifacts are produced) but the xtask image-assembly step still routes through
> the full pipeline. So device *compilation* is verified without Nix; producing
> the linked/flashable artifact needs Nix.

## Hardware build: confirmed blocked on macOS (2026-06-02)

With Nix installed and the dev shell working, I attempted the device build:
- `cargo xtask build` (full firmware) → fails on `rfal-sys` (NFC) `build.rs`,
  which is Ubuntu/Linux-oriented ("known-good bindings" copy). KeyOS firmware is
  officially built on **Ubuntu** — macOS is not supported for the full image.
- `cargo build -p gui-app-liana-signer --release --target armv7a-unknown-xous-elf`
  (app only, dodges rfal-sys) → compiles but **fails at link**:
  `undefined symbol: rustsecp256k1_*` (secp C lib not linked outside xtask's
  app-link pipeline, which itself pulls the full image).

**The app is device-compilable** (`cargo xtask check gui-app-liana-signer` = 0
errors, ARM + sim). The final binary just can't be emitted from macOS.

### To install on hardware — pick one
- **`foundation` CLI from https://foundation.xyz/dev** (recommended): it ships
  the blessed build env and `foundation sideload` pushes the app bundle over USB
  — no full-firmware rebuild, no NFC, no Ubuntu. This is the supported path.
- **Ubuntu build host:** clone KeyOS on Ubuntu 22.04/24.04, `just build-all`,
  `just flash` (or sideload).

## Open verification items
- Full signed device image + on-hardware install (needs a physical Prime).
- End-to-end Signet sign with real Liana (see `SIGNET-TEST.md`; build the Liana
  wallet with the sim's exported key so Passport can sign).
- Taproot (`tr`) descriptors (POC is P2WSH only).
