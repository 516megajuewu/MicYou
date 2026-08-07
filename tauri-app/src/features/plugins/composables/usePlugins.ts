import { ref } from 'vue';
import { invoke } from '@tauri-apps/api/core';

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
  enabled: boolean;
  loaded: boolean;
  dspNode: boolean;
  error?: string | null;
}

export interface PluginSyncStatus {
  deviceConnected: boolean;
  transportReady: boolean;
}

export function usePlugins() {
  const plugins = ref<PluginView[]>([]);
  const syncStatus = ref<PluginSyncStatus>({ deviceConnected: false, transportReady: false });
  const loading = ref(false);
  const busyId = ref<string | null>(null);
  const error = ref<string | null>(null);

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

  async function logs(plugin: PluginView): Promise<string[]> {
    try {
      return await invoke<string[]>('get_plugin_logs', { id: plugin.id });
    } catch {
      return [];
    }
  }

  async function openDir(): Promise<string | null> {
    try {
      return await invoke<string>('open_plugins_dir');
    } catch (e) {
      error.value = String(e);
      return null;
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
    logs,
    openDir,
  };
}
