# Release Verification and Documentation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove the repaired behavior through practical tests, three-platform CI, updated documentation, and a complete release gate.

**Architecture:** Add deterministic local integration tests for real byte transfer and Native Host round trips, extend CI to run the new crates and contracts, then update user/reference documentation from verified behavior only.

**Tech Stack:** Rust integration tests, Node test helpers, GitHub Actions, Markdown documentation, Tauri bundling

**Spec:** `docs/superpowers/specs/2026-10-03-complete-functionality-repair-design.md`

## Global Constraints

- Tests use local deterministic servers except the explicitly labeled public magnet smoke test.
- CI runs on macOS, Ubuntu, and Windows.
- Real-platform checks not performed in CI remain unchecked manual items.
- Documentation must not claim a platform behavior was verified when only its contract test passed.
- No release is acceptable with a failing HIGH scenario.

## Review Focus

- Local HTTP resume verifies final bytes, not only task status.
- Local torrent restart and deletion verify session and filesystem effects.
- Native Host round trip starts from framed stdin and reaches desktop TCP.
- Windows path tests do not depend on the runner user's actual home layout.
- Manual checklist records evidence rather than allowing blanket “passed” marks.

---

### Task 1: Practical integration tests

**Files:**
- Create: `src-tauri/tests/http_resume.rs`
- Create: `src-tauri/tests/torrent_restart.rs`
- Create: `integration/native-host/tests/round_trip.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `integration/native-host/Cargo.toml`

**Interfaces:**
- Consumes: recovery, deletion, and protocol APIs from Plans 1-3.
- Produces deterministic local fixtures that bind loopback ephemeral ports and use temporary directories.

- [ ] **Step 1: Add failing HTTP integration test** that interrupts a Range transfer, reconstructs scheduler state, resumes, and asserts exact final bytes plus no duplicate range.
- [ ] **Step 2: Add failing torrent restart tests** for selected files, active restart, completed seeding policy, keep/delete files, and session removal.
- [ ] **Step 3: Add failing Native Host framed-I/O test** that checks `test_connection`, `open_window`, `download`, unknown action, and stale port behavior against a loopback desktop stub.
- [ ] **Step 4: Run all integration targets.** Run `cargo test --manifest-path src-tauri/Cargo.toml --test http_resume -- --nocapture && cargo test --manifest-path src-tauri/Cargo.toml --test torrent_restart -- --nocapture && cargo test --manifest-path integration/native-host/Cargo.toml --test round_trip -- --nocapture`; expect exact byte, filesystem, session, and protocol assertions to pass. A failure returns the branch to the owning task in Plan 2 or Plan 3 before this plan continues.
- [ ] **Step 5: Commit.** `git add src-tauri/tests src-tauri/Cargo.toml integration/native-host/tests integration/native-host/Cargo.toml && git commit -m "test: add end-to-end recovery coverage"`

### Task 2: Three-platform CI and manual release checklist

**Files:**
- Modify: `.github/workflows/test.yml`
- Create: `docs/development/functionality-release-checklist.md`
- Modify: `docs/development/testing.md`

**Interfaces:**
- Produces CI steps for `integration/native-protocol`, Native Host tests, application unit/integration tests, frontend tests, extension verification, and Tauri build.
- Produces a per-platform checklist for browser install, Native Messaging, tray, notifications, file association, and deep links with evidence fields.

- [ ] **Step 1: Add the exact Plan 4 final-gate commands to `docs/development/testing.md`** and execute each current-platform command once before editing CI.
- [ ] **Step 2: Update the workflow matrix** so all three OS jobs run protocol/host/app tests and build gates; keep OS-specific prerequisites explicit.
- [ ] **Step 3: Write the manual checklist** with separate Windows/macOS/Linux sections, version/build SHA, tester, date, evidence path, and individual pass/fail/not-run fields.
- [ ] **Step 4: Run `npm run docs:check`** and inspect workflow YAML syntax with the repository's available tooling.
- [ ] **Step 5: Commit.** `git add .github/workflows/test.yml docs/development/{functionality-release-checklist,testing}.md && git commit -m "ci: verify repaired functionality across platforms"`

### Task 3: User and architecture documentation

**Files:**
- Modify: `README.md`
- Modify: `docs/architecture/persistence.md`
- Modify: `docs/architecture/download-engine.md`
- Modify: `docs/architecture/bittorrent.md`
- Modify: `docs/architecture/browser-integration.md`
- Modify: `docs/user-guide/getting-started.md`
- Modify: `docs/user-guide/bittorrent.md`
- Modify: `docs/user-guide/browser-extension.md`
- Modify: `docs/user-guide/settings.md`
- Modify: `docs/user-guide/troubleshooting.md`
- Modify: `docs/reference/known-limitations.md`
- Modify: `docs/reference/platform-paths.md`

**Interfaces:**
- Consumes: verified behavior from all preceding tasks.
- Produces user instructions for recovery, deletion choices, unpacked/ZIP extension installation, recovery warnings, and remaining manual platform checks.

- [ ] **Step 1: Update architecture documents** with the versioned stores, startup order, state transitions, deletion transaction, protocol version, and port paths.
- [ ] **Step 2: Update user documents** with automatic recovery, deletion defaults, completion dialog, recovery warnings, and real extension install/export steps.
- [ ] **Step 3: Update limitations only for genuinely unverified external/platform behavior.** Do not remove a promise in place of implementing it.
- [ ] **Step 4: Run `npm run docs:check` and `rg -n "CRX|队列管理器未初始化|test_connection|open_app" README.md docs integration/README.md`**; inspect every remaining match for accuracy.
- [ ] **Step 5: Commit.** `git add README.md docs && git commit -m "docs: document repaired functionality"`

### Task 4: Final verification and branch review

**Files:** No intended production changes.

**Interfaces:** Produces the final evidence bundle and merge recommendation.

- [ ] **Step 1: Run clean dependency installation.** `npm ci`.
- [ ] **Step 2: Run frontend/documentation gates.** `npm run docs:check && npm run test:run && npm run lint && npm run build && npm run build:extension && npm run test:extension`.
- [ ] **Step 3: Run shared protocol and Native Host gates.** Run tests and strict Clippy for both crates, then build Native Host release.
- [ ] **Step 4: Run app gates.** `cd src-tauri && cargo test --all-targets && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 5: Run the public magnet smoke test.** `cd src-tauri && cargo test resolves_real_magnet --lib -- --ignored --nocapture`; record network-dependent evidence separately.
- [ ] **Step 6: Build application bundles.** `npm run tauri -- build`; record output artifact paths.
- [ ] **Step 7: Run `npm audit --omit=dev`, `git diff --check`, and `git status --short`.** Explain rather than hide any unavailable external verification.
- [ ] **Step 8: Request a fresh whole-branch code review** against `origin/main`, fix confirmed findings with focused regression tests, and repeat affected gates.
- [ ] **Step 9: Commit only review corrections** with scoped messages; leave the branch clean and report exact pass/fail counts.
