import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';
import { execSync } from 'child_process';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');
const extensionDir = path.resolve(projectRoot, 'integration', 'extension');
const outputDir = path.resolve(projectRoot, 'dist-extension');
const crxPath = path.resolve(outputDir, 'multidown-extension.crx');
const keyPath = path.resolve(outputDir, 'extension.pem');

// 创建输出目录
if (!fs.existsSync(outputDir)) {
  fs.mkdirSync(outputDir, { recursive: true });
}

// 生成私钥文件（如果不存在）
const generateKey = async () => {
  if (!fs.existsSync(keyPath)) {
    console.log('Generating private key for extension signing...');
    const { generateKeyPairSync } = await import('crypto');
    const { privateKey } = generateKeyPairSync('rsa', {
      modulusLength: 2048,
      publicKeyEncoding: {
        type: 'spki',
        format: 'pem'
      },
      privateKeyEncoding: {
        type: 'pkcs8',
        format: 'pem'
      }
    });
    fs.writeFileSync(keyPath, privateKey);
    console.log(`Private key generated: ${keyPath}`);
  }
};

// 获取目录中的所有文件
const getFiles = (dir) => {
  const files = [];
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const fullPath = path.join(dir, entry.name);
    if (entry.isFile()) {
      files.push(fullPath);
    } else if (entry.isDirectory()) {
      files.push(...getFiles(fullPath));
    }
  }
  return files;
};

// 生成签名的CRX扩展
const createCrxFile = async () => {
  console.log('Creating signed CRX extension...');
  
  // 安装必要的依赖
  try {
    await import('crx3');
  } catch (e) {
    console.log('Installing crx3 library...');
    execSync('npm install crx3 --no-save', { stdio: 'inherit' });
  }
  
  // 获取扩展目录中的所有文件
  const files = getFiles(extensionDir);
  console.log(`Found ${files.length} files to package`);
  
  const crx3 = await import('crx3');
  await crx3.default(files, {
    crxPath: crxPath,
    keyPath: keyPath
  });
  console.log(`Signed CRX extension created: ${crxPath}`);
};

// 主函数
const main = async () => {
  try {
    await generateKey();
    await createCrxFile();
    console.log('\n=== Extension Packaging Complete ===');
    console.log('\nSigned CRX extension has been created:');
    console.log(`   ${crxPath}`);
    console.log('\nTo install the CRX extension:');
    console.log('1. Open Chrome/Edge browser');
    console.log('2. Go to chrome://extensions/ or edge://extensions/');
    console.log('3. Drag and drop the CRX file into the extensions page');
    console.log('4. Click "Add extension" when prompted');
  } catch (error) {
    console.error('Error packaging extension:', error);
  }
};

main();
