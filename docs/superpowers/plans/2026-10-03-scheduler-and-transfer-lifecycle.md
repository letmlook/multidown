# Scheduler and Transfer Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make queue, batch, schedule, HTTP recovery, BitTorrent recovery/seeding, and task deletion behave correctly across restarts.

**Architecture:** Recovery and mutation enter through scheduler-owned lifecycle methods. Managers persist after successful mutations, while workers and torrent sessions determine whether tasks may transition into active states.

**Tech Stack:** Rust, Tokio, reqwest, librqbit, Tauri commands, React/Vitest for deletion UI

**Spec:** `docs/superpowers/specs/2026-10-03-complete-functionality-repair-design.md`

## Global Constraints

- Consume the persistence and initialization interfaces from Plan 1.
- Auto-recovery respects queue pause and concurrency state.
- User-paused tasks never auto-resume.
- A failed delete keeps the task visible with an actionable error.
- File deletion is off by default and path constrained.

## Review Focus

- Recovery with a paused queue cannot start a worker.
- HTTP resume rejects a changed ETag/Last-Modified rather than appending corrupt bytes.
- Completed torrents resume only when their persisted seeding policy remains unsatisfied.
- Removing a paused/completed torrent always deletes its librqbit session.
- Symlink and `..` deletion candidates never escape the configured save directory.

---

### Task 1: Complete queue and batch lifecycles

