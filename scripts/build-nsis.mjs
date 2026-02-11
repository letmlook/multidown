#!/usr/bin/env node

import { execSync } from 'child_process';
import { readFileSync, writeFileSync, existsSync } from 'fs';
import { join } from 'path';

// Build main application
console.log('Building main application...');
try {
  execSync('npm run build:all', { stdio: 'inherit' });
} catch (e) {
  console.log('Warning: Frontend build failed, will use existing files');
}

// Check if compiled executable exists
const mainExePath = join('src-tauri', 'target', 'release', 'multidown.exe');
if (!existsSync(mainExePath)) {
  console.error('Error: Main program executable not found');
  console.error('Please build the project successfully before running this script');
  process.exit(1);
}

// Create necessary directory structure
console.log('Creating directory structure...');
try {
  execSync('New-Item -ItemType Directory -Force -Path src-tauri\\target\\release\\bundle\\nsis', { stdio: 'inherit', shell: 'powershell' });
} catch (e) {
  // Directory may already exist, ignore error
}

// Copy necessary files
console.log('Copying necessary files...');
try {
  execSync('copy src-tauri\\target\\release\\multidown.exe src-tauri\\target\\release\\bundle\\nsis\\', { stdio: 'inherit' });
} catch (e) {
  console.log('Warning: Failed to copy main program file, may already exist');
}

// Create NSIS script
const nsisScriptPath = join('src-tauri', 'target', 'release', 'bundle', 'nsis', 'installer.nsi');
console.log(`Creating NSIS script: ${nsisScriptPath}`);

