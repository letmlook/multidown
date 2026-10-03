# Browser Integration and User Interface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Native Messaging, extension installation/export, recovery warnings, and completion dialogs truthfully reflect working behavior.

**Architecture:** A small shared Rust crate defines the desktop/Native Host wire protocol and platform port-file contract. The extension uses matching action constants, while Tauri exposes validated unpacked/ZIP artifacts and React renders real completion and recovery state.

**Tech Stack:** Rust, serde, Native Messaging JSON, TCP loopback, Tauri, JavaScript extension code, React/Vitest

**Spec:** `docs/superpowers/specs/2026-10-03-complete-functionality-repair-design.md`

## Global Constraints

- Protocol requests carry version `1` and request IDs; responses echo request IDs.
- Native Host logs never include cookies, authorization headers, or complete sensitive URLs.
- Linux desktop and Native Host resolve the same Tauri application-data port file.
- Extension deployment recursively preserves every manifest-referenced file.
- CRX wording and CRX prerequisites are removed; stable extension IDs remain unchanged.

## Review Focus

- Unknown protocol versions and actions produce structured errors rather than hangs.
- A stale port file fails a full TCP round trip and does not produce “connected”.
- `open_app` times out clearly when the registered protocol cannot start the application.
- Nested extension resources survive deployment and ZIP export with identical relative paths.
- Multiple completion events produce one modal and no lost tasks.

---

### Task 1: Shared Native Messaging protocol

**Files:**
- Create: `integration/native-protocol/Cargo.toml`
- Create: `integration/native-protocol/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `integration/native-host/Cargo.toml`
- Modify: `src-tauri/src/lib.rs`
- Modify: `integration/native-host/src/main.rs`
- Test: `integration/native-protocol/src/lib.rs`

**Interfaces:**
- Produces: `PROTOCOL_VERSION: u16 = 1`.
- Produces: `NativeRequest { version, request_id, action, payload }`, `NativeAction`, `NativeResponse`, and `NativeErrorCode`.
- Consumed by both Rust crates through a path dependency.

- [ ] **Step 1: Add failing protocol serialization tests** for all five actions, request-ID echo, unsupported version, unknown action, and error response shape.
- [ ] **Step 2: Run tests.** Run `cargo test --manifest-path integration/native-protocol/Cargo.toml`; expect failure because the crate does not exist.
- [ ] **Step 3: Implement the shared crate and replace ad hoc Rust action parsing.** Keep payload types action-specific and redact sensitive fields from `Debug` output.
- [ ] **Step 4: Run protocol, app, and host tests.** `cargo test --manifest-path integration/native-protocol/Cargo.toml && cd src-tauri && cargo test --lib && cd ../integration/native-host && cargo test`.
- [ ] **Step 5: Commit.** `git add integration/native-protocol src-tauri/Cargo.toml integration/native-host/Cargo.toml src-tauri/src/lib.rs integration/native-host/src/main.rs && git commit -m "feat: share native messaging protocol"`

### Task 2: Port discovery, connection test, and application launch

**Files:**
- Modify: `integration/native-protocol/src/lib.rs`
- Modify: `integration/native-host/src/main.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `integration/extension/background.js`
- Test: Native Host and app Rust modules

**Interfaces:**
- Produces: `port_file_path(platform: Platform, home: &Path, data_home: Option<&Path>) -> PathBuf` in the shared crate.
- Produces Native Host handlers for `test_connection`, `open_app`, `open_window`, `download`, and `get_config`.
- `open_app` launches `multidown://open` through the OS and waits for a bounded fresh TCP handshake.

- [ ] **Step 1: Add failing platform-path tests** for Windows, macOS, Linux with and without XDG data home, plus stale file, malformed port, refused connection, launch success, and launch timeout tests.
- [ ] **Step 2: Run host and shared-crate tests.** Expect path and action tests to fail.
- [ ] **Step 3: Implement shared path resolution and complete action handlers.** Validate the desktop handshake before returning connected and avoid logging sensitive payload values.
- [ ] **Step 4: Rerun tests** and assert stale files never produce success.
- [ ] **Step 5: Commit.** `git add integration/native-protocol/src/lib.rs integration/native-host/src/main.rs src-tauri/src/lib.rs integration/extension/background.js && git commit -m "fix: complete native host lifecycle"`

