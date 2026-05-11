import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";
import type {
  AppSettings,
  QueueSummary,
  BatchJobInfo,
  ScheduleRule,
  ScheduleState,
  ScheduleType,
  Recurrence,
  ProxyConfig,
  ProxyRule,
  ProxyTestResult,
  CategoryRule,
  CategoryMatchType,
} from "../types/download";

interface OptionsModalProps {
  open: boolean;
  onClose: () => void;
}

const TABS = [
  { id: "general", label: "常规" },
  { id: "download", label: "下载" },
  { id: "save", label: "保存至" },
  { id: "connection", label: "连接" },
  { id: "proxy", label: "代理服务器" },
  { id: "sounds", label: "通知与声音" },
  { id: "categories", label: "分类规则" },
  { id: "queues", label: "队列管理" },
  { id: "batches", label: "批次管理" },
  { id: "rules", label: "分类规则2" },
  { id: "schedule", label: "定时调度" },
  { id: "proxies", label: "代理管理" },
];

const defaultSettings: AppSettings = {
  default_save_path: "",
  max_connections_per_task: 8,
  max_concurrent_tasks: 4,
  run_at_startup: false,
  clipboard_monitor: false,
  show_start_dialog: true,
  show_complete_dialog: true,
  duplicate_action: "ask",
  user_agent: "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
  use_last_save_path: true,
  proxy_type: "none",
  proxy_host: "",
  proxy_port: 8080,
  notification_on_complete: true,
  notification_on_fail: true,
  timeout_secs: 30,
  save_progress_interval_secs: 30,
};

function newRule(): CategoryRule {
  return {
    id: "",
    name: "",
    match_type: "extension",
    patterns: [],
    save_path: "",
    enabled: true,
    priority: 0,
  };
}

// ──────────────────────────────────────────────────────────────────────────────
// Schedule form helpers
// ──────────────────────────────────────────────────────────────────────────────

type ScheduleFormRecurrence =
  | { type: "once"; date: string }
  | { type: "daily" }
  | { type: "weekdays" }
  | { type: "weekends" }
  | { type: "weekly"; days: string[] };

interface ScheduleFormData {
  name: string;
  schedule_type: ScheduleType;
  recurrence: ScheduleFormRecurrence;
  start_time: string;
  end_time: string;
  speed_limit_kbps: string;
}

function scheduleRuleToForm(rule: ScheduleRule): ScheduleFormData {
  const rec: ScheduleFormRecurrence =
    rule.recurrence.type === "once"
      ? { type: "once", date: rule.recurrence.date ?? "" }
      : rule.recurrence.type === "daily"
      ? { type: "daily" }
      : rule.recurrence.type === "weekdays"
      ? { type: "weekdays" }
      : rule.recurrence.type === "weekends"
      ? { type: "weekends" }
      : { type: "weekly", days: rule.recurrence.days ?? [] };

  return {
    name: rule.name,
    schedule_type: rule.schedule_type,
    recurrence: rec,
    start_time: rule.start_time,
    end_time: rule.end_time ?? "",
    speed_limit_kbps: rule.speed_limit_kbps?.toString() ?? "",
  };
}

function scheduleFormToRecurrence(rec: ScheduleFormRecurrence): Recurrence {
  if (rec.type === "once") return { type: "once", date: rec.date || undefined };
  if (rec.type === "daily") return { type: "daily" };
  if (rec.type === "weekdays") return { type: "weekdays" };
  if (rec.type === "weekends") return { type: "weekends" };
  return { type: "weekly", days: rec.days };
}

// ──────────────────────────────────────────────────────────────────────────────
// Main Modal
// ──────────────────────────────────────────────────────────────────────────────