// NSIS script content with English comments only
const nsisContent = 'Unicode true\n' +
'ManifestDPIAware true\n' +
'ManifestDPIAwareness PerMonitorV2\n' +
'\n' +
'SetCompressor /SOLID "lzma"\n' +
'\n' +
'!include MUI2.nsh\n' +
'!include FileFunc.nsh\n' +
'!include x64.nsh\n' +
'!include WordFunc.nsh\n' +
'!include "LogicLib.nsh"\n' +
'\n' +
'!define MANUFACTURER "multidown"\n' +
'!define PRODUCTNAME "MultiDown"\n' +
'!define VERSION "0.1.0"\n' +
'!define VERSIONWITHBUILD "0.1.0.0"\n' +
'!define OUTFILE "MultiDown_0.1.0_x64-setup.exe"\n' +
'!define UNINSTKEY "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\Uninstall\\\\${PRODUCTNAME}"\n' +
'!define MANUKEY "Software\\\\${MANUFACTURER}"\n' +
'!define MANUPRODUCTKEY "${MANUKEY}\\\\${PRODUCTNAME}"\n' +
'\n' +
'Name "${PRODUCTNAME}"\n' +
'OutFile "${OUTFILE}"\n' +
'\n' +
'InstallDir "$LOCALAPPDATA\\\\${PRODUCTNAME}"\n' +
'\n' +
'VIProductVersion "${VERSIONWITHBUILD}"\n' +
'VIAddVersionKey "ProductName" "${PRODUCTNAME}"\n' +
'VIAddVersionKey "FileDescription" "${PRODUCTNAME}"\n' +
'VIAddVersionKey "FileVersion" "${VERSION}"\n' +
'VIAddVersionKey "ProductVersion" "${VERSION}"\n' +
'\n' +
'RequestExecutionLevel user\n' +
'\n' +
'!insertmacro MUI_PAGE_WELCOME\n' +
'!insertmacro MUI_PAGE_DIRECTORY\n' +
'!insertmacro MUI_PAGE_INSTFILES\n' +
'!insertmacro MUI_PAGE_FINISH\n' +
'\n' +
'!insertmacro MUI_UNPAGE_CONFIRM\n' +
'!insertmacro MUI_UNPAGE_INSTFILES\n' +
'\n' +
'!insertmacro MUI_LANGUAGE "English"\n' +
'!insertmacro MUI_LANGUAGE "SimpChinese"\n' +
'\n' +
'Function RegisterNativeHost\n' +
'  StrCpy $0 "$INSTDIR\\\\native-host\\\\com.multidown.app.json"\n' +
'  WriteRegStr HKLM "Software\\\\Google\\\\Chrome\\\\NativeMessagingHosts\\\\com.multidown.app" "" "$0"\n' +
'  WriteRegStr HKLM "Software\\\\Microsoft\\\\Edge\\\\NativeMessagingHosts\\\\com.multidown.app" "" "$0"\n' +
'FunctionEnd\n' +
'\n' +
'Function UnregisterNativeHost\n' +
'  DeleteRegKey HKLM "Software\\\\Google\\\\Chrome\\\\NativeMessagingHosts\\\\com.multidown.app"\n' +
'  DeleteRegKey HKLM "Software\\\\Microsoft\\\\Edge\\\\NativeMessagingHosts\\\\com.multidown.app"\n' +
'FunctionEnd\n' +
'\n' +
'Function InstallCrxExtension\n' +
'  StrCpy $0 "$INSTDIR\\\\extension\\\\multidown-extension.crx"\n' +
'  StrCpy $1 ""\n' +
'  ReadRegStr $1 HKLM "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\App Paths\\\\chrome.exe" ""\n' +
'  ${If} $1 != ""\n' +
'    StrCpy $2 \"\"$1\" --install-extension=\"$0\"\"\n' +
'    nsExec::Exec $2\n' +
'  ${EndIf}\n' +
'  StrCpy $1 ""\n' +
'  ReadRegStr $1 HKLM "Software\\\\Microsoft\\\\Windows\\\\CurrentVersion\\\\App Paths\\\\msedge.exe" ""\n' +
'  ${If} $1 != ""\n' +
'    StrCpy $2 \"\"$1\" --install-extension=\"$0\"\"\n' +
'    nsExec::Exec $2\n' +
'  ${EndIf}\n' +
'FunctionEnd\n' +
'\n' +
'Function .onInit\n' +
'  !insertmacro MUI_LANGDLL_DISPLAY\n' +
'FunctionEnd\n' +
'\n' +
'Section "MainSection"\n' +
'  SetOutPath "$INSTDIR"\n' +
'  File "..\\\\..\\\\multidown.exe"\n' +
'  CreateDirectory "$INSTDIR\\\\extension"\n' +
'  CreateDirectory "$INSTDIR\\\\native-host"\n' +
'  File /r "..\\\\..\\\\..\\\\..\\\\..\\\\dist-extension\\\\multidown-extension.crx"\n' +
'  File /r "..\\\\..\\\\..\\\\..\\\\..\\\\dist-extension\\\\unpacked\\\\*"\n' +
'  File /r "..\\\\..\\\\..\\\\..\\\\..\\\\integration\\\\extension\\\\com.multidown.app.json"\n' +
'  File /r "..\\\\..\\\\..\\\\..\\\\..\\\\integration\\\\native-host\\\\target\\\\release\\\\multidown-native-host.exe"\n' +
'  Rename "$INSTDIR\\\\multidown-extension.crx" "$INSTDIR\\\\extension\\\\multidown-extension.crx"\n' +
'  Rename "$INSTDIR\\\\com.multidown.app.json" "$INSTDIR\\\\native-host\\\\com.multidown.app.json"\n' +
'  Rename "$INSTDIR\\\\multidown-native-host.exe" "$INSTDIR\\\\native-host\\\\multidown-native-host.exe"\n' +
'  Call RegisterNativeHost\n' +
'  Call InstallCrxExtension\n' +
'  CreateShortcut "$DESKTOP\\\\${PRODUCTNAME}.lnk" "$INSTDIR\\\\multidown.exe"\n' +
'  CreateShortcut "$SMPROGRAMS\\\\${PRODUCTNAME}.lnk" "$INSTDIR\\\\multidown.exe"\n' +
'  WriteUninstaller "$INSTDIR\\\\uninstall.exe"\n' +
'  WriteRegStr HKCU "${MANUPRODUCTKEY}" "" "$INSTDIR"\n' +
'  WriteRegStr HKCU "${UNINSTKEY}" "DisplayName" "${PRODUCTNAME}"\n' +
'  WriteRegStr HKCU "${UNINSTKEY}" "DisplayIcon" \"$INSTDIR\\\\multidown.exe\"\n' +
'  WriteRegStr HKCU "${UNINSTKEY}" "DisplayVersion" "${VERSION}"\n' +
'  WriteRegStr HKCU "${UNINSTKEY}" "Publisher" "${MANUFACTURER}"\n' +
'  WriteRegStr HKCU "${UNINSTKEY}" "InstallLocation" "$INSTDIR"\n' +
'  WriteRegStr HKCU "${UNINSTKEY}" "UninstallString" \"$INSTDIR\\\\uninstall.exe\"\n' +
'  WriteRegDWORD HKCU "${UNINSTKEY}" "NoModify" 1\n' +
'  WriteRegDWORD HKCU "${UNINSTKEY}" "NoRepair" 1\n' +
'SectionEnd\n' +
'\n' +
'Section "Uninstall"\n' +
'  DeleteRegKey HKLM "Software\\\\Google\\\\Chrome\\\\NativeMessagingHosts\\\\com.multidown.app"\n' +
'  DeleteRegKey HKLM "Software\\\\Microsoft\\\\Edge\\\\NativeMessagingHosts\\\\com.multidown.app"\n' +
'  Delete "$DESKTOP\\\\${PRODUCTNAME}.lnk"\n' +
'  Delete "$SMPROGRAMS\\\\${PRODUCTNAME}.lnk"\n' +
'  Delete "$INSTDIR\\\\multidown.exe"\n' +
'  Delete "$INSTDIR\\\\uninstall.exe"\n' +
'  Delete "$INSTDIR\\\\extension\\\\multidown-extension.crx"\n' +
'  Delete "$INSTDIR\\\\native-host\\\\com.multidown.app.json"\n' +
'  Delete "$INSTDIR\\\\native-host\\\\multidown-native-host.exe"\n' +
'  RMDir /REBOOTOK "$INSTDIR\\\\extension\\\\unpacked"\n' +
'  RMDir /REBOOTOK "$INSTDIR\\\\extension"\n' +
'  RMDir /REBOOTOK "$INSTDIR\\\\native-host"\n' +
'  RMDir /REBOOTOK "$INSTDIR"\n' +
'  DeleteRegKey HKCU "${UNINSTKEY}"\n' +
'  DeleteRegKey /ifempty HKCU "${MANUPRODUCTKEY}"\n' +
'  DeleteRegKey /ifempty HKCU "${MANUKEY}"\n' +
'SectionEnd\n';

