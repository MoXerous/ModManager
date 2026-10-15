# AGENTS.md — ModManager

Persistent instructions for every AI coding agent working in this repository.
Read this file completely at the start of every session. If a prompt conflicts with this file, stop and ask the user.

## 1. What we are building

A brand-new desktop application for **Windows and Linux**: a Minecraft **mod and modpack manager** (Modrinth + CurseForge). It is **not a launcher** and never starts the game.

Goals: high performance, a beautiful and practical UI, room to grow.

A previous Python application exists. It is a **read-only reference for features and behavior** (see `docs/spec/`). Never modify it. Never port its code line by line: re-implement every accepted feature from its behavior description.

## 2. Language policy

- Talk to the user in **Arabic** (technical identifiers stay in English).
- Code, comments, identifiers, commit messages, test names: **English**.
- Documents the user reviews (`docs/spec/*`, ADRs, status reports): **Arabic prose with English identifiers**.

## 3. Sources of truth (highest first)

1. This file.
2. `docs/foundations.md` — stack, architecture, roadmap, risks.
3. `docs/spec/` — behavior spec extracted from the old app. Features have IDs such as `F-012`.
4. `docs/adr/` — architecture decision records.

If sources conflict or something is missing: ask. Do not guess.

## 4. Decided stack (do not change without an ADR and the user's approval)

- Shell: **Tauri 2**. Core: **Rust**. Frontend: **React + TypeScript (strict) + Vite**.
- Styling: **Tailwind CSS 4** (CSS-first config, `@tailwindcss/vite`) + **shadcn/ui**.
- Frontend libs: TanStack Query, TanStack Virtual, Zustand (local UI state only), Wouter, React Hook Form + Zod, i18next (Arabic/RTL + English), Motion, Lucide, cmdk, Biome.
- Core crates: tokio, reqwest (rustls), serde, serde_json, toml, thiserror, tracing + tracing-appender, zip (inside `spawn_blocking`), sha1/sha2, rusqlite + rusqlite_migration, directories, clap, wiremock (tests).
- Types across IPC: **ts-rs** (see ADR 0002). Progress streaming: **Tauri Channels**.
- Tooling: Cargo workspace, pnpm, rustfmt, clippy, GitHub Actions.

## 5. Architecture rules (non-negotiable)

1. `core/` has **no dependency on Tauri or any UI**. The Tauri app and the CLI are thin clients of `core`.
2. The webview never touches the filesystem, the network or SQL. Everything goes through Tauri commands, called only from `src/lib/ipc.ts`.
3. Errors crossing IPC are typed (`IpcError { code, message }`); the UI translates `code`. No raw strings.
4. Every mutation is **plan first, execute second**: build a plan (adds/updates/removals), show it, then apply. Support dry-run.
5. Every download: write to `.part` → verify hash (SHA-1 or SHA-512 per source) → atomic rename. Never trust an unverified file.
6. Archive extraction: use `enclosed_name()`, reject absolute paths, `..` and symlinks, enforce limits on total size, single-file size and file count. Modpacks are untrusted input.
7. Progress goes through Channels, throttled to roughly 100–250 ms.
8. No `unwrap()` / `expect()` in production code. Tests may use them.
9. No hard-coded UI strings: use i18n keys from the first screen. Layout must work in RTL.
10. SQLite: WAL mode, numbered migrations, backup before migrating, and refuse to open a database newer than the supported schema version.
11. CurseForge API key is never committed. Modrinth requests carry a descriptive `User-Agent` with contact info.
12. Disabling a mod = renaming `.jar` to `.jar.disabled`.

## 6. Working agreements

- One small task per session, on its own branch; Conventional Commits.
- **Tests first** for core logic. Test names carry the spec ID, e.g. `spec_f012_rejects_path_traversal`.
- Do not add a dependency before checking its registry page (latest release date, downloads, license, maintenance). If you have no network access, say so and ask. Prefer std or ~100 lines of own code over a tiny unknown crate.
- Do not invent API shapes. Check the official Modrinth / CurseForge docs. If you cannot, ask.
- Never modify the old project, never run destructive commands, never commit secrets.
- Filesystem-touching code and tests run first on temporary directories, never on real mod folders.
- Use a filesystem that supports hard links (NTFS) when running filesystem tests on Windows.

## 7. Definition of done

A task is done only when all of these were run and their results shown:

```
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace          # also regenerates src/bindings; commit the generated files
pnpm build                      # type-checks the frontend
```

If a command cannot run, say exactly why. Never claim something works without running it. Label anything unverified as **غير مُتحقَّق**.

## 8. Report format (Arabic) after every task

- **ما تغيّر**
- **ما تحقّقتُ منه** (الأوامر التي شغّلتها ونتيجتها)
- **ما لم أتحقق منه**
- **أسئلة وقرارات مطلوبة منك**
- **الخطوة التالية المقترحة**

## 9. Repository layout

```
Cargo.toml                workspace: core, cli, src-tauri
core/                     Rust library, no Tauri
  src/ providers/ downloader/ resolver/ instances/ modpack/ jar_reader/ db/ safety/
cli/                      clap client of core
src-tauri/                IPC commands and DTOs only
src/                      React app: app/ features/ components/ lib/ bindings/ (generated)
docs/                     foundations.md, spec/, adr/, baseline*.md
.github/workflows/        ci.yml, bundle.yml
```

## 10. Commands

- Dev app: `pnpm tauri dev`
- CLI: `cargo run -p mm-cli -- <command>`
- Regenerate TS types: `cargo test --workspace`
- Build installers: `pnpm tauri build`
