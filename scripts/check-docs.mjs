import { execFileSync } from 'node:child_process';
import crypto from 'node:crypto';
import { access, readFile } from 'node:fs/promises';
import path from 'node:path';
import { pathToFileURL } from 'node:url';

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
    .replace(/\[([^\]]+)\]\([^)]+\)/gu, '$1')
    .replace(/[`*_~]/gu, '')
    .replace(/[^\p{L}\p{N}\s_-]/gu, '')
    .replace(/\s/gu, '-');
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

function isCurrentStatePath(relativePath) {
  return [
    'README.md',
    'CONTRIBUTING.md',
    'SECURITY.md',
    'TESTING_GUIDE.md',
    'integration/README.md',
    'docs/README.md',
  ].includes(relativePath)
    || /^docs\/(?:user-guide|development|architecture|reference)\//u.test(relativePath);
}

function explicitFactErrors(relativePath, markdown, facts) {
  const errors = [];
  const chromium = markdown.match(/Chromium\s*扩展\s*ID\s*[：:]\s*`([^`]+)`/iu)?.[1];
  if (chromium && chromium !== facts.chromiumExtensionId) {
    errors.push(
      `${relativePath}: Chromium extension ID must be ${facts.chromiumExtensionId}`,
    );
  }
  const firefox = markdown.match(/Firefox\s*扩展\s*ID\s*[：:]\s*`([^`]+)`/iu)?.[1];
  if (firefox && firefox !== facts.firefoxExtensionId) {
    errors.push(`${relativePath}: Firefox extension ID must be ${facts.firefoxExtensionId}`);
  }
  if (isCurrentStatePath(relativePath)) {
    for (const match of markdown.matchAll(/\bv\d+\.\d+\.\d+\b/gu)) {
      if (match[0] !== facts.version) {
        errors.push(
          `${relativePath}: current documentation version must be ${facts.version} (found ${match[0]})`,
        );
      }
    }
  }
  if (relativePath === 'README.md' && !markdown.startsWith(`# ${facts.projectName}\n`)) {
    errors.push(`README.md: first heading must be # ${facts.projectName}`);
  }
  if (relativePath === 'docs/README.md' && !markdown.startsWith(`# ${facts.projectName} 文档中心\n`)) {
    errors.push(`docs/README.md: first heading must be # ${facts.projectName} 文档中心`);
  }
  if (relativePath === 'docs/development/release.md') {
    for (const asset of facts.releaseAssets) {
      if (!markdown.includes(`\`${asset}\``)) {
        errors.push(`${relativePath}: missing ${facts.version} release asset: ${asset}`);
      }
    }
  }
  return errors;
}

export async function validateDocumentation({ rootDir, markdownFiles, packageJson, canonicalFacts }) {
  const errors = [];
  const scripts = packageJson.scripts ?? {};

  for (const relativePath of markdownFiles) {
    const absolutePath = path.resolve(rootDir, relativePath);
    const markdown = await readFile(absolutePath, 'utf8');

    errors.push(...explicitFactErrors(relativePath, markdown, canonicalFacts));

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

function chromiumIdFromKey(key) {
  const digest = crypto
    .createHash('sha256')
    .update(Buffer.from(key, 'base64'))
    .digest()
    .subarray(0, 16);
  return [...digest]
    .flatMap((byte) => [byte >> 4, byte & 15])
    .map((nibble) => 'abcdefghijklmnop'[nibble])
    .join('');
}

function releaseAssets(productName, version) {
  const number = version.replace(/^v/u, '');
  return [
    `${productName}-${number}-1.x86_64.rpm`,
    `${productName}_${number}_aarch64.dmg`,
    `${productName}_${number}_amd64.AppImage`,
    `${productName}_${number}_amd64.deb`,
    `${productName}_${number}_x64-setup.exe`,
    `${productName}_${number}_x64.dmg`,
    `${productName}_${number}_x64_en-US.msi`,
    `${productName}_aarch64.app.tar.gz`,
    `${productName}_x64.app.tar.gz`,
  ];
}

function cargoPackageVersion(toml) {
  return toml.match(/^version\s*=\s*"([^"]+)"/mu)?.[1];
}

export async function readCanonicalFacts(rootDir, packageJson) {
  const [tauriConfig, extensionManifest, nativeHostManifest, buildExtension, appCargo, hostCargo] =
    await Promise.all([
      readFile(path.join(rootDir, 'src-tauri/tauri.conf.json'), 'utf8').then(JSON.parse),
      readFile(path.join(rootDir, 'integration/extension/manifest.json'), 'utf8').then(JSON.parse),
      readFile(path.join(rootDir, 'integration/extension/com.multidown.app.json'), 'utf8').then(JSON.parse),
      readFile(path.join(rootDir, 'scripts/build-extension.mjs'), 'utf8'),
      readFile(path.join(rootDir, 'src-tauri/Cargo.toml'), 'utf8'),
      readFile(path.join(rootDir, 'integration/native-host/Cargo.toml'), 'utf8'),
    ]);
  const version = packageJson.version;
  const sourceVersions = {
    'src-tauri/tauri.conf.json': tauriConfig.version,
    'integration/extension/manifest.json': extensionManifest.version,
    'src-tauri/Cargo.toml': cargoPackageVersion(appCargo),
    'integration/native-host/Cargo.toml': cargoPackageVersion(hostCargo),
  };
  const errors = Object.entries(sourceVersions)
    .filter(([, sourceVersion]) => sourceVersion !== version)
    .map(([file, sourceVersion]) => `${file}: version ${sourceVersion} does not match package.json ${version}`);
  const chromiumExtensionId = chromiumIdFromKey(extensionManifest.key);
  const allowedOrigin = `chrome-extension://${chromiumExtensionId}/`;
  if (JSON.stringify(nativeHostManifest.allowed_origins) !== JSON.stringify([allowedOrigin])) {
    errors.push(`integration/extension/com.multidown.app.json: allowed_origins must be ${allowedOrigin}`);
  }
  const firefoxExtensionId = buildExtension.match(/\bid\s*:\s*['"]([^'"]+)['"]/u)?.[1];
  if (!firefoxExtensionId) {
    errors.push('scripts/build-extension.mjs: Firefox extension ID not found');
  }
  const projectName = packageJson.name.charAt(0).toUpperCase() + packageJson.name.slice(1);
  const productName = tauriConfig.productName;
  return {
    facts: {
      projectName,
      productName,
      version: `v${version}`,
      chromiumExtensionId,
      firefoxExtensionId,
      releaseAssets: releaseAssets(productName, `v${version}`),
    },
    errors,
  };
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
  const canonical = await readCanonicalFacts(rootDir, packageJson);
  const errors = [
    ...canonical.errors,
    ...(await validateDocumentation({
      rootDir,
      markdownFiles,
      packageJson,
      canonicalFacts: canonical.facts,
    })),
  ];
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