export function OptionsModal({ open, onClose }: OptionsModalProps) {
  const [tab, setTab] = useState("general");
  const [settings, setSettings] = useState<AppSettings>(defaultSettings);
  const [loading, setLoading] = useState(false);
  const [saving, setSaving] = useState(false);

  // ── Category rules state ──────────────────────────────────────────────────
  const [rules, setRules] = useState<CategoryRule[]>([]);
  const [rulesLoading, setRulesLoading] = useState(false);
  const [editingRule, setEditingRule] = useState<CategoryRule | null>(null);
  const [showRuleModal, setShowRuleModal] = useState(false);
  const [draggedId, setDraggedId] = useState<string | null>(null);

  // ── Queue state ───────────────────────────────────────────────────────────
  const [queues, setQueues] = useState<QueueSummary[]>([]);
  const [queuesLoading, setQueuesLoading] = useState(false);
  const [selectedQueue, setSelectedQueue] = useState<QueueSummary | null>(null);
  const [queueForm, setQueueForm] = useState({ name: "", max_concurrent: 3, priority: 0 });

  // ── Batch state ────────────────────────────────────────────────────────────
  const [batches, setBatches] = useState<BatchJobInfo[]>([]);
  const [batchesLoading, setBatchesLoading] = useState(false);
  const [showBatchModal, setShowBatchModal] = useState(false);
  const [batchForm, setBatchForm] = useState({ name: "", urls: "" });

  // ── Schedule state ───────────────────────────────────────────────────────
  const [scheduleState, setScheduleState] = useState<ScheduleState>({ enabled: false });
  const [scheduleTasks, setScheduleTasks] = useState<ScheduleRule[]>([]);
  const [scheduleLoading, setScheduleLoading] = useState(false);
  const [showScheduleModal, setShowScheduleModal] = useState(false);
  const [editingSchedule, setEditingSchedule] = useState<ScheduleRule | null>(null);
  const [scheduleForm, setScheduleForm] = useState<ScheduleFormData>({
    name: "",
    schedule_type: "start_download",
    recurrence: { type: "daily" },
    start_time: "08:00",
    end_time: "",
    speed_limit_kbps: "",
  });

  // ── Proxy management state ───────────────────────────────────────────────
  const [proxyList, setProxyList] = useState<ProxyConfig[]>([]);
  const [_proxyRules, setProxyRules] = useState<ProxyRule[]>([]);
  const [proxyLoading, setProxyLoading] = useState(false);
  const [selectedProxy, setSelectedProxy] = useState<ProxyConfig | null>(null);
  const [showProxyModal, setShowProxyModal] = useState(false);
  const [proxyForm, setProxyForm] = useState({
    name: "",
    proxy_type: "http" as "http" | "socks5" | "https",
    host: "",
    port: 8080,
    username: "",
    password: "",
  });
  const [testingProxyId, setTestingProxyId] = useState<string | null>(null);

  useEffect(() => {
    if (open) {
      setLoading(true);
      invoke<AppSettings>("get_settings")
        .then((s) => setSettings({ ...defaultSettings, ...s }))
        .catch(() => setSettings(defaultSettings))
        .finally(() => setLoading(false));
    }
  }, [open]);

  useEffect(() => {
    if (!open) return;
    if (tab === "categories") loadRules();
    if (tab === "queues") loadQueues();
    if (tab === "batches") loadBatches();
    if (tab === "schedule") loadSchedule();
    if (tab === "proxies") loadProxies();
  }, [open, tab]);

  // ── Category Rules ────────────────────────────────────────────────────────

  const loadRules = async () => {
    setRulesLoading(true);
    try {
      const r = await invoke<CategoryRule[]>("list_rules");
      setRules(r);
    } catch (e) {
      console.error(e);
    } finally {
      setRulesLoading(false);
    }
  };

  const update = (patch: Partial<AppSettings>) => {
    setSettings((prev) => ({ ...prev, ...patch }));
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await invoke("set_settings", { settings });
      onClose();
    } catch (e) {
      console.error(e);
    } finally {
      setSaving(false);
    }
  };

  const handleAddRule = () => {
    setEditingRule(newRule());
    setShowRuleModal(true);
  };

  const handleEditRule = (rule: CategoryRule) => {
    setEditingRule({ ...rule });
    setShowRuleModal(true);
  };

  const handleDeleteRule = async (ruleId: string) => {
    try {
      await invoke("delete_rule", { ruleId });
      setRules((prev) => prev.filter((r) => r.id !== ruleId));
    } catch (e) {
      console.error(e);
    }
  };

  const handleSaveRule = async () => {
    if (!editingRule) return;
    try {
      if (editingRule.id) {
        await invoke("update_rule", { rule: editingRule });
        setRules((prev) =>
          prev.map((r) => (r.id === editingRule.id ? editingRule : r))
        );
      } else {
        const id = await invoke<string>("create_rule", { rule: editingRule });
        const added = { ...editingRule, id };
        setRules((prev) => [...prev, added]);
      }
      setShowRuleModal(false);
      setEditingRule(null);
    } catch (e) {
      console.error(e);
    }
  };

  const handleToggleRule = async (rule: CategoryRule) => {
    const updated = { ...rule, enabled: !rule.enabled };
    try {
      await invoke("update_rule", { rule: updated });
      setRules((prev) => prev.map((r) => (r.id === rule.id ? updated : r)));
    } catch (e) {
      console.error(e);
    }
  };

  const handleDragStart = (id: string) => setDraggedId(id);

  const handleDragOver = (e: React.DragEvent, _id: string) => {
    e.preventDefault();
  };

  const handleDrop = async (_e: React.DragEvent, targetId: string) => {
    if (!draggedId || draggedId === targetId) {
      setDraggedId(null);
      return;
    }
    const draggedIdx = rules.findIndex((r) => r.id === draggedId);
    const targetIdx = rules.findIndex((r) => r.id === targetId);
    if (draggedIdx < 0 || targetIdx < 0) {
      setDraggedId(null);
      return;
    }
    const newRules = [...rules];
    const [removed] = newRules.splice(draggedIdx, 1);
    newRules.splice(targetIdx, 0, removed);
    const reordered = newRules.map((r, i) => ({ ...r, priority: i }));
    setRules(reordered);
    setDraggedId(null);
    try {
      await invoke("reorder_rules", { ruleIds: reordered.map((r) => r.id) });
    } catch (e) {
      console.error(e);
    }
  };

  const matchTypeLabel = (t: CategoryMatchType) =>
    ({ extension: "扩展名", domain: "域名", mime_type: "MIME类型", url_contains: "URL包含" }[t]);

  // ── Queue Management ─────────────────────────────────────────────────────

  const loadQueues = async () => {
    setQueuesLoading(true);
    try {
      const qs = await invoke<QueueSummary[]>("list_queues");
      setQueues(qs);
      if (!selectedQueue && qs.length > 0) setSelectedQueue(qs[0]);
    } catch (e) {
      console.error(e);
    } finally {
      setQueuesLoading(false);
    }
  };

  const handleSelectQueue = (q: QueueSummary) => {
    setSelectedQueue(q);
    setQueueForm({ name: q.name, max_concurrent: q.max_concurrent, priority: q.priority });
  };

  const handleUpdateQueue = async () => {
    if (!selectedQueue) return;
    try {
      await invoke("update_queue", {
        queueId: selectedQueue.id,
        name: queueForm.name,
        maxConcurrent: queueForm.max_concurrent,
        priority: queueForm.priority,
      });
      await loadQueues();
    } catch (e) {
      console.error(e);
    }
  };

  const handleCreateQueue = async () => {
    try {
      await invoke("create_queue", {
        name: queueForm.name || "新队列",
        maxConcurrent: queueForm.max_concurrent,
        priority: queueForm.priority,
      });
      setQueueForm({ name: "", max_concurrent: 3, priority: 0 });
      await loadQueues();
    } catch (e) {
      console.error(e);
    }
  };

  const handleDeleteQueue = async () => {
    if (!selectedQueue) return;
    try {
      await invoke("delete_queue", { queueId: selectedQueue.id });
      setSelectedQueue(null);
      await loadQueues();
    } catch (e) {
      console.error(e);
    }
  };

  const handlePauseQueue = async (q: QueueSummary) => {
    try {
      if (q.is_paused) await invoke("resume_queue", { queueId: q.id });
      else await invoke("pause_queue", { queueId: q.id });
      await loadQueues();
    } catch (e) {
      console.error(e);
    }
  };

  // ── Batch Management ──────────────────────────────────────────────────────

  const loadBatches = async () => {
    setBatchesLoading(true);
    try {
      const bs = await invoke<BatchJobInfo[]>("list_batches");
      setBatches(bs);
    } catch (e) {
      console.error(e);
    } finally {
      setBatchesLoading(false);
    }
  };

  const handleCreateBatch = async () => {
    const urls = batchForm.urls.split("\n").map((u) => u.trim()).filter(Boolean);
    if (!batchForm.name || urls.length === 0) return;
    try {
      await invoke("create_batch", {
        name: batchForm.name,
        urls,
        template: null,
        startIndex: 0,
        queueId: null,
      });
      setShowBatchModal(false);
      setBatchForm({ name: "", urls: "" });
      await loadBatches();
    } catch (e) {
      console.error(e);
    }
  };

  const handleDeleteBatch = async (id: string) => {
    try {
      await invoke("delete_batch", { batchId: id });
      await loadBatches();
    } catch (e) {
      console.error(e);
    }
  };

  // ── Schedule ──────────────────────────────────────────────────────────────

  const loadSchedule = async () => {
    setScheduleLoading(true);
    try {
      const [st, tasks] = await Promise.all([
        invoke<ScheduleState>("get_schedule_state"),
        invoke<ScheduleRule[]>("get_schedule_tasks"),
      ]);
      setScheduleState(st);
      setScheduleTasks(tasks);
    } catch (e) {
      console.error(e);
    } finally {
      setScheduleLoading(false);
    }
  };

  const handleSetScheduleEnabled = async (enabled: boolean) => {
    try {
      await invoke("set_schedule_enabled", { enabled });
      setScheduleState((prev) => ({ ...prev, enabled }));
    } catch (e) {
      console.error(e);
    }
  };

  const handleOpenScheduleModal = (rule?: ScheduleRule) => {
    if (rule) {
      setEditingSchedule(rule);
      setScheduleForm(scheduleRuleToForm(rule));
    } else {
      setEditingSchedule(null);
      setScheduleForm({
        name: "",
        schedule_type: "start_download",
        recurrence: { type: "daily" },
        start_time: "08:00",
        end_time: "",
        speed_limit_kbps: "",
      });
    }
    setShowScheduleModal(true);
  };

  const handleSaveSchedule = async () => {
    const rec = scheduleFormToRecurrence(scheduleForm.recurrence);
    const rule: ScheduleRule = {
      id: editingSchedule?.id ?? "",
      name: scheduleForm.name,
      enabled: editingSchedule?.enabled ?? true,
      schedule_type: scheduleForm.schedule_type,
      recurrence: rec,
      start_time: scheduleForm.start_time,
      end_time: scheduleForm.end_time || undefined,
      speed_limit_kbps: scheduleForm.speed_limit_kbps ? parseInt(scheduleForm.speed_limit_kbps) : undefined,
    };
    try {
      if (editingSchedule) {
        await invoke("update_schedule_task", { id: rule.id, updates: rule });
      } else {
        await invoke("create_schedule_task", {
          name: rule.name,
          scheduleType: rule.schedule_type,
          recurrence: rec,
          startTime: rule.start_time,
          endTime: rule.end_time,
          speedLimitKbps: rule.speed_limit_kbps,
        });
      }
      setShowScheduleModal(false);
      setEditingSchedule(null);
      await loadSchedule();
    } catch (e) {
      console.error(e);
    }
  };

  const handleDeleteSchedule = async (id: string) => {
    try {
      await invoke("delete_schedule_task", { id });
      await loadSchedule();
    } catch (e) {
      console.error(e);
    }
  };

  const handleTriggerSchedule = async (id: string) => {
    try {
      await invoke("trigger_schedule_task", { id });
    } catch (e) {
      console.error(e);
    }
  };

  // ── Proxy Management ─────────────────────────────────────────────────────

  const loadProxies = async () => {
    setProxyLoading(true);
    try {
      const [proxies, rules] = await Promise.all([
        invoke<ProxyConfig[]>("list_proxies"),
        invoke<ProxyRule[]>("list_proxy_rules"),
      ]);
      setProxyList(proxies);
      setProxyRules(rules);
    } catch (e) {
      console.error(e);
    } finally {
      setProxyLoading(false);
    }
  };

  const handleSelectProxy = (p: ProxyConfig) => {
    setSelectedProxy(p);
    setProxyForm({
      name: p.name,
      proxy_type: p.proxy_type,
      host: p.host,
      port: p.port,
      username: p.username ?? "",
      password: "",
    });
  };

  const handleSaveProxy = async () => {
    try {
      if (selectedProxy) {
        await invoke("update_proxy", {
          id: selectedProxy.id,
          name: proxyForm.name,
          proxyType: proxyForm.proxy_type,
          host: proxyForm.host,
          port: proxyForm.port,
          username: proxyForm.username || null,
          passwordB64: proxyForm.password ? btoa(proxyForm.password) : null,
        });
      } else {
        await invoke("add_proxy", {
          name: proxyForm.name,
          proxyType: proxyForm.proxy_type,
          host: proxyForm.host,
          port: proxyForm.port,
          username: proxyForm.username || null,
          passwordB64: proxyForm.password ? btoa(proxyForm.password) : null,
        });
      }
      setShowProxyModal(false);
      setSelectedProxy(null);
      await loadProxies();
    } catch (e) {
      console.error(e);
    }
  };

  const handleDeleteProxy = async (id: string) => {
    try {
      await invoke("delete_proxy", { id });
      if (selectedProxy?.id === id) setSelectedProxy(null);
      await loadProxies();
    } catch (e) {
      console.error(e);
    }
  };

  const handleTestProxy = async (p: ProxyConfig) => {
    setTestingProxyId(p.id);
    try {
      const result = await invoke<ProxyTestResult>("test_proxy", { id: p.id });
      alert(result.success ? `✅ 成功 (延迟 ${result.latency_ms}ms)` : `❌ 失败: ${result.error}`);
    } catch (e) {
      alert(`测试失败: ${e}`);
    } finally {
      setTestingProxyId(null);
    }
  };

  const openNewProxyModal = () => {
    setSelectedProxy(null);
    setProxyForm({ name: "", proxy_type: "http", host: "", port: 8080, username: "", password: "" });
    setShowProxyModal(true);
  };

  // ──────────────────────────────────────────────────────────────────────────────

  if (!open) return null;

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && onClose()}>
      <div className="modal options-modal" onClick={(e) => e.stopPropagation()}>
        <div className="modal-title">配置 Multidown</div>
        <div className="options-tabs">
          {TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              className={`options-tab ${tab === t.id ? "active" : ""}`}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>
        <div className="options-tab-content">
          {loading ? (
            <div style={{ padding: 24, color: "#666" }}>加载中…</div>
          ) : (
            <>
              {tab === "general" && (
                <div className="options-section">
                  <div className="options-section-title">通用</div>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.run_at_startup}
                      onChange={(e) => update({ run_at_startup: e.target.checked })}
                    />
                    <span>系统启动时运行 Multidown</span>
                  </label>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.clipboard_monitor}
                      onChange={(e) => update({ clipboard_monitor: e.target.checked })}
                    />
                    <span>监视剪贴板中的下载链接（复制链接后切回窗口时显示下载文件信息）</span>
                  </label>
                </div>
              )}
              {tab === "download" && (
                <div className="options-section">
                  <div className="options-section-title">默认下载设置</div>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.show_start_dialog}
                      onChange={(e) => update({ show_start_dialog: e.target.checked })}
                    />
                    <span>显示开始下载对话框</span>
                  </label>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.show_complete_dialog}
                      onChange={(e) => update({ show_complete_dialog: e.target.checked })}
                    />
                    <span>显示下载完成对话框</span>
                  </label>
                  <div className="form-group">
                    <label>重复下载链接时</label>
                    <select
                      style={{ padding: "6px 10px", minWidth: 200, marginTop: 6, display: "block" }}
                      value={settings.duplicate_action}
                      onChange={(e) => update({ duplicate_action: e.target.value })}
                    >
                      <option value="ask">显示对话框并询问</option>
                      <option value="skip">自动跳过</option>
                      <option value="overwrite">覆盖</option>
                      <option value="rename">重命名</option>
                    </select>
                  </div>
                  <div className="form-group">
                    <label>手动添加任务时使用的 User-Agent</label>
                    <input
                      type="text"
                      value={settings.user_agent}
                      onChange={(e) => update({ user_agent: e.target.value })}
                      style={{ marginTop: 6 }}
                    />
                  </div>
                </div>
              )}
              {tab === "save" && (
                <div className="options-section">
                  <div className="options-section-title">默认下载目录</div>
                  <div className="form-group">
                    <label>默认保存路径</label>
                    <input
                      type="text"
                      value={settings.default_save_path}
                      onChange={(e) => update({ default_save_path: e.target.value })}
                      placeholder="留空则使用系统下载目录"
                      style={{ marginTop: 6 }}
                    />
                  </div>
                  <label className="form-check-row" style={{ marginTop: 4 }}>
                    <input
                      type="checkbox"
                      checked={settings.use_last_save_path}
                      onChange={(e) => update({ use_last_save_path: e.target.checked })}
                    />
                    <span>使用上次的保存路径</span>
                  </label>
                </div>
              )}
              {tab === "connection" && (
                <div className="options-section">
                  <div className="options-section-title">连接</div>
                  <div className="form-group">
                    <label>默认最大连接数（每任务）</label>
                    <select
                      style={{ padding: "6px 10px", minWidth: 80, marginTop: 6 }}
                      value={settings.max_connections_per_task}
                      onChange={(e) => update({ max_connections_per_task: Number(e.target.value) })}
                    >
                      {[4, 8, 16, 24, 32].map((n) => (
                        <option key={n} value={n}>{n}</option>
                      ))}
                    </select>
                  </div>
                  <div className="form-group">
                    <label>全局最大并发任务数</label>
                    <select
                      style={{ padding: "6px 10px", minWidth: 80, marginTop: 6 }}
                      value={settings.max_concurrent_tasks}
                      onChange={(e) => update({ max_concurrent_tasks: Number(e.target.value) })}
                    >
                      {[1, 2, 4, 6, 8, 10].map((n) => (
                        <option key={n} value={n}>{n}</option>
                      ))}
                    </select>
                    <span style={{ color: "#666", fontSize: 12, marginLeft: 8 }}>
                      同时进行中的下载任务数上限
                    </span>
                  </div>
                  <div className="form-group">
                    <label>请求超时（秒）</label>
                    <input
                      type="number"
                      min={5}
                      max={300}
                      value={settings.timeout_secs}
                      onChange={(e) => update({ timeout_secs: Number(e.target.value) || 30 })}
                      style={{ marginTop: 6, width: 100, padding: "6px 10px" }}
                    />
                  </div>
                  <div className="form-group">
                    <label>下载中进度保存间隔（秒）</label>
                    <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 6 }}>
                      <input
                        type="number"
                        min={0}
                        max={300}
                        value={settings.save_progress_interval_secs ?? 30}
                        onChange={(e) => update({ save_progress_interval_secs: Number(e.target.value) || 0 })}
                        style={{ width: 80, padding: "6px 10px" }}
                      />
                      <span style={{ color: "#666", fontSize: 12 }}>0 表示不周期保存</span>
                    </div>
                  </div>
                </div>
              )}
              {tab === "proxy" && (
                <div className="options-section">
                  <div className="options-section-title">代理服务器</div>
                  <div className="form-group">
                    <label>代理使用方式</label>
                    <div style={{ marginTop: 8 }}>
                      <label className="form-check-row" style={{ marginBottom: 8 }}>
                        <input
                          type="radio"
                          name="proxy"
                          checked={settings.proxy_type === "none"}
                          onChange={() => update({ proxy_type: "none" })}
                        />
                        <span>不使用代理</span>
                      </label>
                      <label className="form-check-row" style={{ marginBottom: 8 }}>
                        <input
                          type="radio"
                          name="proxy"
                          checked={settings.proxy_type === "system"}
                          onChange={() => update({ proxy_type: "system" })}
                        />
                        <span>使用系统设置</span>
                      </label>
                      <label className="form-check-row">
                        <input
                          type="radio"
                          name="proxy"
                          checked={settings.proxy_type === "manual"}
                          onChange={() => update({ proxy_type: "manual" })}
                        />
                        <span>手动配置</span>
                      </label>
                    </div>
                  </div>
                  {settings.proxy_type === "manual" && (
                    <>
                      <div className="form-group">
                        <label>代理地址</label>
                        <input
                          type="text"
                          value={settings.proxy_host}
                          onChange={(e) => update({ proxy_host: e.target.value })}
                          placeholder="例如 127.0.0.1"
                          style={{ marginTop: 6 }}
                        />
                      </div>
                      <div className="form-group">
                        <label>端口</label>
                        <input
                          type="number"
                          value={settings.proxy_port}
                          onChange={(e) => update({ proxy_port: Number(e.target.value) || 8080 })}
                          style={{ marginTop: 6, width: 100 }}
                        />
                      </div>
                    </>
                  )}
                </div>
              )}
              {tab === "sounds" && (
                <div className="options-section">
                  <div className="options-section-title">通知</div>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.notification_on_complete}
                      onChange={(e) => update({ notification_on_complete: e.target.checked })}
                    />
                    <span>下载完成时显示系统通知</span>
                  </label>
                  <label className="form-check-row">
                    <input
                      type="checkbox"
                      checked={settings.notification_on_fail}
                      onChange={(e) => update({ notification_on_fail: e.target.checked })}
                    />
                    <span>下载失败时显示系统通知</span>
                  </label>
                </div>
              )}
              {tab === "categories" && (
                <div className="options-section">
                  <div className="options-section-title">分类规则</div>
                  <p style={{ color: "#666", fontSize: 12, marginBottom: 12 }}>
                    拖动排序，规则按从上到下优先级匹配，第一个命中的规则生效。
                    保存路径支持变量：{"{"}category{"}"}（分类名）、{"{"}filename{"}"}（原始文件名）、{"{"}date{"}"}（日期）。
                  </p>
                  {rulesLoading ? (
                    <div style={{ padding: 12, color: "#666" }}>加载中…</div>
                  ) : (
                    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                      {rules.map((rule) => (
                        <div
                          key={rule.id}
                          draggable
                          onDragStart={() => handleDragStart(rule.id)}
                          onDragOver={(e) => handleDragOver(e, rule.id)}
                          onDrop={(e) => handleDrop(e, rule.id)}
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 8,
                            padding: "8px 10px",
                            border: "1px solid #ddd",
                            borderRadius: 6,
                            background: draggedId === rule.id ? "#f0f0f0" : "#fff",
                            cursor: "grab",
                          }}
                        >
                          <span style={{ color: "#999", fontSize: 16 }}>☰</span>
                          <input
                            type="checkbox"
                            checked={rule.enabled}
                            onChange={() => handleToggleRule(rule)}
                            style={{ flexShrink: 0 }}
                          />
                          <span style={{ fontWeight: 500, minWidth: 70 }}>{rule.name}</span>
                          <span style={{ color: "#666", fontSize: 12 }}>
                            {matchTypeLabel(rule.match_type)}: {rule.patterns.join(", ")}
                          </span>
                          <span style={{ color: "#666", fontSize: 12, flex: 1 }}>
                            → {rule.save_path}
                          </span>
                          <button
                            type="button"
                            className="btn"
                            style={{ padding: "2px 8px", fontSize: 12 }}
                            onClick={() => handleEditRule(rule)}
                          >
                            编辑
                          </button>
                          <button
                            type="button"
                            className="btn"
                            style={{ padding: "2px 8px", fontSize: 12, color: "#d00" }}
                            onClick={() => handleDeleteRule(rule.id)}
                          >
                            删除
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                  <button
                    type="button"
                    className="btn btn-primary"
                    style={{ marginTop: 12 }}
                    onClick={handleAddRule}
                  >
                    + 添加规则
                  </button>
                </div>
              )}

              {/* ════════════════════════════════════════════════
                  TAB 1: 队列管理 (Queues)
                  ════════════════════════════════════════════════ */}
              {tab === "queues" && (
                <div className="options-section">
                  <div className="options-section-title">队列管理</div>
                  <div style={{ display: "flex", gap: 16, marginTop: 8 }}>
                    {/* Left: queue list */}
                    <div style={{ width: 220, borderRight: "1px solid #eee", paddingRight: 16 }}>
                      <div style={{ fontWeight: 500, marginBottom: 8, fontSize: 13 }}>队列列表</div>
                      {queuesLoading ? (
                        <div style={{ color: "#666", fontSize: 12 }}>加载中…</div>
                      ) : (
                        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                          {queues.map((q) => (
                            <div
                              key={q.id}
                              onClick={() => handleSelectQueue(q)}
                              style={{
                                padding: "6px 8px",
                                borderRadius: 6,
                                cursor: "pointer",
                                background: selectedQueue?.id === q.id ? "#e8f0fe" : "transparent",
                                border: "1px solid",
                                borderColor: selectedQueue?.id === q.id ? "#1a73e8" : "#ddd",
                                display: "flex",
                                alignItems: "center",
                                justifyContent: "space-between",
                                fontSize: 13,
                              }}
                            >
                              <span style={{ fontWeight: 500 }}>{q.name}</span>
                              <span style={{ color: "#666", fontSize: 11 }}>{q.task_count} 任务</span>
                            </div>
                          ))}
                        </div>
                      )}
                      <button
                        type="button"
                        className="btn btn-primary"
                        style={{ marginTop: 12, width: "100%" }}
                        onClick={handleCreateQueue}
                      >
                        + 新建队列
                      </button>
                    </div>

                    {/* Right: queue detail */}
                    {selectedQueue && (
                      <div style={{ flex: 1 }}>
                        <div style={{ fontWeight: 500, marginBottom: 12 }}>队列详情</div>
                        <div className="form-group">
                          <label>队列名称</label>
                          <input
                            type="text"
                            value={queueForm.name}
                            onChange={(e) => setQueueForm((f) => ({ ...f, name: e.target.value }))}
                            style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                          />
                        </div>
                        <div className="form-group">
                          <label>最大并发数</label>
                          <input
                            type="number"
                            min={1}
                            max={32}
                            value={queueForm.max_concurrent}
                            onChange={(e) => setQueueForm((f) => ({ ...f, max_concurrent: Number(e.target.value) || 1 }))}
                            style={{ marginTop: 4, width: 100, padding: "6px 10px" }}
                          />
                        </div>
                        <div className="form-group">
                          <label>优先级</label>
                          <input
                            type="number"
                            min={0}
                            max={100}
                            value={queueForm.priority}
                            onChange={(e) => setQueueForm((f) => ({ ...f, priority: Number(e.target.value) || 0 }))}
                            style={{ marginTop: 4, width: 100, padding: "6px 10px" }}
                          />
                        </div>
                        <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
                          <button type="button" className="btn btn-primary" onClick={handleUpdateQueue}>
                            保存修改
                          </button>
                          <button
                            type="button"
                            className="btn"
                            style={{ color: selectedQueue?.is_paused ? "#0a0" : "#d00" }}
                            onClick={() => handlePauseQueue(selectedQueue)}
                          >
                            {selectedQueue?.is_paused ? "▶ 启用" : "⏸ 暂停"}
                          </button>
                          <button type="button" className="btn btn-danger" onClick={handleDeleteQueue}>
                            删除队列
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}

              {/* ════════════════════════════════════════════════
                  TAB 2: 批次管理 (Batches)
                  ════════════════════════════════════════════════ */}
              {tab === "batches" && (
                <div className="options-section">
                  <div className="options-section-title">批次管理</div>
                  <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
                    <button
                      type="button"
                      className="btn btn-primary"
                      onClick={() => { setShowBatchModal(true); setBatchForm({ name: "", urls: "" }); }}
                    >
                      + 新建批次
                    </button>
                  </div>
                  {batchesLoading ? (
                    <div style={{ color: "#666", padding: 12 }}>加载中…</div>
                  ) : (
                    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
                      <thead>
                        <tr style={{ borderBottom: "2px solid #eee" }}>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>名称</th>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>任务数</th>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>创建时间</th>
                          <th style={{ padding: "6px 8px" }}></th>
                        </tr>
                      </thead>
                      <tbody>
                        {batches.map((b) => (
                          <tr key={b.id} style={{ borderBottom: "1px solid #f0f0f0" }}>
                            <td style={{ padding: "8px" }}>{b.name}</td>
                            <td style={{ padding: "8px" }}>{b.task_count}</td>
                            <td style={{ padding: "8px", color: "#666" }}>
                              {new Date(b.created_at * 1000).toLocaleString("zh-CN")}
                            </td>
                            <td style={{ padding: "8px", textAlign: "right" }}>
                              <button
                                type="button"
                                className="btn"
                                style={{ padding: "2px 8px", fontSize: 12, color: "#d00" }}
                                onClick={() => handleDeleteBatch(b.id)}
                              >
                                删除
                              </button>
                            </td>
                          </tr>
                        ))}
                        {batches.length === 0 && (
                          <tr>
                            <td colSpan={4} style={{ padding: 24, textAlign: "center", color: "#999" }}>
                              暂无批次
                            </td>
                          </tr>
                        )}
                      </tbody>
                    </table>
                  )}
                </div>
              )}

              {/* ════════════════════════════════════════════════
                  TAB 3: 分类规则2 (Rules) — uses backend rules commands
                  ════════════════════════════════════════════════ */}
              {tab === "rules" && (
                <div className="options-section">
                  <div className="options-section-title">分类规则（规则引擎）</div>
                  <p style={{ color: "#666", fontSize: 12, marginBottom: 12 }}>
                    使用后端规则引擎进行 URL 匹配测试。
                  </p>
                  <div style={{ marginBottom: 12 }}>
                    <input
                      type="text"
                      id="rules-test-url"
                      placeholder="输入 URL 进行测试…"
                      style={{ width: "70%", padding: "6px 10px", marginRight: 8 }}
                    />
                    <button
                      type="button"
                      className="btn btn-primary"
                      onClick={async () => {
                        const url = (document.getElementById("rules-test-url") as HTMLInputElement).value;
                        if (!url) return;
                        try {
                          const result = await invoke<{ matched: boolean; rule_id?: string; save_path?: string } | null>("test_rules", { url });
                          if (result?.matched) {
                            alert(`✅ 匹配规则 ID: ${result.rule_id} → ${result.save_path}`);
                          } else {
                            alert("⚠ 无规则匹配");
                          }
                        } catch (e) {
                          alert(`测试失败: ${e}`);
                        }
                      }}
                    >
                      测试
                    </button>
                  </div>
                  {rulesLoading ? (
                    <div style={{ color: "#666" }}>加载中…</div>
                  ) : (
                    <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                      {rules.map((rule) => (
                        <div
                          key={rule.id}
                          style={{
                            display: "flex",
                            alignItems: "center",
                            gap: 8,
                            padding: "8px 10px",
                            border: "1px solid #ddd",
                            borderRadius: 6,
                            background: "#fff",
                          }}
                        >
                          <input
                            type="checkbox"
                            checked={rule.enabled}
                            onChange={() => handleToggleRule(rule)}
                            style={{ flexShrink: 0 }}
                          />
                          <span style={{ fontWeight: 500, minWidth: 70 }}>{rule.name}</span>
                          <span style={{ color: "#666", fontSize: 12 }}>
                            {matchTypeLabel(rule.match_type)}: {rule.patterns.join(", ")}
                          </span>
                          <span style={{ color: "#666", fontSize: 12, flex: 1 }}>→ {rule.save_path}</span>
                          <button type="button" className="btn" style={{ padding: "2px 8px", fontSize: 12 }} onClick={() => handleEditRule(rule)}>
                            编辑
                          </button>
                          <button type="button" className="btn" style={{ padding: "2px 8px", fontSize: 12, color: "#d00" }} onClick={() => handleDeleteRule(rule.id)}>
                            删除
                          </button>
                        </div>
                      ))}
                    </div>
                  )}
                  <button type="button" className="btn btn-primary" style={{ marginTop: 12 }} onClick={handleAddRule}>
                    + 添加规则
                  </button>
                </div>
              )}

              {/* ════════════════════════════════════════════════
                  TAB 4: 定时调度 (Schedule)
                  ════════════════════════════════════════════════ */}
              {tab === "schedule" && (
                <div className="options-section">
                  <div className="options-section-title">定时调度</div>
                  <div style={{ marginBottom: 12, display: "flex", alignItems: "center", gap: 12 }}>
                    <span style={{ fontSize: 14 }}>调度总开关</span>
                    <label className="form-check-row" style={{ gap: 6 }}>
                      <input
                        type="checkbox"
                        checked={scheduleState.enabled}
                        onChange={(e) => handleSetScheduleEnabled(e.target.checked)}
                      />
                      <span>{scheduleState.enabled ? "已启用" : "已停用"}</span>
                    </label>
                  </div>
                  <div style={{ display: "flex", justifyContent: "flex-end", marginBottom: 8 }}>
                    <button
                      type="button"
                      className="btn btn-primary"
                      onClick={() => handleOpenScheduleModal()}
                    >
                      + 新建调度任务
                    </button>
                  </div>
                  {scheduleLoading ? (
                    <div style={{ color: "#666", padding: 12 }}>加载中…</div>
                  ) : (
                    <table style={{ width: "100%", borderCollapse: "collapse", fontSize: 13 }}>
                      <thead>
                        <tr style={{ borderBottom: "2px solid #eee" }}>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>名称</th>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>类型</th>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>重复</th>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>时间</th>
                          <th style={{ textAlign: "left", padding: "6px 8px", color: "#666" }}>状态</th>
                          <th style={{ padding: "6px 8px" }}></th>
                        </tr>
                      </thead>
                      <tbody>
                        {scheduleTasks.map((task) => (
                          <tr key={task.id} style={{ borderBottom: "1px solid #f0f0f0" }}>
                            <td style={{ padding: "8px", fontWeight: 500 }}>{task.name}</td>
                            <td style={{ padding: "8px", color: "#666" }}>
                              {task.schedule_type === "start_download" ? "开始下载"
                                : task.schedule_type === "pause_all" ? "暂停全部"
                                : task.schedule_type === "resume_all" ? "恢复全部"
                                : task.schedule_type === "speed_limit" ? `限速 ${task.speed_limit_kbps} KB/s`
                                : task.schedule_type}
                            </td>
                            <td style={{ padding: "8px", color: "#666" }}>
                              {task.recurrence.type === "once" ? `一次 (${task.recurrence.date ?? ""})`
                                : task.recurrence.type === "daily" ? "每天"
                                : task.recurrence.type === "weekdays" ? "工作日"
                                : task.recurrence.type === "weekends" ? "周末"
                                : task.recurrence.type === "weekly" ? `每周 ${(task.recurrence as { type: "weekly"; days: string[] }).days.join(", ")}`
                                : JSON.stringify(task.recurrence)}
                            </td>
                            <td style={{ padding: "8px" }}>{task.start_time}</td>
                            <td style={{ padding: "8px" }}>
                              <span style={{ color: task.enabled ? "#0a0" : "#999", fontSize: 12 }}>
                                {task.enabled ? "启用" : "停用"}
                              </span>
                            </td>
                            <td style={{ padding: "8px", textAlign: "right", display: "flex", gap: 4 }}>
                              <button type="button" className="btn" style={{ padding: "2px 8px", fontSize: 12 }} onClick={() => handleOpenScheduleModal(task)}>
                                编辑
                              </button>
                              <button type="button" className="btn" style={{ padding: "2px 8px", fontSize: 12 }} onClick={() => handleTriggerSchedule(task.id)}>
                                立即执行
                              </button>
                              <button type="button" className="btn" style={{ padding: "2px 8px", fontSize: 12, color: "#d00" }} onClick={() => handleDeleteSchedule(task.id)}>
                                删除
                              </button>
                            </td>
                          </tr>
                        ))}
                        {scheduleTasks.length === 0 && (
                          <tr>
                            <td colSpan={6} style={{ padding: 24, textAlign: "center", color: "#999" }}>
                              暂无调度任务
                            </td>
                          </tr>
                        )}
                      </tbody>
                    </table>
                  )}
                </div>
              )}

              {/* ════════════════════════════════════════════════
                  TAB 5: 代理管理 (Proxy)
                  ════════════════════════════════════════════════ */}
              {tab === "proxies" && (
                <div className="options-section">
                  <div className="options-section-title">代理管理</div>
                  <div style={{ display: "flex", gap: 16, marginTop: 8 }}>
                    {/* Left: proxy list */}
                    <div style={{ width: 260, borderRight: "1px solid #eee", paddingRight: 16 }}>
                      <div style={{ fontWeight: 500, marginBottom: 8, fontSize: 13 }}>代理列表</div>
                      {proxyLoading ? (
                        <div style={{ color: "#666", fontSize: 12 }}>加载中…</div>
                      ) : (
                        <div style={{ display: "flex", flexDirection: "column", gap: 4 }}>
                          {proxyList.map((p) => (
                            <div
                              key={p.id}
                              onClick={() => handleSelectProxy(p)}
                              style={{
                                padding: "6px 8px",
                                borderRadius: 6,
                                cursor: "pointer",
                                background: selectedProxy?.id === p.id ? "#e8f0fe" : "transparent",
                                border: "1px solid",
                                borderColor: selectedProxy?.id === p.id ? "#1a73e8" : "#ddd",
                                fontSize: 13,
                              }}
                            >
                              <div style={{ fontWeight: 500 }}>{p.name}</div>
                              <div style={{ color: "#666", fontSize: 11 }}>
                                {p.host}:{p.port} · {p.proxy_type}
                                {p.avg_latency_ms != null && ` · ${p.avg_latency_ms}ms`}
                              </div>
                            </div>
                          ))}
                        </div>
                      )}
                      <button
                        type="button"
                        className="btn btn-primary"
                        style={{ marginTop: 12, width: "100%" }}
                        onClick={openNewProxyModal}
                      >
                        + 添加代理
                      </button>
                    </div>

                    {/* Right: proxy detail */}
                    {selectedProxy && (
                      <div style={{ flex: 1 }}>
                        <div style={{ fontWeight: 500, marginBottom: 12 }}>代理详情</div>
                        <div className="form-group">
                          <label>名称</label>
                          <input
                            type="text"
                            value={proxyForm.name}
                            onChange={(e) => setProxyForm((f) => ({ ...f, name: e.target.value }))}
                            style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                          />
                        </div>
                        <div className="form-group">
                          <label>类型</label>
                          <select
                            value={proxyForm.proxy_type}
                            onChange={(e) => setProxyForm((f) => ({ ...f, proxy_type: e.target.value as "http" | "socks5" | "https" }))}
                            style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                          >
                            <option value="http">HTTP</option>
                            <option value="https">HTTPS</option>
                            <option value="socks5">SOCKS5</option>
                          </select>
                        </div>
                        <div className="form-group">
                          <label>主机</label>
                          <input
                            type="text"
                            value={proxyForm.host}
                            onChange={(e) => setProxyForm((f) => ({ ...f, host: e.target.value }))}
                            placeholder="127.0.0.1"
                            style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                          />
                        </div>
                        <div className="form-group">
                          <label>端口</label>
                          <input
                            type="number"
                            value={proxyForm.port}
                            onChange={(e) => setProxyForm((f) => ({ ...f, port: Number(e.target.value) || 8080 }))}
                            style={{ marginTop: 4, width: 120, padding: "6px 10px" }}
                          />
                        </div>
                        <div className="form-group">
                          <label>用户名（可选）</label>
                          <input
                            type="text"
                            value={proxyForm.username}
                            onChange={(e) => setProxyForm((f) => ({ ...f, username: e.target.value }))}
                            style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                          />
                        </div>
                        <div className="form-group">
                          <label>密码（可选）</label>
                          <input
                            type="password"
                            value={proxyForm.password}
                            onChange={(e) => setProxyForm((f) => ({ ...f, password: e.target.value }))}
                            style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                          />
                        </div>
                        <div style={{ marginTop: 12, display: "flex", gap: 8 }}>
                          <button
                            type="button"
                            className="btn btn-primary"
                            onClick={() => {
                              setSelectedProxy(selectedProxy);
                              handleSaveProxy();
                            }}
                          >
                            保存修改
                          </button>
                          <button
                            type="button"
                            className="btn"
                            onClick={() => handleTestProxy(selectedProxy)}
                            disabled={testingProxyId === selectedProxy.id}
                          >
                            {testingProxyId === selectedProxy.id ? "测试中…" : "测试连接"}
                          </button>
                          <button
                            type="button"
                            className="btn btn-danger"
                            onClick={() => handleDeleteProxy(selectedProxy.id)}
                          >
                            删除
                          </button>
                        </div>
                      </div>
                    )}
                  </div>
                </div>
              )}
            </>
          )}
        </div>
        <div className="modal-footer">
          <button type="button" className="btn" onClick={onClose}>取消</button>
          <button type="button" className="btn btn-primary" onClick={handleSave} disabled={saving}>
            {saving ? "保存中…" : "确定"}
          </button>
        </div>
      </div>

      {/* Rule edit modal */}
      {showRuleModal && editingRule && (
        <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setShowRuleModal(false)}>
          <div className="modal" style={{ minWidth: 480 }} onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{editingRule.id ? "编辑规则" : "添加规则"}</div>
            <div style={{ padding: "0 20px", display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="form-group">
                <label>规则名称</label>
                <input
                  type="text"
                  value={editingRule.name}
                  onChange={(e) => setEditingRule({ ...editingRule, name: e.target.value })}
                  placeholder="例如 视频"
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>匹配类型</label>
                <select
                  value={editingRule.match_type}
                  onChange={(e) => setEditingRule({ ...editingRule, match_type: e.target.value as CategoryMatchType })}
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                >
                  <option value="extension">扩展名</option>
                  <option value="domain">域名</option>
                  <option value="mime_type">MIME类型</option>
                  <option value="url_contains">URL包含</option>
                </select>
              </div>
              <div className="form-group">
                <label>
                  匹配模式
                  <span style={{ color: "#999", fontWeight: 400, marginLeft: 8 }}>
                    {editingRule.match_type === "extension" ? "（不带点，多个用逗号分隔）" : "（多个用逗号分隔）"}
                  </span>
                </label>
                <input
                  type="text"
                  value={editingRule.patterns.join(", ")}
                  onChange={(e) =>
                    setEditingRule({
                      ...editingRule,
                      patterns: e.target.value.split(",").map((s) => s.trim()).filter(Boolean),
                    })
                  }
                  placeholder={
                    editingRule.match_type === "extension"
                      ? "mp4, avi, mkv"
                      : editingRule.match_type === "domain"
                      ? "youtube.com, bilibili.com"
                      : "video/*"
                  }
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>
                  保存路径
                  <span style={{ color: "#999", fontWeight: 400, marginLeft: 8 }}>
                    变量：category, filename, date 可用
                  </span>
                </label>
                <input
                  type="text"
                  value={editingRule.save_path}
                  onChange={(e) => setEditingRule({ ...editingRule, save_path: e.target.value })}
                  placeholder="视频/"
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
            </div>
            <div className="modal-footer">
              <button type="button" className="btn" onClick={() => { setShowRuleModal(false); setEditingRule(null); }}>
                取消
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleSaveRule}
                disabled={!editingRule.name || editingRule.patterns.length === 0 || !editingRule.save_path}
              >
                保存
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Batch create modal */}
      {showBatchModal && (
        <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setShowBatchModal(false)}>
          <div className="modal" style={{ minWidth: 520 }} onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">新建批次</div>
            <div style={{ padding: "0 20px", display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="form-group">
                <label>批次名称</label>
                <input
                  type="text"
                  value={batchForm.name}
                  onChange={(e) => setBatchForm((f) => ({ ...f, name: e.target.value }))}
                  placeholder="例如 我的视频合集"
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>URL 列表（每行一个）</label>
                <textarea
                  value={batchForm.urls}
                  onChange={(e) => setBatchForm((f) => ({ ...f, urls: e.target.value }))}
                  placeholder="https://example.com/video1.mp4&#10;https://example.com/video2.mp4"
                  rows={8}
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box", resize: "vertical" }}
                />
              </div>
            </div>
            <div className="modal-footer">
              <button type="button" className="btn" onClick={() => setShowBatchModal(false)}>取消</button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleCreateBatch}
                disabled={!batchForm.name || !batchForm.urls.trim()}
              >
                创建
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Schedule create/edit modal */}
      {showScheduleModal && (
        <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setShowScheduleModal(false)}>
          <div className="modal" style={{ minWidth: 500 }} onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{editingSchedule ? "编辑调度任务" : "新建调度任务"}</div>
            <div style={{ padding: "0 20px", display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="form-group">
                <label>任务名称</label>
                <input
                  type="text"
                  value={scheduleForm.name}
                  onChange={(e) => setScheduleForm((f) => ({ ...f, name: e.target.value }))}
                  placeholder="例如 夜间下载"
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>动作类型</label>
                <select
                  value={scheduleForm.schedule_type}
                  onChange={(e) => setScheduleForm((f) => ({ ...f, schedule_type: e.target.value as ScheduleType }))}
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                >
                  <option value="start_download">开始下载全部任务</option>
                  <option value="pause_all">暂停全部任务</option>
                  <option value="resume_all">恢复全部任务</option>
                  <option value="speed_limit">全局限速</option>
                </select>
              </div>
              <div className="form-group">
                <label>重复方式</label>
                <select
                  value={scheduleForm.recurrence.type}
                  onChange={(e) =>
                    setScheduleForm((f) => ({
                      ...f,
                      recurrence:
                        e.target.value === "once" ? { type: "once", date: "" }
                        : e.target.value === "daily" ? { type: "daily" }
                        : e.target.value === "weekdays" ? { type: "weekdays" }
                        : e.target.value === "weekends" ? { type: "weekends" }
                        : { type: "weekly", days: [] },
                    }))
                  }
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                >
                  <option value="once">仅一次</option>
                  <option value="daily">每天</option>
                  <option value="weekdays">工作日（周一至周五）</option>
                  <option value="weekends">周末（周六、周日）</option>
                  <option value="weekly">每周</option>
                </select>
              </div>
              {scheduleForm.recurrence.type === "once" && (
                <div className="form-group">
                  <label>日期（YYYY-MM-DD）</label>
                  <input
                    type="text"
                    value={(scheduleForm.recurrence as { type: "once"; date: string }).date}
                    onChange={(e) =>
                      setScheduleForm((f) => ({
                        ...f,
                        recurrence: { type: "once", date: e.target.value },
                      }))
                    }
                    placeholder="2025-01-01"
                    style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                  />
                </div>
              )}
              <div className="form-group">
                <label>开始时间（HH:MM）</label>
                <input
                  type="text"
                  value={scheduleForm.start_time}
                  onChange={(e) => setScheduleForm((f) => ({ ...f, start_time: e.target.value }))}
                  placeholder="08:00"
                  style={{ marginTop: 4, width: 120, padding: "6px 10px" }}
                />
              </div>
              {scheduleForm.schedule_type === "speed_limit" && (
                <div className="form-group">
                  <label>限速值（KB/s）</label>
                  <input
                    type="number"
                    min={1}
                    value={scheduleForm.speed_limit_kbps}
                    onChange={(e) => setScheduleForm((f) => ({ ...f, speed_limit_kbps: e.target.value }))}
                    placeholder="500"
                    style={{ marginTop: 4, width: 120, padding: "6px 10px" }}
                  />
                </div>
              )}
              {scheduleForm.schedule_type === "speed_limit" && (
                <div className="form-group">
                  <label>结束时间（HH:MM，可选）</label>
                  <input
                    type="text"
                    value={scheduleForm.end_time}
                    onChange={(e) => setScheduleForm((f) => ({ ...f, end_time: e.target.value }))}
                    placeholder="23:00"
                    style={{ marginTop: 4, width: 120, padding: "6px 10px" }}
                  />
                </div>
              )}
            </div>
            <div className="modal-footer">
              <button type="button" className="btn" onClick={() => { setShowScheduleModal(false); setEditingSchedule(null); }}>
                取消
              </button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleSaveSchedule}
                disabled={!scheduleForm.name || !scheduleForm.start_time}
              >
                保存
              </button>
            </div>
          </div>
        </div>
      )}

      {/* Proxy create/edit modal */}
      {showProxyModal && (
        <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setShowProxyModal(false)}>
          <div className="modal" style={{ minWidth: 480 }} onClick={(e) => e.stopPropagation()}>
            <div className="modal-title">{selectedProxy ? "编辑代理" : "添加代理"}</div>
            <div style={{ padding: "0 20px", display: "flex", flexDirection: "column", gap: 12 }}>
              <div className="form-group">
                <label>名称</label>
                <input
                  type="text"
                  value={proxyForm.name}
                  onChange={(e) => setProxyForm((f) => ({ ...f, name: e.target.value }))}
                  placeholder="我的代理"
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>类型</label>
                <select
                  value={proxyForm.proxy_type}
                  onChange={(e) => setProxyForm((f) => ({ ...f, proxy_type: e.target.value as "http" | "socks5" | "https" }))}
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                >
                  <option value="http">HTTP</option>
                  <option value="https">HTTPS</option>
                  <option value="socks5">SOCKS5</option>
                </select>
              </div>
              <div className="form-group">
                <label>主机</label>
                <input
                  type="text"
                  value={proxyForm.host}
                  onChange={(e) => setProxyForm((f) => ({ ...f, host: e.target.value }))}
                  placeholder="127.0.0.1"
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>端口</label>
                <input
                  type="number"
                  value={proxyForm.port}
                  onChange={(e) => setProxyForm((f) => ({ ...f, port: Number(e.target.value) || 8080 }))}
                  style={{ marginTop: 4, width: 120, padding: "6px 10px" }}
                />
              </div>
              <div className="form-group">
                <label>用户名（可选）</label>
                <input
                  type="text"
                  value={proxyForm.username}
                  onChange={(e) => setProxyForm((f) => ({ ...f, username: e.target.value }))}
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
              <div className="form-group">
                <label>密码（可选）</label>
                <input
                  type="password"
                  value={proxyForm.password}
                  onChange={(e) => setProxyForm((f) => ({ ...f, password: e.target.value }))}
                  style={{ marginTop: 4, width: "100%", padding: "6px 10px", boxSizing: "border-box" }}
                />
              </div>
            </div>
            <div className="modal-footer">
              <button type="button" className="btn" onClick={() => setShowProxyModal(false)}>取消</button>
              <button
                type="button"
                className="btn btn-primary"
                onClick={handleSaveProxy}
                disabled={!proxyForm.name || !proxyForm.host}
              >
                保存
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}