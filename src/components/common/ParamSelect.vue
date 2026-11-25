<script setup lang="ts">
import AppSelect from '@/components/common/AppSelect.vue'
/**
 * 节点面板 / 列表里的参数下拉：比例、出图模型、视频模型统一用这一套外观，
 * 避免三个控件各写一份 CSS 导致圆角、箭头、字号对不齐。
 */
withDefaults(
  defineProps<{
    label?: string
    modelValue?: string
    /** 固定宽度的下拉（如比例）传窄一点，自适应传 'auto' */
    width?: string
    maxWidth?: string
    title?: string
  }>(),
  { label: '', modelValue: '', width: 'auto', maxWidth: '', title: '' }
)

const emit = defineEmits<{ 'update:modelValue': [value: string] }>()

</script>

<template>
  <label
    class="psel"
    :style="{
      ...(width === 'auto' ? {} : { flex: `0 0 ${width}` }),
      ...(maxWidth ? { maxWidth } : {})
    }"
  >
    <span v-if="label" class="psel-k">{{ label }}</span>
    <AppSelect :model-value="modelValue" :title="title || label" @update:model-value="emit('update:modelValue', $event)">
      <slot />
    </AppSelect>
  </label>
</template>

<style scoped>
.psel {
  display: flex;
  align-items: center;
  gap: 6px;
  flex: 1 1 auto;
  min-width: 0;
  margin: 0;
}
.psel-k {
  flex: 0 0 auto;
  font-size: 10px;
  line-height: 1;
  color: var(--color-text-4);
  letter-spacing: 0.02em;
  white-space: nowrap;
}
.psel .app-select {
  flex: 1 1 auto;
  min-width: 0;
  width: 100%;
  height: 28px;
  padding: 0 8px;
  border-radius: 8px;
  font-size: 12px;
  line-height: 1;
  font-family: inherit;
}
</style>
