# Complete Functionality Repair Plan Index

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver every approved functionality repair through four independently reviewable plans.

**Architecture:** Execute compatibility and startup work first, then scheduler and transfer lifecycles, then browser/UI integration, and finally release verification. Each plan leaves the branch buildable and supplies interfaces consumed by the next plan.

**Tech Stack:** Rust 1.88+, Tauri 2, Tokio, serde/serde_json, React 18, TypeScript 5.6, Vitest, librqbit 9.0.1

**Spec:** `docs/superpowers/specs/2026-10-03-complete-functionality-repair-design.md`

## Global Constraints

- Preserve legacy task, settings, category-rule, schedule, and queue files; migration-on-read creates a backup before rewriting.
- Never report an active task until its worker or BitTorrent session exists.
- File deletion is opt-in and rejects targets outside the configured save directory or through symlink escape.
- Stable Chromium and Firefox extension IDs and Native Messaging origins do not change.
- Windows and Linux platform behavior requires automated contract tests; unperformed real-platform checks remain manual release items.
- Any failing HIGH audit scenario leaves the delivery in BLOCK status.

## Review Focus

- A partially corrupt legacy array must recover valid records without overwriting the source; pinned in Plan 1 Task 1.
- A crash during atomic replacement must leave either the old or new complete file, never truncated JSON; pinned in Plan 1 Task 1.
- An app restart with queues paused and tasks marked downloading must not bypass queue admission; pinned in Plan 2 Task 1.
- A malicious or symlinked delete target must remain untouched while the task stays visible with an error; pinned in Plan 2 Task 5.
- A stale Native Host port file must not count as a successful connection or launch; pinned in Plan 3 Task 2.

---

Execute and review in this order:

1. [Persistence and startup recovery](2026-10-03-persistence-and-startup-recovery.md)
2. [Scheduler and transfer lifecycle](2026-10-03-scheduler-and-transfer-lifecycle.md)
3. [Browser integration and user interface](2026-10-03-browser-integration-and-ui.md)
4. [Release verification and documentation](2026-10-03-release-verification.md)

Do not start a later plan until the preceding plan's full verification command passes and its commits are reviewed.
