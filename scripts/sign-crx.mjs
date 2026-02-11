import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, '..');
const outputDir = path.resolve(projectRoot, 'dist-extension');
const unpackedDir = path.resolve(outputDir, 'unpacked');

// 主函数
const main = async () => {
  try {
    // 检查解压后的扩展目录
    if (!fs.existsSync(unpackedDir)) {
      console.error('Error: Unpacked extension directory not found.');
      console.log('Please run "npm run build:extension" first.');
      process.exit(1);
    }
    
    console.log('=== CRX Signing Guide ===\n');
    console.log('Chrome browser can sign extensions directly. Follow these steps:\n');
    console.log('1. Open Chrome browser');
    console.log('2. Go to chrome://extensions/');
    console.log('3. Enable "Developer mode" (top right)');
    console.log('4. Click "Load unpacked" and select:');
    console.log(`   ${unpackedDir}`);
    console.log('5. The extension will be installed in developer mode');
    console.log('6. Click "Pack extension" (top left)');
    console.log('7. Select the unpacked directory:');
    console.log(`   ${unpackedDir}`);
    console.log('8. Leave "Private key file" empty (Chrome will generate one)');
    console.log('9. Click "Pack extension"');
    console.log('10. Chrome will generate:');
    console.log('    - A signed CRX file');
    console.log('    - A PEM private key file');
    console.log('\n=== Alternative: Use Developer Mode (Recommended) ===\n');
    console.log('For development and testing, using developer mode is recommended:');
    console.log('1. Follow steps 1-4 above');
    console.log('2. The extension will work without signing');
    console.log('3. You can make changes and reload the extension');
    console.log('\n=== Important Notes ===\n');
    console.log('- CRX files require proper signing to be installed in normal mode');
    console.log('- Self-signed extensions may show warnings');
    console.log('- For production, consider publishing to Chrome Web Store');
    console.log('\n=== Files Ready ===\n');
    console.log('Unpacked extension directory:', unpackedDir);
    console.log('You can now use this directory with Chrome\'s extension management.');
    
  } catch (error) {
    console.error('Error:', error);
    process.exit(1);
  }
};

main();

