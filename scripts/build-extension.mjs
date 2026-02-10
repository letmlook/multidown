import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { createWriteStream } from 'fs';
import { pipeline } from 'stream';
import { promisify } from 'util';
import { createGzip } from 'zlib';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');
const extensionDir = path.resolve(projectRoot, 'integration', 'extension');
const outputDir = path.resolve(projectRoot, 'dist-extension');
const crxPath = path.resolve(outputDir, 'multidown-extension.crx');

// 创建输出目录
if (!fs.existsSync(outputDir)) {
  fs.mkdirSync(outputDir, { recursive: true });
}

// 创建一个zip文件（模拟crx文件）
const createCrxFile = async () => {
  const zipPath = path.resolve(outputDir, 'extension.zip');
  
  // 使用Node.js内置模块创建zip文件
  // 注意：这是一个简化的实现，实际项目中可以使用更完善的zip库
  const archiver = (await import('archiver')).default;
  const output = createWriteStream(zipPath);
  const archive = archiver('zip', { zlib: { level: 9 } });
  
  const pipelinePromise = promisify(pipeline);
  
  output.on('close', () => {
    console.log(`Zip file created: ${archive.pointer()} total bytes`);
    
    // 将zip文件重命名为crx文件
    fs.renameSync(zipPath, crxPath);
    console.log(`CRX file created: ${crxPath}`);
  });
  
  archive.on('error', (err) => {
    throw err;
  });
  
  archive.pipe(output);
  
  // 添加扩展文件到zip
  const files = fs.readdirSync(extensionDir);
  files.forEach(file => {
    const filePath = path.join(extensionDir, file);
    const stats = fs.statSync(filePath);
    if (stats.isFile()) {
      archive.file(filePath, { name: file });
    }
  });
  
  await archive.finalize();
};

// 安装archiver库（如果不存在）
const installArchiver = async () => {
  const { execSync } = await import('child_process');
  try {
    // 尝试导入archiver
    await import('archiver');
  } catch (e) {
    console.log('Installing archiver library...');
    execSync('npm install archiver --no-save', { stdio: 'inherit' });
  }
};

// 主函数
const main = async () => {
  try {
    await installArchiver();
    await createCrxFile();
    console.log('Extension packaged as CRX file');
    console.log('Note: This is an unsigned CRX file, which can be loaded in developer mode.');
  } catch (error) {
    console.error('Error packaging extension:', error);
  }
};

main();
