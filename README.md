# Cled

**Copy once. Paste anywhere.**

Cled is an open-source, cross-platform clipboard synchronization app. Copy text or an image on one
device and paste it on your other devices — Windows, macOS, and Linux (X11 and Wayland).

> **Status: early development.** Cled can watch, read, and write the local clipboard (text and images) and skips content that password managers mark as private. Sync
> between devices is not implemented yet. Nothing here is usable as a product.

## Stack

- [Tauri 2](https://tauri.app) desktop shell
- Rust for all native functionality (clipboard, platform integration, and later sync/encryption)
- React + TypeScript + Tailwind CSS for the UI

## Repository layout

```text
apps/desktop/          Tauri desktop app
  src/                 React UI
  src-tauri/           Rust entry point and Tauri glue
crates/cled-clipboard/ Clipboard access and change detection (no Tauri dependency)
crates/cled-sync/      Clipboard items and sync rules: loop prevention, deduplication
docs/                  Architecture and design documents
```

## Development

### Prerequisites

- [Rust](https://rustup.rs) (the toolchain is pinned via `rust-toolchain.toml`)
- [Node.js](https://nodejs.org) 22+ and [pnpm](https://pnpm.io) (`corepack enable` works)
- Tauri's platform dependencies — see [Tauri prerequisites](https://tauri.app/start/prerequisites/)

On Fedora:

```sh
sudo dnf install webkit2gtk4.1-devel gtk3-devel libsoup3-devel librsvg2-devel libxdo-devel openssl-devel
```

On Debian/Ubuntu:

```sh
sudo apt install libwebkit2gtk-4.1-dev libgtk-3-dev libsoup-3.0-dev librsvg2-dev libxdo-dev libssl-dev
```

### Commands

```sh
pnpm install        # install JS dependencies
pnpm dev            # run the desktop app with hot reload
pnpm build          # build a release bundle
pnpm check          # lint (Biome) + typecheck (tsc)
cargo fmt --all     # format Rust
cargo clippy --workspace --all-targets
cargo test --workspace
```

To exercise the clipboard without the desktop app:

```sh
cargo run -p cled-clipboard --example clip -- watch          # print clipboard changes
cargo run -p cled-clipboard --example clip -- read
cargo run -p cled-clipboard --example clip -- write "hello" --hold 5
cargo run -p cled-clipboard --example clip -- write-image picture.png --hold 5
```

See [docs/architecture.md](docs/architecture.md) for how the pieces fit together, and
[docs/building.md](docs/building.md) for building and installing on Windows, macOS, and Linux.

Note: Rust builds of the desktop app expect the frontend to have been built once
(`pnpm --filter cled-desktop build`), because Tauri embeds `apps/desktop/dist` at compile time.
`pnpm dev` handles this for you.

## License

To be decided.
