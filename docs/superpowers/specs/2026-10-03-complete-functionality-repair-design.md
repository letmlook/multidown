# MultiDown Complete Functionality Repair Design

**Date:** 2026-10-03  
**Status:** Approved in conversation; pending written-spec review  
**Baseline:** `origin/main` at `21087c8f04de3c8cda8f631650ea2da1dfd73811`

## 1. Purpose

Repair every HIGH and MEDIUM functionality gap identified by the 2026-10-03 audit without reducing documented product promises. The result must preserve existing user data, replace misleading UI state with verified runtime state, and add tests at the component boundaries that allowed CI to remain green while public features were broken.

The implementation covers:

- versioned persistence and backward-compatible migration;
- HTTP and BitTorrent restart recovery;
- queue, batch, category-rule, and schedule lifecycle completion;
- safe task and file deletion;
- persistent BitTorrent seeding policies;
- browser extension packaging and Native Messaging interoperability;
- aggregated completion dialogs and actionable recovery warnings;
- cross-platform automated verification and a release manual-test checklist.

## 2. Guiding Principles

1. Runtime state is only reported after the corresponding worker or BitTorrent session exists.
2. Existing files are never silently discarded or overwritten after a read or migration failure.
3. User-visible state changes and persistent state changes commit only after the underlying operation succeeds.
4. Old persistence formats remain readable through explicit migrations.
5. Cross-process protocols use versioned request and response types rather than duplicated string conventions.
6. File deletion is opt-in, path-constrained, and conservative.
7. Fixes must be demonstrated at the boundary where the original failure occurred, not only through isolated unit tests.

## 3. Delivery Strategy

Use a compatibility-first layered repair:

1. add persistence infrastructure and migrations;
2. establish deterministic application initialization;
3. complete scheduler, queue, batch, schedule, and BitTorrent lifecycles;
4. add safe deletion and completion UI;
5. repair Native Messaging and extension packaging;
6. run full automated and practical verification.

This avoids a high-risk scheduler rewrite while still fixing the shared causes behind the individual findings.

## 4. Versioned Persistence

### 4.1 File envelope

Each persisted domain uses an independent envelope:

```json
{
  "schema_version": 1,
  "written_at": "2026-10-03T12:00:00Z",
  "data": []
}
```

Settings use an object in `data`; collection domains use arrays. Tasks, settings, category rules, schedules, queues, and batches have separate DTOs and migration functions. Runtime structs are not serialized directly.

### 4.2 Compatible reads

Readers accept both the new envelope and the existing bare array/object formats. Missing fields use documented migration defaults. A successful legacy read is rewritten to the current envelope using the atomic-write process.

Before the first rewrite of a legacy file, the original is copied to `<name>.bak`. Existing backups are not overwritten without rotation.

### 4.3 Atomic writes

Writers serialize to a uniquely named temporary file in the destination directory, flush file contents, atomically replace the destination, and sync the directory where supported. A failed write leaves the prior destination intact.

### 4.4 Corrupt records and files

- A record-level migration failure does not block valid records. Failed records are written to a timestamped recovery file containing the source filename, record index or ID, error, and original JSON value.
- A file-level parse failure preserves the original file unchanged, loads an empty in-memory state for that domain, and emits a structured recovery warning.
- Recovery warnings are available through a Tauri command and shown in the UI until acknowledged. They include the recovery path and never claim that all data loaded successfully.

## 5. Deterministic Initialization and Recovery

Application setup follows this order:

1. resolve application data paths;
2. load or create effective default settings;
3. construct and configure the scheduler and BitTorrent engine configuration;
4. initialize and load queues and batches;
5. migrate and load tasks;
6. load category rules, schedule rules, and the persisted schedule-enabled state;
7. normalize transient task states;
8. start schedule processing;
9. recover eligible tasks through normal queue and concurrency admission.

Previously `Downloading` tasks enter an internal recovery phase. They are not exposed as actively downloading until the HTTP worker or BitTorrent session is established. Recovery success transitions to `Downloading`; recovery failure transitions to `Failed` with an actionable error.

User-paused and user-stopped tasks stay paused. Previously active HTTP and BitTorrent tasks resume automatically. Recovery respects queue pause state, queue and global concurrency, schedules, and current speed limits.

