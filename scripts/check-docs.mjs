import { execFileSync } from 'node:child_process';
import { access, readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

const CHROMIUM_EXTENSION_ID = 'bceackgdejcgphcbhinfgejepgoeiail';
const FIREFOX_EXTENSION_ID = 'multidown@letmlook';
const CURRENT_VERSION = 'v0.3.0';
const RELEASE_ASSETS = [
  'MultiDown-0.3.0-1.x86_64.rpm',
  'MultiDown_0.3.0_aarch64.dmg',
  'MultiDown_0.3.0_amd64.AppImage',
  'MultiDown_0.3.0_amd64.deb',
  'MultiDown_0.3.0_x64-setup.exe',
  'MultiDown_0.3.0_x64.dmg',
  'MultiDown_0.3.0_x64_en-US.msi',
  'MultiDown_aarch64.app.tar.gz',
  'MultiDown_x64.app.tar.gz',
];

function withoutFencedCode(markdown) {
  let fence = null;
  return markdown
    .split('\n')
    .filter((line) => {
      const marker = line.match(/^\s*(`{3,}|~{3,})/u)?.[1];
      if (marker && fence === null) {
        fence = marker[0];
        return false;
      }
      if (marker && fence === marker[0]) {
        fence = null;
        return false;
      }
      return fence === null;
    })
    .join('\n');
}

function githubSlug(text) {
  return text
    .toLowerCase()
    .trim()
    .replace(/<[^>]+>/gu, '')
    .replace(/[\p{P}\p{S}]/gu, '')
    .replace(/\s+/gu, '-');
}

function collectAnchors(markdown) {
  const anchors = new Set();
  const counts = new Map();
  for (const line of withoutFencedCode(markdown).split('\n')) {
    const match = line.match(/^#{1,6}\s+(.+?)\s*#*$/u);
    if (!match) continue;
    const base = githubSlug(match[1]);
    const count = counts.get(base) ?? 0;
    counts.set(base, count + 1);
    anchors.add(count === 0 ? base : `${base}-${count}`);
  }
  return anchors;
}

function markdownDestinations(markdown) {
  const destinations = [];
  const source = withoutFencedCode(markdown);
  const linkPattern = /!?\[[^\]]*\]\((<[^>]+>|[^)\s]+)(?:\s+['"][^'"]*['"])?\)/gu;
  for (const match of source.matchAll(linkPattern)) {
    const raw = match[1];
    destinations.push(raw.startsWith('<') ? raw.slice(1, -1) : raw);
  }
  return destinations;
}

async function exists(filePath) {
  try {
    await access(filePath);
    return true;
  } catch {
    return false;
  }
}

function safeDecode(value) {
  try {
    return decodeURIComponent(value);
  } catch {
    return value;
  }
}

function explicitFactErrors(relativePath, markdown) {
  const errors = [];
  const chromium = markdown.match(/Chromium\s*扩展\s*ID\s*[：:]\s*`([^`]+)`/iu)?.[1];
  if (chromium && chromium !== CHROMIUM_EXTENSION_ID) {
    errors.push(
      `${relativePath}: Chromium extension ID must be ${CHROMIUM_EXTENSION_ID}`,
    );
  }
  const firefox = markdown.match(/Firefox\s*扩展\s*ID\s*[：:]\s*`([^`]+)`/iu)?.[1];
  if (firefox && firefox !== FIREFOX_EXTENSION_ID) {
    errors.push(`${relativePath}: Firefox extension ID must be ${FIREFOX_EXTENSION_ID}`);
  }
  const version = markdown.match(/(?:当前稳定版本|当前版本|稳定版本)\s*[：:]\s*`?(v\d+\.\d+\.\d+)`?/iu)?.[1];
  if (version && version !== CURRENT_VERSION) {
    errors.push(`${relativePath}: current stable version must be ${CURRENT_VERSION}`);
  }
  if (relativePath === 'README.md' && !markdown.startsWith('# Multidown\n')) {
    errors.push('README.md: first heading must be # Multidown');
  }
  if (relativePath === 'docs/development/release.md') {
    for (const asset of RELEASE_ASSETS) {
      if (!markdown.includes(`\`${asset}\``)) {
        errors.push(`${relativePath}: missing v0.3.0 release asset: ${asset}`);
      }
    }
  }
  return errors;
}

export async function validateDocumentation({ rootDir, markdownFiles, packageJson }) {
  const errors = [];
  const scripts = packageJson.scripts ?? {};

  for (const relativePath of markdownFiles) {
    const absolutePath = path.resolve(rootDir, relativePath);
    const markdown = await readFile(absolutePath, 'utf8');

    errors.push(...explicitFactErrors(relativePath, markdown));

    for (const match of markdown.matchAll(/\bnpm\s+run\s+([A-Za-z0-9:_-]+)/gu)) {
      const script = match[1];
      if (!(script in scripts)) {
        errors.push(`${relativePath}: npm script does not exist: ${script}`);
      }
    }

    for (const destination of markdownDestinations(markdown)) {
      if (/^(?:[a-z][a-z0-9+.-]*:|\/\/)/iu.test(destination)) continue;

      const [rawTarget, rawAnchor = ''] = destination.split('#', 2);
      const target = safeDecode(rawTarget);
      const targetPath = target
        ? path.resolve(path.dirname(absolutePath), target)
        : absolutePath;
      const displayTarget = target || relativePath;

      if (!(await exists(targetPath))) {
        errors.push(`${relativePath}: link target does not exist: ${target}`);
        continue;
      }

      if (rawAnchor && path.extname(targetPath).toLowerCase() === '.md') {
        const targetMarkdown = await readFile(targetPath, 'utf8');
        const anchor = safeDecode(rawAnchor).toLowerCase();
        if (!collectAnchors(targetMarkdown).has(anchor)) {
          errors.push(`${relativePath}: anchor does not exist in ${displayTarget}: #${safeDecode(rawAnchor)}`);
        }
      }
    }
  }

  return [...new Set(errors)].sort();
}

export async function listMarkdownFiles(rootDir) {
  return execFileSync(
    'git',
    ['-c', 'core.quotepath=false', 'ls-files', '-co', '-z', '--exclude-standard', '--', '*.md'],
    { cwd: rootDir },
  )
    .toString('utf8')
    .split('\0')
    .filter(Boolean);
}

async function main() {
  const rootDir = process.cwd();
  const markdownFiles = await listMarkdownFiles(rootDir);
  const packageJson = JSON.parse(await readFile(path.join(rootDir, 'package.json'), 'utf8'));
  const errors = await validateDocumentation({ rootDir, markdownFiles, packageJson });
  if (errors.length > 0) {
    for (const error of errors) console.error(`- ${error}`);
    process.exitCode = 1;
    return;
  }
  console.log(`Documentation check passed (${markdownFiles.length} Markdown files).`);
}

if (process.argv[1] && pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url) {
  await main();
}
