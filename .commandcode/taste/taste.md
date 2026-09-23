- Iteratively debug via compilation: run `cargo check`, fix one error at a time by reading the failing region, re-check, then run `cargo test`. Never batch-fix all errors at once. Confidence: 0.9

- Cross-platform via `#[cfg]` guards with graceful degradation, not panics: every Windows-only feature (sar_bridge module, Tauri commands, Phase 4 streaming logic, exit handlers) is gated behind `#[cfg(target_os = "windows")]`, with `#[cfg(not(target_os = "windows"))]` fallback blocks that log a warning or return a default/`Err` string. Confidence: 0.9

- Type every Tauri invoke on the frontend: always call `invoke<ReturnType>("command_name", { ... })` with an explicit generic so the return shape is checked at the call site. Confidence: 0.85

- Organize Rust with section-divider comments and doc comments: use `// ── Section Name ──` banner dividers to segment Tauri command groups, and write `///` doc comments on every public `#[tauri::command]`. Confidence: 0.8

- Poll backend status with cleanup on the React frontend: when polling needs re-checking, use `setInterval` + `clearInterval` returned from `useEffect`, and guard `setState` calls behind a `mountedRef`/`isMounted` check to avoid state updates after unmount. Confidence: 0.8

- Write a summary/architecture doc after the work, not before: capture the overall approach (e.g., "why we use the prebuilt SAR kernel driver but port the SarClient protocol to Rust in `sar_bridge.rs`") in a markdown file at the repo root. Confidence: 0.75
- Maintain CHANGELOG.md with structured entries for each new feature, using standard changelog format (`### Added` section with descriptive bullets, bold feature names, code spans for CLI flags). Confidence: 0.7
- After frontend changes, verify TypeScript compilation with `npx tsc --noEmit` alongside Rust `cargo check`/`cargo test`, treating both as part of the same verification workflow. Confidence: 0.65
