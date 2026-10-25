# Repository Guidelines

## Project Structure & Module Organization

Tunely is a local SSH port-forwarding manager built with Vue 3, JavaScript, Vite, and Tauri 2.

- `src/App.vue` coordinates the main interface; `src/components/` contains reusable Vue components.
- `src/services/api.js` bridges Tauri commands and browser-preview behavior; `src/style.css` contains shared styles.
- `src-tauri/src/` contains Rust modules for commands, models, storage, SSH, tunnels, and desktop lifecycle. Local SSH integration tests live in `tests.rs`.
- `public/` and `src/assets/` hold frontend assets; `src-tauri/icons/` holds application icons.
- `src-tauri/tauri.conf.json` configures development and packaging.

## Build, Test, and Development Commands

Use Bun 1.3.13, Rust 1.89+, and the platform's Tauri prerequisites.

- `bun install --frozen-lockfile`: install pinned frontend dependencies.
- `./dev.sh`: install dependencies and launch the desktop app with hot reload.
- `bun run dev`: serve the browser preview at `http://localhost:1420`.
- `bun run build`: compile the frontend into `dist/`.
- `./build.sh`: build desktop bundles in `src-tauri/target/release/bundle/`.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib`: run Rust tests.
- `cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`: check Rust formatting.

## Coding Style & Naming Conventions

Follow existing formatting: two-space indentation, double quotes, and semicolons in JavaScript; four-space indentation and rustfmt conventions in Rust. Use PascalCase Vue component filenames, camelCase JavaScript functions, and snake_case Rust modules/functions. Keep native calls in the API adapter. No JavaScript formatter or linter is configured.

## Testing Guidelines

Rust tests use `#[test]` and Tokio async tests with temporary local SSH servers. Add descriptive snake_case regression tests for authentication, fingerprints, forwarding, and cleanup changes. No coverage threshold or frontend test runner is configured. Run the frontend build and relevant Rust checks before submitting.

Use only the **ego-browser** skill for frontend page acceptance. Browser preview cannot validate real SSH operations; verify those through Rust tests and the desktop app. Run the ignored `system_credentials_round_trip` test separately when a system credential store is available.

## Commit & Pull Request Guidelines

History contains only `Initial commit`; no established message convention exists. Use concise, imperative commit subjects. PRs should describe behavior changes, link relevant issues, list validation performed, and include screenshots for UI changes.

## Security & Configuration

Keep passwords and key passphrases in the system credential store, never JSON, browser storage, or logs. Preserve host-fingerprint verification. Do not commit private keys or personal connection configurations. Update `bun.lock` or `src-tauri/Cargo.lock` when changing dependencies.
