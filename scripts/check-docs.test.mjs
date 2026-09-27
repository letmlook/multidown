import { mkdtemp, mkdir, writeFile } from 'node:fs/promises';
import { execFileSync } from 'node:child_process';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, expect, test } from 'vitest';

import { listMarkdownFiles, validateDocumentation } from './check-docs.mjs';

async function fixture(files, scripts = { build: 'vite build', 'test:run': 'vitest run' }) {
  const rootDir = await mkdtemp(path.join(tmpdir(), 'multidown-docs-'));
  for (const [relativePath, contents] of Object.entries(files)) {
    const absolutePath = path.join(rootDir, relativePath);
    await mkdir(path.dirname(absolutePath), { recursive: true });
    await writeFile(absolutePath, contents);
  }
  return {
    rootDir,
    markdownFiles: Object.keys(files).filter((file) => file.endsWith('.md')),
    packageJson: { scripts },
  };
}

describe('validateDocumentation', () => {
  test('accepts Chinese paths, parent paths, valid anchors, external links, and code examples', async () => {
    const input = await fixture({
      'README.md': [
        '# Multidown',
        '',
        '[中文指南](docs/用户指南.md#快速开始)',
        '[带空格的文件](<docs/with space.md>)',
        '[官网](https://example.com)',
        '',
        '```text',
        '[示例文本](missing-example.md)',
        '```',
        '',
        '运行 `npm run build`。',
      ].join('\n'),
      'docs/用户指南.md': '# 用户指南\n\n## 快速开始\n\n[返回](../README.md)\n',
      'docs/with space.md': '# Space\n',
    });

    await expect(validateDocumentation(input)).resolves.toEqual([]);
  });

  test('reports the source file for missing targets and anchors', async () => {
    const input = await fixture({
      'README.md': '# Multidown\n\n[缺失文件](docs/missing.md)\n[缺失章节](docs/guide.md#不存在)\n',
      'docs/guide.md': '# Guide\n\n## Existing\n',
    });

    const errors = await validateDocumentation(input);

    expect(errors).toContain('README.md: link target does not exist: docs/missing.md');
    expect(errors).toContain('README.md: anchor does not exist in docs/guide.md: #不存在');
  });

  test('reports npm commands that are not declared by the project', async () => {
    const input = await fixture({
      'README.md': '# Multidown\n\n```bash\nnpm run docs:check\n```\n',
    });

    await expect(validateDocumentation(input)).resolves.toContain(
      'README.md: npm script does not exist: docs:check',
    );
  });

  test('rejects incorrect explicit browser extension identities', async () => {
    const input = await fixture({
      'docs/browser.md': [
        '# Browser',
        '',
        'Chromium 扩展 ID：`wrong-chromium-id`',
        '',
        'Firefox 扩展 ID：`wrong-firefox-id`',
      ].join('\n'),
    });

    const errors = await validateDocumentation(input);

    expect(errors).toContain(
      'docs/browser.md: Chromium extension ID must be bceackgdejcgphcbhinfgejepgoeiail',
    );
    expect(errors).toContain(
      'docs/browser.md: Firefox extension ID must be multidown@letmlook',
    );
  });
});

test('discovers tracked Markdown files with Chinese names without Git quoting', async () => {
  const rootDir = await mkdtemp(path.join(tmpdir(), 'multidown-docs-git-'));
  await mkdir(path.join(rootDir, 'docs'), { recursive: true });
  await writeFile(path.join(rootDir, 'README.md'), '# Multidown\n');
  await writeFile(path.join(rootDir, 'docs', '用户指南.md'), '# 用户指南\n');
  execFileSync('git', ['init', '-q'], { cwd: rootDir });
  execFileSync('git', ['add', 'README.md', 'docs/用户指南.md'], { cwd: rootDir });

  await expect(listMarkdownFiles(rootDir)).resolves.toEqual([
    'README.md',
    'docs/用户指南.md',
  ]);
});
