# Multidown Documentation Overhaul Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the current README-centric documentation with a Chinese-first, audience-layered documentation center that accurately describes Multidown v0.3.0 and can be checked automatically for drift.

**Architecture:** Keep `README.md` as the concise public landing page and make `docs/README.md` the canonical navigation hub. Split task-oriented user and developer guides from architecture and reference material, migrate historical research out of the current-state path, and add a Node-based documentation checker for internal links and pinned project facts.

**Tech Stack:** Markdown, Node.js 20/22 ESM, Vitest, npm scripts, Git/GitHub Actions conventions already used by the repository.

**Spec:** `docs/superpowers/specs/2026-09-27-documentation-overhaul-design.md`

## Global Constraints

- Primary language is Simplified Chinese; README includes only a short English description, not a full English mirror.
- Use `Multidown` for the project/repository and `MultiDown` for the displayed product and installer names.
- Current-state claims must be supported by `main` code/configuration, automated tests, workflows, the public v0.3.0 release, or CHANGELOG.
- Do not describe planned functionality as supported; expose Web Seed, MSE/PE, sequential downloading, and public-network test gaps explicitly.
- Do not add a documentation site generator, deploy a docs site, upgrade dependencies, change product behavior, or fabricate screenshots.
- Commands must exist in `package.json` or be valid Cargo commands for the referenced manifest.
- Preserve historical source material through Git moves into `docs/design/` or `docs/archive/`; do not delete useful research.
- All commits use `letmlook <letmlook@aliyun.com>`.

## Review Focus

- A Markdown link containing Chinese characters, spaces, an anchor, or `../` must resolve to the intended file and heading; Task 1 tests these forms.
- A fenced code block containing URL-like text must not be treated as a Markdown link; Task 1 tests parser behavior.
- User-facing install guidance must distinguish a release install from a source build and must not require developer tools for normal use; Tasks 3 and 4 review both routes.
- Browser integration guidance must keep the Chromium ID `bceackgdejcgphcbhinfgejepgoeiail` and Firefox ID `multidown@letmlook` separate; Tasks 1 and 4 pin both facts.
- Release documentation must require the tag commit, four successful build jobs, and all nine assets before publication; Tasks 6 and 8 verify this checklist.

---

### Task 1: Documentation Validation Contract

**Files:**
- Create: `scripts/check-docs.mjs`
- Create: `scripts/check-docs.test.mjs`
- Modify: `package.json`
- Modify: `.github/workflows/test.yml`

**Interfaces:**
- Produces: `validateDocumentation({ rootDir, markdownFiles, packageJson }) -> string[]`, returning human-readable validation errors.
- Produces: `npm run docs:check`, the single command used by later tasks and CI/manual verification.
- Validates: Markdown relative file links and anchors, package script references, project/product naming, v0.3.0, Chromium/Firefox IDs, and the expected nine release asset names where those facts appear.

- [ ] **Step 1: Add failing Vitest cases for relative paths, anchors, ignored external links, fenced code, missing npm scripts, and the two browser IDs**

Use temporary fixture directories in `scripts/check-docs.test.mjs`. Assert that valid fixtures return `[]` and each invalid fixture returns a message naming the source file and broken target or fact.

- [ ] **Step 2: Run the focused tests and confirm failure**

Run: `npx vitest run scripts/check-docs.test.mjs`

Expected: FAIL because `scripts/check-docs.mjs` does not exist.

- [ ] **Step 3: Implement the checker and add the npm script**

Export `validateDocumentation` for tests. When executed directly, discover tracked `*.md` files under the repository, print every error, and exit non-zero on failure. Add `"docs:check": "node scripts/check-docs.mjs"` to `package.json`.

- [ ] **Step 4: Add the documentation check to PR CI**

Add `npm run docs:check` after dependency installation and before the build steps in `.github/workflows/test.yml`, so broken documentation blocks all platform jobs before expensive packaging.

- [ ] **Step 5: Run focused and existing frontend tests**

Run: `npx vitest run scripts/check-docs.test.mjs && npm run test:run`

Expected: checker tests and the existing four frontend tests pass.

- [ ] **Step 6: Commit**

```bash
git add .github/workflows/test.yml scripts/check-docs.mjs scripts/check-docs.test.mjs package.json
git commit -m "test(docs): 添加文档一致性校验"
```

