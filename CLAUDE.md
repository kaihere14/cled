# Cled

Open-source, cross-platform clipboard sync. Tauri 2 + Rust (native/core) + React + TypeScript + Tailwind v4.

## UI work: design skill is mandatory

Whenever a task touches the UI — anything under `apps/desktop/src/`, styling, components, layout,
animation, interaction, or UI review — you MUST first load the `emil-design-eng` skill
(`.agents/skills/emil-design-eng/SKILL.md`, also exposed at `.claude/skills/emil-design-eng`)
and follow every practice in it. This is not optional and applies to small tweaks too.

- Load it before writing or editing UI code, not after.
- Skip the skill's "Initial Response" greeting; apply the practices directly to the task.
- When reviewing UI, use the skill's required review format.
- Dependency policy still applies: prefer CSS (transitions, `@starting-style`, WAAPI) over adding
  an animation library. Ask before adding any new UI dependency.
- Always honor `prefers-reduced-motion`.

## Project rules

- Rust owns system functionality (clipboard, platform code, sync, crypto). React only renders UI.
- Platform-specific code (`#[cfg(target_os)]`) lives only in a crate's `platform/` module.
- Add dependencies only when the feature that needs them is being implemented.
- Work milestone by milestone and stop for review after each.

## Commands

```sh
pnpm dev                                          # run desktop app
pnpm check                                        # Biome + tsc
cargo fmt --all && cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```