**Files:**
- Modify: `src-tauri/src/engine/queue.rs`
- Modify: `src-tauri/src/engine/batch.rs`
- Modify: `src-tauri/src/engine/scheduler.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: the same engine modules

**Interfaces:**
- Consumes: initialized non-optional managers and `save_store` from Plan 1.
- Produces: scheduler queue mutations that persist before returning success.
- Produces: `BatchManager::restore(Vec<BatchJobRecord>, known_task_ids: &HashSet<TaskId>) -> Vec<RecoveryWarning>`.

- [ ] **Step 1: Add failing tests** for create/update/reorder/pause persistence, task membership restoration, paused-queue recovery admission, batch round trip, and missing batch task references.
- [ ] **Step 2: Run focused tests.** Run `cd src-tauri && cargo test engine::queue::tests -- --nocapture && cargo test engine::batch::tests -- --nocapture && cargo test engine::scheduler::tests::queue -- --nocapture`; expect lifecycle tests to fail.
- [ ] **Step 3: Implement persistence after successful mutations** and route all start/recovery paths through one queue admission function.
- [ ] **Step 4: Rerun focused tests** and expect queue and batch state to survive manager reconstruction.
- [ ] **Step 5: Commit.** `git add src-tauri/src/engine/{queue,batch,scheduler}.rs src-tauri/src/lib.rs && git commit -m "feat: persist queue and batch lifecycles"`

### Task 2: Repair schedules and category rules

**Files:**
- Modify: `src-tauri/src/engine/schedule.rs`
- Modify: `src-tauri/src/engine/rules.rs`
- Modify: `src-tauri/src/engine/scheduler.rs`
- Modify: `src/components/OptionsModal.tsx`
- Modify: `src/types/download.ts`
- Test: Rust modules and `src/components/OptionsModal.test.tsx`

**Interfaces:**
- Consumes: Plan 1 tagged `Recurrence` and store APIs.
- Produces: `ScheduleStateRecord { enabled: bool, last_fired: ... }` persisted with rules.
- Produces: `apply_category_rules(filename: &str, url: &str, mime: Option<&str>)` after HTTP probe.

- [ ] **Step 1: Add failing Rust and React tests** for daily/weekly IPC JSON, legacy recurrence migration, one-shot non-repetition, toggle persistence, MIME matching, and reorder rewriting continuous priorities.
- [ ] **Step 2: Run focused tests.** Run `cd src-tauri && cargo test engine::schedule::tests && cargo test engine::rules::tests && cd .. && npm run test:run -- src/components/OptionsModal.test.tsx`; expect contract failures.
- [ ] **Step 3: Align serde and TypeScript contracts, persist schedule state, pass probe MIME, and rewrite priorities on reorder.**
- [ ] **Step 4: Repeat focused tests** and expect all schedule/rule assertions to pass.
- [ ] **Step 5: Commit.** `git add src-tauri/src/engine/{schedule,rules,scheduler}.rs src/components/OptionsModal.tsx src/components/OptionsModal.test.tsx src/types/download.ts && git commit -m "fix: connect schedules and category rules"`

### Task 3: Recover interrupted HTTP downloads

**Files:**
- Modify: `src-tauri/src/engine/scheduler.rs`
- Modify: `src-tauri/src/network/client.rs`
- Test: `src-tauri/src/engine/scheduler.rs`

**Interfaces:**
- Produces: `Scheduler::recover_tasks(app_handle, max_connections, network_options) -> RecoverySummary`.
- Produces: `validate_resume_identity(expected_etag, expected_last_modified, probe) -> Result<(), ResumeError>`.

- [ ] **Step 1: Add a failing local HTTP server test** covering valid Range resume, changed ETag rejection, non-Range restart requirement, paused non-recovery, and queue-limited recovery.
- [ ] **Step 2: Run the recovery tests.** Run `cd src-tauri && cargo test engine::scheduler::tests::http_recovery -- --nocapture`; expect failures before recovery exists.
- [ ] **Step 3: Implement HTTP recovery** through normal segmented download, retry, rate-limit, and queue admission paths; transition to active only after worker creation.
- [ ] **Step 4: Rerun focused tests** and assert bytes on disk exactly match server content.
- [ ] **Step 5: Commit.** `git add src-tauri/src/engine/scheduler.rs src-tauri/src/network/client.rs && git commit -m "feat: resume interrupted http downloads"`

### Task 4: Recover BitTorrent sessions and seeding policies

**Files:**
- Modify: `src-tauri/src/engine/types.rs`
- Modify: `src-tauri/src/engine/task.rs`
- Modify: `src-tauri/src/engine/scheduler.rs`
- Modify: `src-tauri/src/torrent/engine.rs`
- Modify: `src-tauri/src/settings.rs`
- Modify: `src/components/options/BitTorrentSettings.tsx`
- Test: `src-tauri/src/torrent/tests.rs` and scheduler tests

**Interfaces:**
- Produces: recoverable seeding supervisor using persisted `completed_at` and `seeding_started_at` wall-clock seconds.
- Produces: `TorrentEngine::reconfigure_session(config) -> Result<ReconfigureOutcome, TorrentError>`.

- [ ] **Step 1: Add failing tests** for first-run defaults, active torrent restart, paused non-recovery, completed ratio/time/forever restoration, already-satisfied policies, and peer-limit session rebuild rollback.
- [ ] **Step 2: Run focused BT tests.** Run `cd src-tauri && cargo test torrent -- --nocapture && cargo test engine::scheduler::tests::torrent_recovery -- --nocapture`; expect new lifecycle tests to fail.
- [ ] **Step 3: Move seeding supervision out of the local worker closure, reattach sessions from persisted metainfo/fastresume, and implement controlled session reconfiguration.**
- [ ] **Step 4: Rerun focused tests** and expect byte-consistent local torrent transfer plus policy assertions to pass.
- [ ] **Step 5: Commit.** `git add src-tauri/src/engine/{types,task,scheduler}.rs src-tauri/src/torrent/{engine,tests}.rs src-tauri/src/settings.rs src/components/options/BitTorrentSettings.tsx && git commit -m "feat: recover torrent and seeding state"`

### Task 5: Safe deletion backend and UI

**Files:**
- Modify: `src-tauri/src/engine/scheduler.rs`
- Modify: `src-tauri/src/torrent/engine.rs`
- Modify: `src-tauri/src/lib.rs`
- Create: `src/components/DeleteTaskModal.tsx`
- Create: `src/components/DeleteTaskModal.test.tsx`
- Modify: `src/App.tsx`
- Modify: `src/components/Toolbar.tsx`
- Test: scheduler and torrent tests

**Interfaces:**
- Produces: `DeletionPreview { task_id: String, paths: Vec<String>, can_delete_files: bool }`.
- Produces: `Scheduler::preview_task_deletion(task_id: &str) -> Result<DeletionPreview, TaskOperationError>`.
- Produces: `Scheduler::remove_task(task_id: &str, delete_files: bool) -> Result<(), TaskOperationError>`.
- Tauri command consumes `{ taskId, deleteFiles }`.

- [ ] **Step 1: Add failing backend tests** for running/paused/completed torrent session removal, keep/delete file choices, HTTP temp cleanup, task retention after failure, `..` escape, save-root deletion, and symlink escape.
- [ ] **Step 2: Add failing UI tests** asserting the checkbox defaults off, paths are shown, cancel does nothing, and confirm sends the selected `deleteFiles` value.
- [ ] **Step 3: Run focused tests.** Run `cd src-tauri && cargo test engine::scheduler::tests::deletion -- --nocapture && cargo test torrent::tests::deletion -- --nocapture && cd .. && npm run test:run -- src/components/DeleteTaskModal.test.tsx`; expect failures.
- [ ] **Step 4: Implement the deletion transaction and replace all direct remove invocations with the modal flow.** Remove session/worker before persistent records and retain the task on critical failure.
- [ ] **Step 5: Repeat focused tests** and verify forbidden paths remain present on disk.
- [ ] **Step 6: Commit.** `git add src-tauri/src/engine/scheduler.rs src-tauri/src/torrent/engine.rs src-tauri/src/lib.rs src/App.tsx src/components/{Toolbar,DeleteTaskModal}.tsx src/components/DeleteTaskModal.test.tsx && git commit -m "feat: add safe task deletion"`

### Task 6: Plan gate

**Files:** No production changes.

**Interfaces:** Produces verified lifecycle behavior for Plan 3.

- [ ] **Step 1: Run `cargo fmt --check`, full Rust tests, and strict Clippy.** `cd src-tauri && cargo fmt --check && cargo test --lib && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 2: Run frontend tests and lint.** `npm run test:run && npm run lint`.
- [ ] **Step 3: Run `git diff --check` and inspect status.**
- [ ] **Step 4: Commit test-only corrections** as `test: complete transfer lifecycle coverage` before proceeding.
