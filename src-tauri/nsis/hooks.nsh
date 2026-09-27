; MultiDown NSIS 安装钩子
;
; 设计原则（docs/磁力链接与种子下载实施方案.md §4.9）：把 MultiDown 注册为磁力链接的
; **候选应用**（RegisteredApplications + Capabilities + 独立 ProgID），绝不静默抢占
; 其它客户端（如 qBittorrent）的 `magnet` 默认关联——是否成为默认由用户在
; 「系统设置 > 应用 > 默认应用」里选择。卸载时只清理属于本程序的键名。

!macro NSIS_HOOK_POSTINSTALL
  ; ProgID：磁力链接的打开方式（键名独属于本程序，可安全写入/删除）
  WriteRegStr SHCTX "Software\Classes\MultiDown.magnet" "" "MultiDown 磁力链接"
  WriteRegStr SHCTX "Software\Classes\MultiDown.magnet" "URL Protocol" ""
  WriteRegStr SHCTX "Software\Classes\MultiDown.magnet\DefaultIcon" "" '"$INSTDIR\MultiDown.exe",0'
  WriteRegStr SHCTX "Software\Classes\MultiDown.magnet\shell\open\command" "" '"$INSTDIR\MultiDown.exe" "%1"'

  ; ProgID：.torrent 文件的打开方式
  WriteRegStr SHCTX "Software\Classes\MultiDown.torrent" "" "BitTorrent 种子文件"
  WriteRegStr SHCTX "Software\Classes\MultiDown.torrent" "Content Type" "application/x-bittorrent"
  WriteRegStr SHCTX "Software\Classes\MultiDown.torrent\DefaultIcon" "" '"$INSTDIR\MultiDown.exe",0'
  WriteRegStr SHCTX "Software\Classes\MultiDown.torrent\shell\open\command" "" '"$INSTDIR\MultiDown.exe" "%1"'

  ; 候选应用声明（"默认应用"设置页据此列出 MultiDown）
  WriteRegStr SHCTX "Software\com.multidown.app\Capabilities" "ApplicationName" "MultiDown"
  WriteRegStr SHCTX "Software\com.multidown.app\Capabilities" "ApplicationDescription" "MultiDown 跨平台多线程下载工具（支持磁力链接与种子文件）"
  WriteRegStr SHCTX "Software\com.multidown.app\Capabilities\URLAssociations" "magnet" "MultiDown.magnet"
  WriteRegStr SHCTX "Software\com.multidown.app\Capabilities\FileAssociations" ".torrent" "MultiDown.torrent"

  ; 登记为已注册应用
  WriteRegStr SHCTX "Software\RegisteredApplications" "MultiDown" "Software\com.multidown.app\Capabilities"

  ; 通知 Shell 刷新文件关联缓存
  System::Call 'Shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 只删除独属于本程序的键（MultiDown.magnet / com.multidown.app），不触碰共享的
  ; magnet 键或其它应用的登记，保证卸载后系统里不留残渣、也不破坏既有关联
  DeleteRegKey SHCTX "Software\Classes\MultiDown.magnet"
  DeleteRegKey SHCTX "Software\Classes\MultiDown.torrent"
  DeleteRegKey /ifempty SHCTX "Software\com.multidown.app\Capabilities\FileAssociations"
  DeleteRegKey /ifempty SHCTX "Software\com.multidown.app\Capabilities\URLAssociations"
  DeleteRegKey /ifempty SHCTX "Software\com.multidown.app\Capabilities"
  DeleteRegKey /ifempty SHCTX "Software\com.multidown.app"

  ; RegisteredApplications 是共享键，只删仍指向本程序 Capabilities 的值
  ReadRegStr $0 SHCTX "Software\RegisteredApplications" "MultiDown"
  StrCmp $0 "Software\com.multidown.app\Capabilities" 0 +2
    DeleteRegValue SHCTX "Software\RegisteredApplications" "MultiDown"

  System::Call 'Shell32::SHChangeNotify(i 0x08000000, i 0, p 0, p 0)'
!macroend