### Task 2: Establish the Documentation Tree and Preserve History

**Files:**
- Create: `docs/README.md`
- Create directories: `docs/user-guide/`, `docs/development/`, `docs/architecture/`, `docs/reference/`, `docs/archive/research/`
- Move: `docs/IDM核心原理与功能模块分析.md` → `docs/archive/research/IDM核心原理与功能模块分析.md`
- Move: `docs/技术栈选型分析.md` → `docs/archive/research/技术栈选型分析.md`
- Move: `docs/开发计划.md` → `docs/archive/开发计划-v0.3.0前.md`
- Move: `docs/磁力链接与种子下载实施方案.md` → `docs/design/磁力链接与种子下载实施方案.md`
- Move: `docs/功能模块.md` → `docs/archive/功能模块设计-v0.3.0前.md`
- Modify: `README.md`
- Modify: links in moved documents that reference repository files or sibling assets.

**Interfaces:**
- Produces: the canonical `docs/README.md` navigation categories consumed by every new guide.
- Preserves: original historical content and Git history; current-state pages may link to it as background only.

- [ ] **Step 1: Move historical documents with `git mv`**

Keep `docs/design/图标与Logo设计说明.md`, design assets, screenshots, and `docs/superpowers/` in place.

- [ ] **Step 2: Create the documentation hub with audience-based reading routes**

Include routes for “我想安装和使用”, “我想参与开发”, “我想理解架构”, and “我负责发布维护”. Mark new topic links as part of the same change only after their target files exist in later tasks; until then use a checklist without broken links.

- [ ] **Step 3: Repair links affected by moves**

Check README's existing document links, repository-relative paths, image paths under `docs/idm/`, and links between historical documents. This step only repairs paths; Task 3 performs the README rewrite.

- [ ] **Step 4: Verify moves and current links**

Run: `npm run docs:check`

Expected: PASS for every file currently present; no placeholder link is emitted.

- [ ] **Step 5: Commit**

```bash
git add README.md docs
git commit -m "docs: 建立分层文档目录并归档历史资料"
```

### Task 3: Rewrite the Public Landing and Governance Documents

**Files:**
- Rewrite: `README.md`
- Create: `CONTRIBUTING.md`
- Create: `SECURITY.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/README.md`

**Interfaces:**
- Consumes: Task 2 navigation categories.
- Produces: concise public entry points for users, contributors, security reporters, and release history.

- [ ] **Step 1: Rewrite README as a landing page**

Use the approved order: positioning and English summary; v0.3.0 download; core capabilities; supported platforms and limitations; user quick start; developer quick start; documentation routes; contributing/security/license. Replace deep tutorials with links to the owning guides.

- [ ] **Step 2: Add contribution rules**

Document environment floors, branch/commit expectations, required identity `letmlook <letmlook@aliyun.com>` for project-owned commits, test commands, documentation checks, PR scope, and review expectations.

- [ ] **Step 3: Add security reporting guidance**

State supported release scope, security boundaries, what information a report should contain, and that suspected vulnerabilities should not be disclosed in public issues. Use GitHub private vulnerability reporting when enabled; otherwise direct reporters to the repository owner through a private channel without inventing an email address.

- [ ] **Step 4: Normalize CHANGELOG and hub links**

Keep released facts intact, ensure v0.3.0 links to the public release, and add all root documents to `docs/README.md` navigation.

- [ ] **Step 5: Verify**

Run: `npm run docs:check`

Expected: PASS, with README free of duplicated long-form installation or architecture content.

- [ ] **Step 6: Commit**

```bash
git add README.md CHANGELOG.md CONTRIBUTING.md SECURITY.md docs/README.md
git commit -m "docs: 重写项目首页与贡献规范"
```

### Task 4: Write the User Guide

**Files:**
- Create: `docs/user-guide/installation.md`
- Create: `docs/user-guide/getting-started.md`
- Create: `docs/user-guide/bittorrent.md`
- Create: `docs/user-guide/browser-extension.md`
- Create: `docs/user-guide/settings.md`
- Create: `docs/user-guide/troubleshooting.md`
- Modify: `docs/README.md`

**Interfaces:**
- Consumes: release asset names, app settings types, browser manifests, Native Host manifests, and current UI behavior.
- Produces: task-oriented end-user instructions; developer docs link here for manual acceptance scenarios instead of duplicating them.

