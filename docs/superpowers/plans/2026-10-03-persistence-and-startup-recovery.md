# Persistence and Startup Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add backward-compatible versioned persistence, recovery diagnostics, and deterministic application initialization.

**Architecture:** A generic JSON store owns envelope detection, backup, quarantine, and atomic writes. Domain modules own migration from `serde_json::Value` into current DTOs, while application setup initializes every required manager before exposing state or recovering tasks.

**Tech Stack:** Rust, serde/serde_json, Tokio filesystem APIs, Tauri managed state

**Spec:** `docs/superpowers/specs/2026-10-03-complete-functionality-repair-design.md`

## Global Constraints

- Current schema version starts at `1`; existing bare arrays and objects are version `0` inputs.
- The first successful legacy rewrite preserves the original as a rotated `.bak` file.
- File-level parse failure never overwrites the source.
- Record-level failure quarantines the original JSON value with domain, index or ID, and error.
- Atomic saves use a same-directory temporary file and replacement.

## Review Focus

- Mixed valid/invalid legacy records recover valid values and preserve invalid JSON in recovery output.
- Failure after writing the temporary file leaves the old destination readable.
- Existing `.bak` files rotate rather than being overwritten.
- Missing settings on first run yields effective defaults and a configured BitTorrent engine.
- `Downloading` values load as recovery candidates rather than active transfers.

---

### Task 1: Generic versioned JSON store

**Files:**
- Create: `src-tauri/src/storage.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/storage.rs`

**Interfaces:**
- Produces: `VersionedEnvelope<T>`, `RecoveryWarning`, `LoadReport<T>`, `StoreError`.
- Produces: `load_store<T>(path: &Path, domain: &'static str, migrate: impl FnOnce(u32, serde_json::Value) -> Result<(T, Vec<RecoveryWarning>), StoreError>) -> Result<LoadReport<T>, StoreError>`.
- Produces: `save_store<T: Serialize>(path: &Path, schema_version: u32, data: &T) -> Result<(), StoreError>`.

- [ ] **Step 1: Write failing storage tests** for version-1 envelope round trip, bare-value version-0 detection, mixed-record quarantine, backup rotation, whole-file corruption preservation, and an injected replace failure that leaves the old file readable.
- [ ] **Step 2: Run the focused tests.** Run `cd src-tauri && cargo test storage::tests -- --nocapture`; expect failures because `storage` and its interfaces do not exist.
- [ ] **Step 3: Implement the store interfaces** with `schema_version`, RFC3339 `written_at`, same-directory temporary files, file flush, atomic replacement, backup rotation, and timestamped `<stem>.recovery-<timestamp>.json` output.
- [ ] **Step 4: Expose `mod storage` and rerun tests.** Run `cd src-tauri && cargo test storage::tests -- --nocapture`; expect all focused tests to pass.
- [ ] **Step 5: Commit.** `git add src-tauri/src/storage.rs src-tauri/src/lib.rs && git commit -m "feat: add versioned atomic persistence"`

