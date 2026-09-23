# Contributing to Cled

Thanks for helping. Cled is a clipboard sync app, so it handles content people care about keeping
private. Keep that in mind in code, tests, logs, and issues.

## Development setup

### Requirements

- [Rust](https://rustup.rs). The toolchain (stable, with `rustfmt` and `clippy`) is pinned in
  `rust-toolchain.toml`; rustup installs it on first use.
- [Node.js](https://nodejs.org) 22.22 or newer (CI uses 24) and [pnpm](https://pnpm.io). The pnpm
  version is pinned in `package.json`; `corepack enable` picks it up.
- Tauri's system dependencies for your OS: see
  [Tauri prerequisites](https://tauri.app/start/prerequisites/). The README lists the exact
  packages for Fedora and Debian/Ubuntu.

### Running the app

```sh
pnpm install   # JS dependencies
pnpm dev       # desktop app with hot reload
pnpm build     # release installers for the current OS (output in target/release/bundle/)
```

Tauri embeds the built frontend at compile time. Before building or testing the Rust workspace
without `pnpm dev`, build the frontend once:

```sh
pnpm --filter cled-desktop build
```

### Checks

These are the same checks CI runs. Run them before opening a pull request:

```sh
pnpm check                                               # Biome lint/format check + TypeScript
pnpm --filter cled-relay test                            # relay tests
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

`pnpm format` applies Biome's formatting.

### Git hooks

`pnpm install` sets up Git hooks with [Husky](https://typicode.github.io/husky/):

- **pre-commit** runs on staged files only, so it stays fast: Biome lints and formats
  TypeScript, JSON, and CSS; `rustfmt` formats Rust. Fixes are applied and re-staged.
- **pre-push** runs the full checks above (`pnpm check`, `cargo fmt --check`, clippy, and
  tests), the same as CI. It builds the frontend first if `apps/desktop/dist` is missing.

Biome is the project's formatter and linter; there is no Prettier or ESLint. Skip a hook in an
emergency with `--no-verify`, or disable hooks entirely with `HUSKY=0`. CI runs every check
regardless.

Some tests are opt-in because they overwrite your real clipboard and need a desktop session:

```sh
cargo test -p cled-sync --test real_clipboard -- --ignored
```

### Trying things without the full app

```sh
cargo run -p cled-clipboard --example clip -- watch          # print clipboard changes
cargo run -p cled-clipboard --example clip -- read
cargo run -p cled-clipboard --example clip -- write "hello" --hold 5
cargo run -p cled-lan --example lan_peer -- /tmp/cled-peer   # headless peer; type `code` to pair
```

`lan_peer` lets you test pairing and sync against the desktop app with a single computer.

## Project structure

```text
crates/cled-clipboard/  OS clipboard access and change detection
crates/cled-sync/       Clipboard items and sync rules
crates/cled-lan/        Same-network discovery, pairing, and encrypted connections
apps/desktop/src-tauri/ Tauri app: commands, events, tray, glue between the crates
apps/desktop/src/       React + TypeScript + Tailwind UI
apps/relay/             Relay server (Node.js + TypeScript + Fastify), early development
docs/                   Architecture, RFCs, and platform spikes
```

Where things belong:

- **`cled-clipboard`** is the only code that talks to the OS clipboard. It has no Tauri or
  network dependency. Platform-specific code (`#[cfg(target_os = ...)]`, OS clipboard crates)
  lives only in `src/platform/`.
- **`cled-sync`** is pure logic: no networking, no OS calls. It decides what to broadcast and
  what to write (echo suppression, deduplication, newest wins).
- **`cled-lan`** moves items between paired devices: mDNS discovery, pairing, Noise-encrypted
  connections, and the wire format. It decides nothing about what to sync; that's `cled-sync`.
- **`apps/desktop/src-tauri`** only translates between the crates and the UI. No clipboard or
  sync logic here.
- **`apps/desktop/src`** only renders state and calls Tauri commands. The UI never touches the
  clipboard or network directly. Command and event types in `src/lib/ipc.ts` mirror the Rust
  side and are kept in sync by hand.
- **`apps/relay`** is a standalone server that runs on a VPS, not inside the desktop app. It
  routes opaque encrypted payloads and never sees plaintext. Its code is grouped by feature under
  `src/features/`; see [apps/relay/README.md](apps/relay/README.md). It needs only Node.js, not
  Rust or Tauri.

Read [docs/architecture.md](docs/architecture.md) before larger changes, and
[docs/rfcs/0001-lan-sync.md](docs/rfcs/0001-lan-sync.md) before touching networking.

A few changes need extra care. Please open an issue to discuss them first:

- **Wire format, `ContentHash`, or pairing/handshake changes.** These break compatibility between
  devices running different versions.
- **New dependencies.** Add one only when the feature that needs it is being implemented. For the
  UI, prefer CSS (transitions, `@starting-style`) over adding a library.
- **Cryptography.** Cled uses existing, reviewed libraries (`snow`, `spake2`). No custom crypto.

UI changes should honor `prefers-reduced-motion`.

## Contribution workflow

1. Fork the repository.
2. Create a branch from `main` (e.g. `fix/windows-image-read`).
3. Make your change. Keep platform-specific code isolated in `platform/` modules, and keep
   changes focused: one fix or feature per pull request.
4. Run the checks above (the pre-push hook does this for you). Add or update tests where it
   makes sense.
5. Open a pull request and fill in the template, including which platforms you tested on.
   The **Pull request** workflow runs CI on Windows, macOS, and Linux, the dependency audit,
   and a review of any dependencies your change adds.

## Good first contributions

- **Testing on real systems.** Windows, macOS, and Linux desktops other than Hyprland (KDE
  Plasma, GNOME) have not been systematically verified. Reports of what works and what doesn't
  are valuable, even without a code change.
- **Documentation:** setup notes for your OS, clarifications, fixes.
- **Tests**, especially for the Tauri glue and platform behavior.
- **Bug fixes.**
- **UI improvements.**
- **Platform-specific clipboard improvements** in `crates/cled-clipboard/src/platform/`.

## Reporting bugs and security issues

Use the issue templates for bugs and feature requests. Never paste clipboard contents, passwords,
keys, or tokens into an issue.

Report security vulnerabilities privately as described in [SECURITY.md](SECURITY.md), not in a
public issue.

## License

By contributing, you agree that your contributions are licensed under the
[MIT License](LICENSE).

This project follows a [Code of Conduct](CODE_OF_CONDUCT.md).
