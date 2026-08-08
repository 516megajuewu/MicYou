import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { openPath } from '@tauri-apps/plugin-opener';

export interface PluginView {
  id: string;
  name: string;
  version: string;
  author?: string | null;
  description?: string | null;
  runtime: string; // native | wasm
  kind: string; // dsp | utility | ui | bridge
  platforms: string[];
  capabilities: string[];
  ui?: {
    route: string;
    label?: string;
    entry?: string | null;
    panels?: Array<{ id: string; label: string; entry: string; sidebar?: boolean }>;
  } | null;
  enabled: boolean;
  loaded: boolean;
  dspNode: boolean;
  error?: string | null;
}

export interface PluginSyncStatus {
  deviceConnected: boolean;
  transportReady: boolean;
}

// 模块级单例：设置对话框与（曾经的）独立对话框共享同一份状态
const plugins = ref<PluginView[]>([]);
const syncStatus = ref<PluginSyncStatus>({ deviceConnected: false, transportReady: false });
const loading = ref(false);
const busyId = ref<string | null>(null);
const error = ref<string | null>(null);

export function usePlugins() {
  async function refresh() {
    loading.value = true;
    error.value = null;
    try {
      plugins.value = await invoke<PluginView[]>('list_plugins');
      syncStatus.value = await invoke<PluginSyncStatus>('get_plugin_sync_status');
    } catch (e) {
      error.value = String(e);
    } finally {
      loading.value = false;
    }
  }

  async function toggle(plugin: PluginView) {
    busyId.value = plugin.id;
    error.value = null;
    try {
      await invoke('set_plugin_enabled', { id: plugin.id, enabled: !plugin.enabled });
      await refresh();
    } catch (e) {
      error.value = String(e);
      await refresh();
    } finally {
      busyId.value = null;
    }
  }

  async function uninstall(plugin: PluginView) {
    busyId.value = plugin.id;
    error.value = null;
    try {
      await invoke('uninstall_plugin', { id: plugin.id });
      await refresh();
    } catch (e) {
      error.value = String(e);
    } finally {
      busyId.value = null;
    }
  }

  async function saveConfig(plugin: PluginView, key: string, value: unknown) {
    try {
      await invoke('set_plugin_config', { id: plugin.id, key, value });
      return true;
    } catch (e) {
      error.value = String(e);
      return false;
    }
  }

  async function getConfig(plugin: PluginView): Promise<Record<string, unknown>> {
    try {
      const v = await invoke<Record<string, unknown>>('get_plugin_config', { id: plugin.id });
      return v ?? {};
    } catch {
      return {};
    }
  }

  async function logs(plugin: PluginView): Promise<string[]> {
    try {
      return await invoke<string[]>('get_plugin_logs', { id: plugin.id });
    } catch {
      return [];
    }
  }

  /** 触发插件 UI 动作（soundpad 按钮等）：topic ui:<action>，payload 为 JSON 字符串 */
  async function trigger(plugin: PluginView, action: string, payload?: string) {
    error.value = null;
    try {
      await invoke('plugin_trigger', { pluginId: plugin.id, action, payload: payload ?? null });
      return true;
    } catch (e) {
      error.value = String(e);
      return false;
    }
  }

  /** 打开系统文件管理器显示插件目录 */
  async function openDir(): Promise<boolean> {
    try {
      const dir = await invoke<string>('open_plugins_dir');
      if (dir) await openPath(dir);
      return true;
    } catch (e) {
      error.value = String(e);
      return false;
    }
  }

  /** 选择并导入插件压缩包（.zip） */
  async function importPlugin(): Promise<boolean> {
    try {
      const picked = await open({
        multiple: false,
        directory: false,
        filters: [{ name: 'MicYou plugin', extensions: ['zip'] }],
      });
      if (!picked) return false; // 用户取消
      busyId.value = 'import';
      error.value = null;
      await invoke('import_plugin', { source: String(picked) });
      await refresh();
      return true;
    } catch (e) {
      error.value = String(e);
      return false;
    } finally {
      busyId.value = null;
    }
  }

  return {
    plugins,
    syncStatus,
    loading,
    busyId,
    error,
    refresh,
    toggle,
    uninstall,
    saveConfig,
    getConfig,
    logs,
    trigger,
    openDir,
    importPlugin,
  };
}
