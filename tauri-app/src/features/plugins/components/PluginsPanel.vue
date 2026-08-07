<script setup lang="ts">
import { ref, onMounted } from 'vue';
import {
  RefreshCw,
  Puzzle,
  FolderOpen,
  Download,
  Trash2,
  ToggleLeft,
  ToggleRight,
  TerminalSquare,
} from '@lucide/vue';
import { usePlugins, type PluginView } from '../composables/usePlugins';

// 可复用的插件管理面板：用于设置对话框的「插件」页面
// 首次挂载即拉取插件列表（单例状态，两个入口共享）
const p = usePlugins();
onMounted(() => {
  p.refresh();
});

const showLogsFor = ref<string | null>(null);
const logLines = ref<string[]>([]);
const openConfigFor = ref<string | null>(null);
const configJson = ref('{}');
const configSaved = ref(false);

function runtimeLabel(runtime: string) {
  return runtime === 'wasm' ? 'WASM' : 'Native';
}

function kindLabel(kind: string) {
  return `plugins.kind.${kind}`;
}

async function showLogs(plugin: PluginView) {
  if (showLogsFor.value === plugin.id) {
    showLogsFor.value = null;
    return;
  }
  showLogsFor.value = plugin.id;
  logLines.value = await p.logs(plugin);
}

function openConfig(plugin: PluginView) {
  openConfigFor.value = plugin.id;
  configSaved.value = false;
  configJson.value = '{}';
}

async function saveConfig(plugin: PluginView) {
  try {
    const parsed = JSON.parse(configJson.value);
    // write each top-level key
    for (const [key, value] of Object.entries(parsed)) {
      await p.saveConfig(plugin, key, value);
    }
    configSaved.value = true;
    p.error.value = null;
  } catch (e) {
    p.error.value = String(e);
  }
}

function confirmUninstall(plugin: PluginView) {
  if (window.confirm(`Uninstall ${plugin.name} (${plugin.id})?`)) {
    p.uninstall(plugin);
  }
}
</script>