## 6. Scheduler and Management Features

### 6.1 Required managers

`Scheduler` initialization requires a `QueueManager`, `BatchManager`, settings, and persistence paths. Required managers are no longer represented as `Option`. Queue commands therefore cannot reach an "uninitialized" production state.

### 6.2 Queues

Persist and restore:

- queue identity and display name;
- ordering and priority;
- paused state;
- concurrency limit;
- task membership and task order.

All task start and recovery paths use the same queue admission logic. Every mutating queue command persists after successful mutation.

### 6.3 Batches

Add a full batch persistence DTO containing batch identity, name, source inputs, save directory, task IDs, and creation time. Completed and failed counts remain derived from current member task states rather than stored as stale counters.

Batch membership is restored after tasks load. Missing task references are retained in recovery diagnostics but omitted from the active batch.

### 6.4 Category rules

HTTP category evaluation occurs after probe metadata is available, including MIME type. Rule reorder rewrites continuous priorities so displayed order and matching order are identical. Mutations persist atomically.

### 6.5 Schedules

The canonical recurrence representation is a tagged object:

```json
{ "type": "daily" }
{ "type": "weekly", "days": ["mon", "wed"] }
```

Rust uses an internally tagged serde representation compatible with the TypeScript model. The migration reader also accepts existing Rust string/external-tag forms and the current React object form.

Persist the global enabled flag, rule enabled state, one-shot execution state, and last-fired data needed to prevent duplicate execution after restart.

## 7. HTTP Recovery

HTTP recovery uses the existing URL, destination, pending ranges, downloaded byte count, range support, ETag, Last-Modified, authentication, and headers.

Before resuming partial content:

- verify the destination and temporary segment state;
- revalidate ETag or Last-Modified when available;
- reject unsafe append/resume when the remote representation changed;
- fall back to an explicit restart path rather than silently combining different representations.

Recovered work uses the normal segmented downloader, retry policy, global rate limiter, and queue admission.

## 8. BitTorrent Lifecycle

### 8.1 First-run initialization

Effective defaults initialize `torrent_cfg` even when no settings file exists. Opening and saving settings is never required before the first magnet or `.torrent` operation.

### 8.2 Session recovery

Persisted metainfo, info hash, selected files, save path, and librqbit session data are used to reattach or add the torrent. Only after a valid handle exists does the task become active.

### 8.3 Seeding policy

Persist wall-clock completion/seeding start time and the upload/statistics baseline required by ratio calculations. On restart:

- already satisfied ratio/time policies do not resume;
- unsatisfied ratio/time policies resume and continue from persisted state;
- `forever` resumes automatically;
- manually paused tasks remain paused.

Policy enforcement is owned by a recoverable supervisor rather than a local `Instant` inside a single worker closure.

### 8.4 Settings updates

Download and upload limits update in place. Session-wide settings that cannot be safely changed, including peer limit where required by librqbit behavior, trigger a controlled session rebuild that reattaches active tasks without losing progress. If rebuilding fails, the old session remains active and the settings command returns a structured failure; the UI does not claim that the new value took effect.

## 9. Safe Task and File Deletion

The UI uses one deletion confirmation for HTTP and BitTorrent tasks. "Delete downloaded files" is off by default and shows the paths that will be affected.

The scheduler deletion transaction is:

1. mark the task as deleting and block new scheduling;
2. stop or join its worker/supervisor;
3. remove any BitTorrent session and fastresume entry;
4. if requested, validate and delete managed files;
5. remove queue and batch references;
6. remove the task and persist all affected domains;
7. emit the final UI event.

If a critical step fails, the task remains visible with an error and deletion can be retried.

Deletion constraints:

- HTTP may delete only its resolved target and owned temporary segment files.
- A single-file torrent may delete only its resolved target file.
- A multi-file torrent may delete only its dedicated torrent root.
- Every candidate is normalized and must remain beneath the configured save directory.
- The save directory itself, arbitrary external paths, and symlink escapes are rejected.

## 10. Browser Extension and Native Messaging

### 10.1 Versioned protocol