### Task 2: Migrate settings, rules, schedules, queues, and batches

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/engine/rules_persistence.rs`
- Modify: `src-tauri/src/engine/schedule.rs`
- Modify: `src-tauri/src/engine/queue.rs`
- Modify: `src-tauri/src/engine/batch.rs`
- Test: the same Rust modules

**Interfaces:**
- Consumes: Plan Task 1 `load_store` and `save_store`.
- Produces: `BatchJobRecord` containing ID, name, input URLs, save directory, task IDs, and creation time.
- Produces: internally tagged `Recurrence` JSON with `type` and optional `days`.
- Produces: `load_*_report(path) -> Result<LoadReport<DomainData>, StoreError>` for each domain.

- [ ] **Step 1: Add failing legacy fixtures** for the existing bare settings object, rule array, Rust recurrence strings/external-tag weekly form, React tagged recurrence form, queue array, and a new complete batch record.
- [ ] **Step 2: Run domain tests.** Run `cd src-tauri && cargo test settings::tests && cargo test engine::rules_persistence::tests && cargo test engine::schedule::tests && cargo test engine::queue::tests && cargo test engine::batch::tests`; expect migration/envelope tests to fail.
- [ ] **Step 3: Route every domain through the store** and derive/implement serde only on persistence DTOs. Keep runtime-only locks and derived batch counts out of stored data.
- [ ] **Step 4: Verify focused migration tests.** Repeat the Task 2 command; expect all legacy and current-format assertions to pass.
- [ ] **Step 5: Commit.** `git add src-tauri/src/settings.rs src-tauri/src/engine/{rules_persistence,schedule,queue,batch}.rs && git commit -m "feat: version management data stores"`

### Task 3: Version task persistence and recovery metadata

**Files:**
- Modify: `src-tauri/src/engine/persistence.rs`
- Modify: `src-tauri/src/engine/types.rs`
- Modify: `src-tauri/src/engine/task.rs`
- Test: `src-tauri/src/engine/persistence.rs`

**Interfaces:**
- Consumes: Task 1 store APIs.
- Produces: `PersistedTaskV1` with optional `completed_at: Option<i64>` and `seeding_started_at: Option<i64>`.
- Produces: `TaskStatus::Recovering`; deserialization maps legacy `downloading` records into a recovery candidate before UI publication.
- Produces: `load_tasks_report(path) -> Result<LoadReport<Vec<PersistedTaskV1>>, StoreError>`.

- [ ] **Step 1: Add failing tests** for the existing HTTP fixture, current torrent fixture, legacy `downloading` normalization, seeding timestamps, a corrupt record beside a valid record, and envelope round trip.
- [ ] **Step 2: Run persistence tests.** Run `cd src-tauri && cargo test engine::persistence::tests -- --nocapture`; expect new assertions to fail.
- [ ] **Step 3: Implement the V1 DTO and conversions** while preserving every existing auth/header/range/torrent field. Keep runtime handles out of the DTO.
- [ ] **Step 4: Rerun persistence tests.** Expect all tests to pass with valid records retained and invalid records quarantined.
- [ ] **Step 5: Commit.** `git add src-tauri/src/engine/{persistence,types,task}.rs && git commit -m "feat: persist recoverable task state"`

### Task 4: Deterministic initialization and recovery warning API

**Files:**
- Modify: `src-tauri/src/engine/scheduler.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/types/download.ts`
- Test: `src-tauri/src/engine/scheduler.rs`

**Interfaces:**
- Consumes: Tasks 1-3 load reports and effective settings defaults.
- Produces: `SchedulerPaths { tasks, queues, batches, rules, schedules }`.
- Produces: `Scheduler::initialize(paths: SchedulerPaths, settings: AppSettings) -> Result<(Scheduler, Vec<RecoveryWarning>), String>`.
- Produces Tauri commands: `list_recovery_warnings() -> Vec<RecoveryWarning>` and `acknowledge_recovery_warnings(ids: Vec<String>)`.

- [ ] **Step 1: Add failing initialization tests** asserting managers are always present, first-run defaults configure `torrent_cfg`, load order restores memberships after tasks, and `Downloading` tasks are not returned as active before recovery.
- [ ] **Step 2: Run scheduler initialization tests.** Run `cd src-tauri && cargo test engine::scheduler::tests::initialization -- --nocapture`; expect failures.
- [ ] **Step 3: Implement `Scheduler::initialize` and replace setup's ad hoc loads.** Manage recovery warnings in Tauri state and register both commands.
- [ ] **Step 4: Run focused and full Rust tests.** Run `cd src-tauri && cargo test --lib`; expect all tests to pass.
- [ ] **Step 5: Commit.** `git add src-tauri/src/engine/scheduler.rs src-tauri/src/lib.rs src/types/download.ts && git commit -m "feat: initialize recoverable scheduler state"`

### Task 5: Plan gate

**Files:** No production changes.

**Interfaces:** Produces a verified base for Plan 2.

- [ ] **Step 1: Run formatting and static analysis.** Run `cd src-tauri && cargo fmt --check && cargo clippy --all-targets -- -D warnings`.
- [ ] **Step 2: Run application tests.** Run `cd src-tauri && cargo test --lib`; expect zero failures.
- [ ] **Step 3: Confirm only intended changes.** Run `git status --short && git diff --check`.
- [ ] **Step 4: Commit any test-only corrections** using `test: complete persistence recovery coverage`; do not proceed with a dirty or failing branch.