<template>
  <div class="space-y-4">
    <!-- Toolbar: sync status + refresh -->
    <div class="flex items-center justify-between">
      <span
        class="px-3 py-1 rounded-full text-xs font-medium"
        :class="
          p.syncStatus.value.deviceConnected
            ? 'bg-green-500/15 text-green-400'
            : 'bg-surface-variant/40 text-on-surface-variant'
        "
      >
        {{
          p.syncStatus.value.deviceConnected
            ? $t('plugins.sync.connected')
            : $t('plugins.sync.disconnected')
        }}
      </span>
      <button
        @click="p.refresh"
        class="w-9 h-9 rounded-full bg-surface-variant/40 hover:bg-surface-variant flex items-center justify-center transition-colors"
        :title="$t('plugins.refresh')"
      >
        <RefreshCw
          class="w-4 h-4 text-on-surface-variant"
          :class="{ 'animate-spin': p.loading.value }"
        />
      </button>
    </div>

    <p v-if="p.error.value" class="px-4 py-2 rounded-lg bg-red-500/10 text-red-400 text-sm">
      {{ p.error.value }}
    </p>

    <div
      v-if="p.loading.value && p.plugins.value.length === 0"
      class="py-16 text-center text-on-surface-variant text-sm"
    >
      {{ $t('plugins.loading') }}
    </div>

    <div v-else-if="p.plugins.value.length === 0" class="py-16 text-center">
      <p class="text-on-surface-variant text-sm">{{ $t('plugins.noPlugins') }}</p>
      <button
        @click="p.openDir()"
        class="mt-4 inline-flex items-center gap-2 px-4 py-2 rounded-full bg-primary/20 text-primary hover:bg-primary/30 text-sm font-medium"
      >
        <FolderOpen class="w-4 h-4" />
        {{ $t('plugins.openDir') }}
      </button>
    </div>

    <template v-else>
      <!-- Install hint: import zip + open dir -->
      <div
        class="rounded-lg bg-surface-variant/20 px-4 py-3 text-xs text-on-surface-variant space-y-2"
      >
        <div class="flex items-center justify-between gap-3">
          <span>{{ $t('plugins.installHint') }}</span>
          <button
            @click="p.importPlugin()"
            :disabled="p.busyId.value === 'import'"
            class="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-primary/20 text-primary hover:bg-primary/30 font-medium disabled:opacity-50"
          >
            <Download class="w-3.5 h-3.5" />
            {{ p.busyId.value === 'import' ? $t('plugins.importing') : $t('plugins.import') }}
          </button>
        </div>
        <div class="flex items-center justify-between gap-3">
          <span>{{ $t('plugins.installHintDir') }}</span>
          <button
            @click="p.openDir()"
            class="inline-flex items-center gap-1.5 px-3 py-1 rounded-full bg-surface-variant/40 hover:bg-surface-variant text-on-surface-variant font-medium"
          >
            <FolderOpen class="w-3.5 h-3.5" />
            {{ $t('plugins.openDir') }}
          </button>
        </div>
      </div>

      <div
        v-for="plugin in p.plugins.value"
        :key="plugin.id"
        class="rounded-xl bg-surface-container-lowest/60 border border-surface-variant/20 p-4"
      >
        <div class="flex items-start justify-between gap-3">
          <div class="min-w-0">
            <div class="flex items-center gap-2 flex-wrap">
              <h3 class="font-bold text-on-surface">{{ plugin.name }}</h3>
              <span
                class="px-2 py-0.5 rounded-md text-[10px] font-semibold tracking-wide"
                :class="
                  plugin.runtime === 'wasm'
                    ? 'bg-purple-500/15 text-purple-400'
                    : 'bg-amber-500/15 text-amber-400'
                "
              >
                {{ runtimeLabel(plugin.runtime) }}
              </span>
              <span class="px-2 py-0.5 rounded-md text-[10px] bg-primary/10 text-primary">
                {{ $t(kindLabel(plugin.kind)) }}
              </span>
              <span
                v-if="plugin.dspNode"
                class="px-2 py-0.5 rounded-md text-[10px] bg-green-500/15 text-green-400"
              >
                {{ $t('plugins.inChain') }}
              </span>
            </div>
            <p class="text-xs text-on-surface-variant mt-1 font-mono">
              {{ plugin.id }} · v{{ plugin.version }}
            </p>
            <p
              v-if="plugin.description"
              class="text-xs text-on-surface-variant/80 mt-1 line-clamp-2"
            >
              {{ plugin.description }}
            </p>
            <p v-if="plugin.error" class="text-xs text-red-400 mt-1">{{ plugin.error }}</p>
            <div v-if="plugin.capabilities.length" class="flex flex-wrap gap-1 mt-2">
              <span
                v-for="cap in plugin.capabilities"
                :key="cap"
                class="px-1.5 py-0.5 rounded text-[10px] bg-surface-variant/30 text-on-surface-variant/70 font-mono"
              >
                {{ cap }}
              </span>
            </div>
          </div>

          <div class="flex items-center gap-2 shrink-0">
            <button
              @click="showLogs(plugin)"
              class="w-9 h-9 rounded-full bg-surface-variant/40 hover:bg-surface-variant flex items-center justify-center transition-colors"
              :title="$t('plugins.logs')"
            >
              <TerminalSquare class="w-4 h-4 text-on-surface-variant" />
            </button>
            <button
              @click="openConfig(plugin)"
              class="w-9 h-9 rounded-full bg-surface-variant/40 hover:bg-surface-variant flex items-center justify-center transition-colors"
              :title="$t('plugins.config')"
            >
              <Puzzle class="w-4 h-4 text-on-surface-variant" />
            </button>
            <button
              @click="confirmUninstall(plugin)"
              class="w-9 h-9 rounded-full bg-surface-variant/40 hover:bg-red-500/20 flex items-center justify-center transition-colors"
              :title="$t('plugins.uninstall')"
            >
              <Trash2 class="w-4 h-4 text-on-surface-variant hover:text-red-400" />
            </button>
            <button
              @click="p.toggle(plugin)"
              :disabled="p.busyId.value === plugin.id"
              class="flex items-center gap-1.5 px-3 py-1.5 rounded-full text-xs font-medium transition-colors disabled:opacity-50"
              :class="
                plugin.enabled
                  ? 'bg-primary/20 text-primary'
                  : 'bg-surface-variant/40 text-on-surface-variant'
              "
            >
              <ToggleRight v-if="plugin.enabled" class="w-4 h-4" />
              <ToggleLeft v-else class="w-4 h-4" />
              {{ plugin.enabled ? $t('plugins.enabled') : $t('plugins.disabled') }}
            </button>
          </div>
        </div>

        <!-- Config editor -->
        <div
          v-if="openConfigFor === plugin.id"
          class="mt-3 pt-3 border-t border-surface-variant/20"
        >
          <div class="flex items-center justify-between mb-2">
            <span class="text-xs font-medium text-on-surface-variant">{{
              $t('plugins.config')
            }}</span>
            <span v-if="configSaved" class="text-xs text-green-400">{{
              $t('plugins.configSaved')
            }}</span>
          </div>
          <textarea
            v-model="configJson"
            rows="3"
            spellcheck="false"
            class="w-full bg-surface-variant/20 rounded-lg p-2 text-xs font-mono text-on-surface outline-none focus:ring-1 focus:ring-primary/40"
            placeholder='{ "key": "value" }'
          ></textarea>
          <div class="flex justify-end mt-2">
            <button
              @click="saveConfig(plugin)"
              class="px-4 py-1.5 rounded-full bg-primary/20 text-primary hover:bg-primary/30 text-xs font-medium"
            >
              {{ $t('plugins.save') }}
            </button>
          </div>
        </div>

        <!-- Logs -->
        <div v-if="showLogsFor === plugin.id" class="mt-3 pt-3 border-t border-surface-variant/20">
          <span class="text-xs font-medium text-on-surface-variant">{{ $t('plugins.logs') }}</span>
          <pre
            class="mt-2 max-h-40 overflow-y-auto bg-black/30 rounded-lg p-3 text-[11px] font-mono text-green-300/90 whitespace-pre-wrap"
            >{{ logLines.join('\n') || $t('plugins.noLogs') }}</pre>
        </div>
      </div>
    </template>
  </div>
</template>