### Task 3: Validated unpacked deployment and ZIP export

**Files:**
- Modify: `src-tauri/src/browser_integration.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/components/InstallExtensionModal.tsx`
- Modify: `src/utils/browserInstall.ts`
- Modify: `scripts/build-extension.mjs`
- Test: `src-tauri/src/browser_integration.rs`, `src/utils/browserInstall.test.ts`, and `scripts/build-extension.test.mjs`

**Interfaces:**
- Produces: `validate_extension_directory(path: &Path) -> Result<ExtensionManifest, ExtensionInstallError>` checking every referenced local resource.
- Produces: recursive `deploy_extension_directory(source, destination)`.
- Produces Tauri commands `get_extension_directory` and `export_extension_zip`; removes `get_browser_extension_path` CRX dependency.

- [ ] **Step 1: Add failing tests** for nested icons, missing referenced resources, recursive copy, ZIP path preservation, ZIP generation without CRX, and stable ID verification.
- [ ] **Step 2: Run focused Rust/Node/frontend tests.** Expect failures on current top-level-only copying and CRX prerequisite.
- [ ] **Step 3: Implement recursive validated deployment, ZIP export, and UI copy updates.** Remove or retire `sign:crx` from normal build flow without changing extension IDs.
- [ ] **Step 4: Build and inspect artifacts.** Run `npm run build:extension && npm run test:extension`; inspect ZIP entries in the automated test.
- [ ] **Step 5: Commit.** `git add src-tauri/src/{browser_integration,lib}.rs src/components/InstallExtensionModal.tsx src/utils/browserInstall* scripts/build-extension* package.json && git commit -m "fix: ship complete browser extension artifacts"`

### Task 4: Completion modal and recovery warning UI

**Files:**
- Create: `src/components/CompletionModal.tsx`
- Create: `src/components/CompletionModal.test.tsx`
- Create: `src/components/RecoveryWarnings.tsx`
- Create: `src/components/RecoveryWarnings.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/types/download.ts`
- Modify: `src/index.css`

**Interfaces:**
- Consumes: Plan 1 recovery warning commands and existing `download-finished` events.
- Produces: `CompletionItem { taskId, filename, savePath }` queue state.
- Produces one modal for all unacknowledged completion items when `show_complete_dialog` is true.

- [ ] **Step 1: Add failing React tests** for one completion, burst aggregation, close/acknowledge, disabled-dialog behavior, open-file/open-directory actions, warning listing, recovery path display, and warning acknowledgement.
- [ ] **Step 2: Run focused tests.** `npm run test:run -- src/components/CompletionModal.test.tsx src/components/RecoveryWarnings.test.tsx`; expect failures.
- [ ] **Step 3: Implement both components and replace console-only completion/error handling in `App.tsx`.** Preserve independent system notification settings.
- [ ] **Step 4: Rerun focused tests, full frontend tests, and lint.** Expect zero failures and zero warnings.
- [ ] **Step 5: Commit.** `git add src/App.tsx src/components/CompletionModal* src/components/RecoveryWarnings* src/types/download.ts src/index.css && git commit -m "feat: surface completion and recovery state"`

### Task 5: Plan gate

**Files:** No production changes.

**Interfaces:** Produces browser/UI behavior ready for final release verification.

- [ ] **Step 1: Run frontend and extension gates.** `npm run test:run && npm run lint && npm run build && npm run build:extension && npm run test:extension`.
- [ ] **Step 2: Run all Rust gates.** Test and Clippy the protocol crate, Native Host, and Tauri app.
- [ ] **Step 3: Run `git diff --check` and inspect status.**
- [ ] **Step 4: Commit test-only corrections** as `test: complete browser integration coverage`.