Define shared protocol types for `test_connection`, `open_app`, `open_window`, `download`, and `get_config`. Requests include a protocol version and request ID; responses echo the request ID and use a consistent success/error body.

### 10.2 Port-file discovery

Desktop and Native Host use the same platform path resolver and the same filename. Platform-specific path functions are directly unit tested for Windows, macOS, and Linux conventions. Linux uses the Tauri application data location rather than an unrelated XDG config location.

### 10.3 Connection and launch behavior

`test_connection` performs a complete Native Host to desktop TCP round trip. `open_app` focuses a running application; when the application is not running, the Native Host asks the operating system to open a `multidown://open` URL through the registered protocol handler, then waits with a bounded timeout for a fresh port file and successful TCP connection.

Stale port files are detected through failed connection and removed only after validating that they belong to the expected application path.

### 10.4 Extension packaging

The canonical artifact is the validated unpacked directory. Runtime deployment recursively copies all files and directories. Validation checks `manifest.json` and every referenced icon, script, and page.

The product exposes:

- open/install the unpacked extension directory;
- export a ZIP generated from the validated directory.

The nonexistent CRX prerequisite and CRX wording are removed. Stable Chromium and Firefox IDs and Native Messaging allowed origins remain unchanged.

## 11. Completion and Recovery UI

`show_complete_dialog` controls a real in-app completion dialog. Completion events enter a queue; one modal lists all newly completed tasks with per-task "Open file" and "Open directory" actions. Closing the modal acknowledges the displayed events and prevents a batch from creating a modal storm.

System notifications remain independently controlled by `notification_on_complete` and continue to cover hidden/background windows.

Migration and recovery warnings are displayed in an actionable panel with domain, error summary, backup/recovery path, and acknowledgement action.

## 12. Error Handling and Observability

- Tauri commands return structured error codes and messages for recoverable user-facing failures.
- State transitions include task ID, prior state, requested operation, resulting state, and failure reason in debug logs.
- Native Messaging logs request IDs and action names without logging secrets, cookies, authorization headers, or full sensitive URLs.
- Recovery and deletion failures never disappear behind generic console-only errors.

## 13. Verification

### 13.1 Automated tests

Add tests for:

- every legacy persistence format, envelope round trip, backup, atomic-write failure, record quarantine, and whole-file corruption;
- first-run BitTorrent initialization;
- HTTP and BitTorrent auto-recovery, paused non-recovery, and recovery failure;
- delete-keep-files, delete-files, path escape rejection, session cleanup, and failure rollback;
- completed, ratio, time, and forever seeding behavior across restart;
- queue initialization, persistence, ordering, membership, pause state, and admission;
- batch persistence and derived progress;
- schedule JSON compatibility, one-shot idempotence, and global toggle persistence;
- MIME classification and priority reorder;
- all Native Messaging actions and protocol errors;
- platform-specific port paths, application launch, recursive extension copy, validation, and ZIP contents;
- deletion confirmation, completion aggregation, settings outcome, and recovery warnings in React.

### 13.2 Practical tests

- local HTTP Range download and interrupted resume;
- local BitTorrent seeding, selected-file download, deletion, and restart recovery;
- public magnet metadata resolution;
- Native Host to desktop TCP round trip;
- macOS application and DMG build.

### 13.3 Full gate

The existing documentation check, frontend tests, ESLint, frontend build, extension build/identity check, application Rust tests, application Clippy, Native Host tests, Native Host Clippy/build, and Tauri build must all pass.

Windows and Linux must compile and run the new platform-contract tests in CI. Real-browser installation, tray, notification, file association, and deep-link behavior on Windows and Linux remain explicit release-manual-test items and must not be described as verified until performed.

Any failing HIGH scenario leaves the delivery in BLOCK status.

## 14. Documentation and Rollout

Update user and reference documentation to describe actual restart recovery, deletion choices, extension ZIP/unpacked installation, recovery warnings, and platform manual-validation requirements. Documentation changes clarify implemented behavior; they do not weaken existing promises to hide failures.

Rollout is migration-on-read with automatic backup. No destructive one-way migration occurs without a retained legacy copy. The first release containing the change records migration counts and recovery warnings locally for troubleshooting.
