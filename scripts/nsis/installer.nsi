; 包含必要的 NSIS 插件和库
!include "LogicLib.nsh"

; 定义钩子宏来扩展 Tauri 生成的 NSIS 脚本
!macro NSIS_HOOK_POSTINSTALL
  ; 注册本地主机消息
  Call RegisterNativeHost
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