- [ ] **Step 1: Write installation and first-run guides**

Cover choosing the correct Windows/macOS/Linux asset, system requirements, installation, first launch, adding an HTTP task, task controls, and uninstall expectations. Separate release use from source builds.

- [ ] **Step 2: Write the BitTorrent guide**

Cover magnets, `.torrent` files, metadata placeholders, file selection, seeding policies, SOCKS5 privacy behavior, system association choices, and current limitations.

- [ ] **Step 3: Write browser integration guidance**

Separate Chromium and Firefox instructions; explain Native Host installation, extension identity, verification, safe manual loading, browser capture rules, and fallback deep links.

- [ ] **Step 4: Write settings and troubleshooting guides**

Organize settings by user intent. Troubleshooting must be symptom-first and include extension connection, Native Host, failed HTTP probes, proxy/TLS, BT metadata, permissions, and platform integration.

- [ ] **Step 5: Link the user route and verify**

Run: `npm run docs:check`

Expected: PASS; each user guide is reachable from `docs/README.md` and README.

- [ ] **Step 6: Commit**

```bash
git add README.md docs/README.md docs/user-guide
git commit -m "docs: 补全安装使用与故障排查指南"
```

### Task 5: Write the Developer Guide

**Files:**
- Create: `docs/development/setup.md`
- Create: `docs/development/project-structure.md`
- Create: `docs/development/testing.md`
- Create: `docs/development/building.md`
- Create: `docs/development/release.md`
- Rewrite: `TESTING_GUIDE.md` as a migration page.
- Rewrite: `integration/README.md`
- Modify: `docs/README.md`

**Interfaces:**
- Consumes: `package.json`, Cargo manifests, Tauri platform configs, `.github/workflows/test.yml`, `.github/workflows/release.yml`, and Task 1's validation command.
- Produces: the canonical build/test/release instructions used by CONTRIBUTING and future maintainers.

- [ ] **Step 1: Document environment setup and repository structure**

Include Node.js 20/22, Rust 1.88+, platform prerequisites, dependency installation, component boundaries, and the required Native Host/frontend build ordering.

- [ ] **Step 2: Document automated and manual testing**

Include `npm run test:run`, `npm run lint`, `npm run test:extension`, `npm run docs:check`, frontend build, app tests/Clippy, Native Host tests/Clippy, the ignored public DHT test, and platform acceptance scope.

- [ ] **Step 3: Document component and release builds**

Cover extension outputs, Native Host outputs, Tauri development/build commands, platform bundle locations, version synchronization, PR CI, annotated tag, draft inspection, four release jobs, nine asset names, and publication verification.

- [ ] **Step 4: Rewrite the integration directory entry**

Describe its two components and link to user installation, architecture, build, and troubleshooting pages. Remove placeholder IDs and stale manual-registration instructions.

- [ ] **Step 5: Replace the root testing guide with a migration entry**

Point manual user acceptance to the relevant user guides and automated testing to `docs/development/testing.md`; do not retain a second copy of the procedures.

- [ ] **Step 6: Link the developer and maintainer routes and verify**

Run: `npm run docs:check`

Expected: PASS and every command referenced by an npm script exists.

- [ ] **Step 7: Commit**

```bash
git add TESTING_GUIDE.md docs/README.md docs/development integration/README.md
git commit -m "docs: 补全开发构建测试与发布指南"
```

### Task 6: Write the Architecture Guide

**Files:**
- Create: `docs/architecture/overview.md`
- Create: `docs/architecture/download-engine.md`
- Create: `docs/architecture/bittorrent.md`
- Create: `docs/architecture/browser-integration.md`
- Create: `docs/architecture/persistence.md`
- Create: `docs/architecture/security.md`
- Modify: `docs/README.md`

**Interfaces:**
- Consumes: current Rust/TypeScript modules and the design/history documents preserved in Task 2.
- Produces: stable module boundaries and data-flow explanations; project-structure and security documents link here rather than duplicating internals.

- [ ] **Step 1: Write architecture overview**

Include a Mermaid component/data-flow diagram for React → Tauri commands → HTTP/BT engines → storage, plus browser extension → Native Host → local app. Mark trust boundaries and platform adapters.

- [ ] **Step 2: Document HTTP and BitTorrent engines**

