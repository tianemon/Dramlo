<script setup lang="ts">
import { ref, useAttrs } from 'vue'
import HomeIcon from './HomeIcon.vue'

defineOptions({ inheritAttrs: false })
const props = defineProps<{
  modelValue?: number
  value?: string | number | boolean | null
  disabled?: boolean
  readonly?: boolean
}>()
const emit = defineEmits<{ 'update:modelValue': [value: number | undefined] }>()
const attrs = useAttrs()
const input = ref<HTMLInputElement>()

function inputAttrs() {
  const { class: _class, style: _style, ...rest } = attrs
  return rest
}

function readValue(event: Event) {
  const el = event.target as HTMLInputElement
  emit('update:modelValue', Number.isFinite(el.valueAsNumber) ? el.valueAsNumber : undefined)
}

function step(direction: 1 | -1) {
  const el = input.value
  if (!el || props.disabled || props.readonly) return
  const previous = el.value
  if (el.step === 'any') {
    const min = el.min === '' ? -Infinity : Number(el.min)
    const max = el.max === '' ? Infinity : Number(el.max)
    const value = Number.isFinite(el.valueAsNumber) ? el.valueAsNumber : 0
    el.valueAsNumber = Math.min(max, Math.max(min, value + direction))
  } else {
    el.stepUp(direction)
  }
  if (el.value === previous) return
  el.dispatchEvent(new Event('input', { bubbles: true }))
  el.dispatchEvent(new Event('change', { bubbles: true }))
}
</script>

<template>
  <span class="app-number nodrag" :class="$attrs.class" :style="$attrs.style">
    <input ref="input" v-bind="inputAttrs()" type="number" :value="modelValue ?? value ?? ''"
      :disabled="disabled" :readonly="readonly" @input="readValue">
    <span class="number-steps">
      <button type="button" tabindex="-1" aria-label="增大数值" :disabled="disabled || readonly" @mousedown.prevent @click="step(1)">
        <HomeIcon name="chevronDown" :size="12" />
      </button>
      <button type="button" tabindex="-1" aria-label="减小数值" :disabled="disabled || readonly" @mousedown.prevent @click="step(-1)">
        <HomeIcon name="chevronDown" :size="12" />
      </button>
    </span>
  </span>
</template>

<style scoped>
.app-number {
  display: inline-grid;
  grid-template-columns: minmax(0, 1fr) 18px;
  align-items: center;
  gap: 4px;
  min-width: 0;
  padding: 4px 8px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-2);
  vertical-align: middle;
}
.app-number input {
  appearance: textfield;
  min-width: 0;
  width: 100%;
  padding: 0;
  border: 0;
  background: transparent;
  font: inherit;
}
.app-number input::-webkit-inner-spin-button,
.app-number input::-webkit-outer-spin-button { appearance: none; margin: 0; }
.app-number:has(input:focus-visible) { outline: 2px solid var(--focus-ring); outline-offset: 2px; }
.app-number input:focus-visible { outline: none; }
.number-steps { display: grid; margin-block: -3px; }
.number-steps button {
  display: grid;
  place-items: center;
  height: 12px;
  padding: 0;
  border-radius: 3px;
  color: var(--color-text-3);
}
.number-steps button:first-child svg { rotate: 180deg; }
.number-steps button:disabled { opacity: .4; cursor: default; }
</style>
