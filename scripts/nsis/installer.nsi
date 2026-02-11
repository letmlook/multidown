; 包含必要的 NSIS 插件和库
!include "LogicLib.nsh"

; 定义钩子宏来扩展 Tauri 生成的 NSIS 脚本
!macro NSIS_HOOK_POSTINSTALL
  ; 注册本地主机消息
  Call RegisterNativeHost
  
  ; 安装 CRX 扩展
  Call InstallCrxExtension
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 移除本地主机注册
  Call UnregisterNativeHost
!macroend

; 注册本地主机消息
Function RegisterNativeHost
  ; 获取安装目录
  StrCpy $0 "$INSTDIR\native-host\com.multidown.app.json"
  
  ; 注册 Chrome 本地主机
  WriteRegStr HKLM "Software\Google\Chrome\NativeMessagingHosts\com.multidown.app" "" "$0"
  
  ; 注册 Edge 本地主机
  WriteRegStr HKLM "Software\Microsoft\Edge\NativeMessagingHosts\com.multidown.app" "" "$0"
FunctionEnd

; 卸载本地主机注册
Function UnregisterNativeHost
  ; 移除 Chrome 本地主机注册
  DeleteRegKey HKLM "Software\Google\Chrome\NativeMessagingHosts\com.multidown.app"
  
  ; 移除 Edge 本地主机注册
  DeleteRegKey HKLM "Software\Microsoft\Edge\NativeMessagingHosts\com.multidown.app"
FunctionEnd

; 安装 CRX 扩展
Function InstallCrxExtension
  ; 获取 CRX 文件路径
  StrCpy $0 "$INSTDIR\extension\multidown-extension.crx"
  
  ; 检查 Chrome 是否安装
  StrCpy $1 ""
  ReadRegStr $1 HKLM "Software\Microsoft\Windows\CurrentVersion\App Paths\chrome.exe" ""
  
  ; 如果 Chrome 已安装，尝试安装扩展
  ${If} $1 != ""
    ; 构建命令行
    StrCpy $2 '"$1" --install-extension="$0"'
    
    ; 执行命令
    nsExec::Exec $2
  ${EndIf}
  
  ; 检查 Edge 是否安装
  StrCpy $1 ""
  ReadRegStr $1 HKLM "Software\Microsoft\Windows\CurrentVersion\App Paths\msedge.exe" ""
  
  ; 如果 Edge 已安装，尝试安装扩展
  ${If} $1 != ""
    ; 构建命令行
    StrCpy $2 '"$1" --install-extension="$0"'
    
    ; 执行命令
    nsExec::Exec $2
  ${EndIf}
FunctionEnd