Explain protocol detection, dynamic segment state, queue/schedule/rules application, write path, retries, rate limits, magnet placeholders, librqbit sessions, file selection, seeding, proxy privacy, and completion semantics.

- [ ] **Step 3: Document browser integration and persistence**

Explain fixed identities, Native Messaging origin restrictions, local TCP handoff, deep-link fallback, settings/task JSON, backward compatibility, BT fastresume, and recovery expectations.

- [ ] **Step 4: Document the security model**

Cover strict TLS verification, external input validation, safe browser installation guidance, proxy leak prevention, local-only IPC, least-privilege capabilities, file path safety, and limitations.

- [ ] **Step 5: Link architecture routes and verify**

Run: `npm run docs:check`

Expected: PASS; no page depends on exact command/component counts.

- [ ] **Step 6: Commit**

```bash
git add docs/README.md docs/architecture
git commit -m "docs: 补全系统架构与安全模型"
```

### Task 7: Add Reference Pages and Harmonize Cross-Links

**Files:**
- Create: `docs/reference/commands.md`
- Create: `docs/reference/platform-paths.md`
- Create: `docs/reference/known-limitations.md`
- Modify: `README.md`
- Modify: `CONTRIBUTING.md`
- Modify: `SECURITY.md`
- Modify: `TESTING_GUIDE.md`
- Modify: `integration/README.md`
- Modify: `docs/README.md`
- Modify: any new guide whose cross-links need normalization.

**Interfaces:**
- Consumes: all preceding topic pages and Task 1's checker.
- Produces: a complete navigation graph with no orphaned current-state pages and one canonical owner for each repeated fact.

- [ ] **Step 1: Write commands and platform-path references**

Index commands by purpose, show the working directory, and list build/data/log/manifest paths by platform. Mark paths that are discovered dynamically rather than promising a fixed location.

- [ ] **Step 2: Write the known-limitations reference**

Separate unsupported functionality, platform/browser caveats, network-dependent validation, and development-tooling risks. Link to relevant troubleshooting and architecture pages.

- [ ] **Step 3: Normalize navigation and remove duplicate definitions**

Ensure every current-state page is reachable from `docs/README.md`; replace repeated procedures with links to their canonical topic; add “相关文档” sections where useful.

- [ ] **Step 4: Run documentation checks**

Run: `npm run docs:check`

Expected: PASS with no broken relative file/anchor links, missing scripts, or pinned-fact mismatches.

- [ ] **Step 5: Commit**

```bash
git add README.md CONTRIBUTING.md SECURITY.md TESTING_GUIDE.md integration/README.md docs
git commit -m "docs: 添加参考手册并统一文档导航"
```

### Task 8: Full Verification and Documentation Review

**Files:**
- Modify only files required to fix verification findings.

**Interfaces:**
- Consumes: the full documentation set and all existing quality gates.
- Produces: an evidence-backed, review-ready documentation branch.

- [ ] **Step 1: Run the complete automated gate**

Run:

```bash
npm run docs:check
npm run test:run
npm run lint
npm run test:extension
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path integration/native-host/Cargo.toml
cargo clippy --manifest-path integration/native-host/Cargo.toml --all-targets -- -D warnings
```

Expected: all commands pass; Rust app reports 63 passed and one public-DHT test ignored unless the test inventory intentionally changes.

- [ ] **Step 2: Audit canonical facts against code and release**

Check package/app/extension versions, manifests, extension IDs, Node/Rust floors, product names, workflow job count, and the nine v0.3.0 asset names against the current repository and public release.

- [ ] **Step 3: Perform the four reading-route reviews**

Follow documentation as a first-time installer, first-time downloader, new developer, and release maintainer. Record and fix unclear prerequisites, circular navigation, missing expected results, and unsupported claims.

- [ ] **Step 4: Check Git diff and commit final corrections**

Run: `git diff --check && git status --short`

Commit only if verification produced corrections:

```bash
git add README.md CHANGELOG.md CONTRIBUTING.md SECURITY.md TESTING_GUIDE.md integration/README.md docs package.json scripts
git commit -m "docs: 完成文档体系一致性校验"
```

- [ ] **Step 5: Request whole-branch review**

Review the branch against this plan with special attention to unsupported claims, platform-specific instructions, security advice, broken navigation, and duplicated sources of truth.
