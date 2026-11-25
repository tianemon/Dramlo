<script setup lang="ts">
import AppSelect from '@/components/common/AppSelect.vue'
import { computed } from 'vue'
import { llmChoiceKey, namedModelGroups, parseLlmChoiceKey, useSettingsStore } from '@/stores'

withDefaults(
  defineProps<{
    label?: string
  }>(),
  { label: '配音' }
)

const settings = useSettingsStore()
const groups = computed(() => namedModelGroups(settings.config.ttsProfiles ?? []))

const model = computed({
  get() {
    const id = settings.config.activeTtsId
    const name = settings.config.tts.model
    return id && name ? llmChoiceKey(id, name) : ''
  },
  set(v: string) {
    const parsed = parseLlmChoiceKey(v)
    if (parsed) settings.selectTtsModel(parsed.profileId, parsed.model)
  }
})
</script>

<template>
  <label class="tts-model">
    <span>{{ label }}</span>
    <AppSelect v-model="model">
      <option v-if="!groups.some((g) => g.models.length)" value="">请先在设置里添加配音模型</option>
      <optgroup v-for="g in groups" :key="g.id" :label="g.name">
        <option v-for="m in g.models" :key="g.id + m" :value="llmChoiceKey(g.id, m)">{{ m }}</option>
      </optgroup>
    </AppSelect>
  </label>
</template>

<style scoped>
.tts-model {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: center;
  gap: 6px;
  margin: 0;
}
.tts-model span {
  flex-shrink: 0;
  font-size: 10px;
  color: var(--color-text-4);
}
.tts-model .app-select {
  flex: 1;
  min-width: 0;
  height: 28px;
  padding: 0 8px;
  border: 1px solid var(--color-border);
  border-radius: 4px;
  background-color: var(--color-bg-2);




  color: var(--color-text-1);
  font-size: 11px;


  cursor: pointer;
}
</style>