// Write NSIS script
writeFileSync(nsisScriptPath, nsisContent, 'utf8');
console.log('NSIS script created successfully');

// Find makensis.exe
let makensisPath = null;
try {
  // Try to find in Tauri directory
  const tauriNsisPath = join(process.env.LOCALAPPDATA || '', 'tauri', 'NSIS', 'makensis.exe');
  if (existsSync(tauriNsisPath)) {
    makensisPath = tauriNsisPath;
  }
} catch (e) {
  console.log('Tauri NSIS not found, using system installed version');
}

if (!makensisPath) {
  try {
    // Try to find in system path
    execSync('makensis /VERSION', { stdio: 'inherit' });
    makensisPath = 'makensis';
  } catch (e) {
    console.error('Error: makensis.exe not found');
    console.error('Please ensure NSIS is installed and added to system path');
    process.exit(1);
  }
}

// Compile NSIS script
const nsisDir = join('src-tauri', 'target', 'release', 'bundle', 'nsis');
console.log(`Compiling NSIS script: ${makensisPath}`);
try {
  execSync(`${makensisPath} installer.nsi`, { stdio: 'inherit', cwd: nsisDir });
  console.log('NSIS script compiled successfully');
  
  const targetExe = join(nsisDir, 'MultiDown_0.1.0_x64-setup.exe');
  console.log(`Installer generated: ${targetExe}`);
} catch (error) {
  console.error('NSIS compilation failed:', error.message);
  process.exit(1);
}

console.log('\n=== NSIS Build Complete ===');
console.log('Installer generated:');
console.log('  src-tauri/target/release/bundle/nsis/MultiDown_0.1.0_x64-setup.exe');
console.log('\nInstaller features:');
console.log('  1. Install MultiDown main program');
console.log('  2. Register native host for Chrome and Edge browsers');
console.log('  3. Automatically install signed CRX extension');
console.log('  4. Create desktop and start menu shortcuts');
console.log('  5. Support English and Chinese interface');
