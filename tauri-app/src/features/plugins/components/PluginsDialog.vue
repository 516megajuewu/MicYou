<script setup lang="ts">
import { watch } from 'vue';
import { X, Puzzle } from '@lucide/vue';
import { usePlugins } from '../composables/usePlugins';
import PluginsPanel from './PluginsPanel.vue';

const props = defineProps<{ isOpen: boolean }>();
const emit = defineEmits<{ close: [] }>();

const p = usePlugins();

watch(
  () => props.isOpen,
  (open) => {
    if (open) p.refresh();
  },
);
</script>

<template>
  <Transition name="dialog">
    <div
      v-if="props.isOpen"
      class="fixed inset-0 z-50 flex items-center justify-center p-8 bg-black/60 backdrop-blur-sm"
      @click.self="emit('close')"
    >
      <div class="settings-panel relative backdrop-blur-2xl w-full max-w-3xl">
        <button
          @click="emit('close')"
          class="absolute top-4 right-4 z-[100] w-10 h-10 rounded-full bg-surface-variant/40 hover:bg-surface-variant/80 flex items-center justify-center transition-colors"
        >
          <X class="w-5 h-5 text-on-surface" />
        </button>

        <!-- Header -->
        <div class="p-6 pb-4 border-b border-surface-variant/20">
          <div class="flex items-center gap-3">
            <Puzzle class="w-6 h-6 text-primary" />
            <div>
              <h2 class="text-xl font-bold text-primary">{{ $t('plugins.title') }}</h2>
              <p class="text-xs text-on-surface-variant mt-0.5">{{ $t('plugins.desc') }}</p>
            </div>
          </div>
        </div>

        <!-- Body: 复用插件管理面板 -->
        <div class="settings-scrollbar p-6 overflow-y-auto overscroll-contain max-h-[65vh]">
          <PluginsPanel />
        </div>
      </div>
    </div>
  </Transition>
</template>
