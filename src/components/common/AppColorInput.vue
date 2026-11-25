<script setup lang="ts">
import { computed, ref } from 'vue'
import GradientButton from './GradientButton.vue'

const props = defineProps<{ modelValue?: string; value?: string; disabled?: boolean }>()
const emit = defineEmits<{ 'update:modelValue': [value: string] }>()
const dialog = ref<HTMLDialogElement>()
const draft = ref('#000000')
const current = computed(() => props.modelValue ?? props.value ?? '#000000')
const channels = ['红', '绿', '蓝']

function normalize(color: string) {
  const value = color.trim().replace(/^#/, '')
  if (/^[0-9a-f]{6}$/i.test(value)) return '#' + value.toLowerCase()
  if (/^[0-9a-f]{3}$/i.test(value)) return '#' + [...value.toLowerCase()].map(char => char.repeat(2)).join('')
  return undefined
}

const valid = computed(() => normalize(draft.value))
const preview = computed(() => valid.value ?? normalize(current.value) ?? '#000000')
const rgb = computed(() => [1, 3, 5].map(offset => Number.parseInt(preview.value.slice(offset, offset + 2), 16)))

function start() {
  draft.value = normalize(current.value) ?? '#000000'
  dialog.value?.showModal()
}

function changeChannel(index: number, value: number) {
  const next = [...rgb.value]
  next[index] = value
  draft.value = '#' + next.map(channel => channel.toString(16).padStart(2, '0')).join('')
}

function apply() {
  if (!valid.value) return
  emit('update:modelValue', valid.value)
  dialog.value?.close()
}
</script>

<template>
  <button class="app-color" type="button" :disabled="disabled" :title="current" @click="start">
    <span :style="{ backgroundColor: current }" aria-hidden="true" />
    <Teleport to="body">
      <dialog ref="dialog" class="color-dialog" aria-label="选择颜色">
        <div>
          <h2>选择颜色</h2>
          <div class="color-editor">
            <div class="color-preview" :style="{ backgroundColor: preview }" aria-hidden="true" />
            <div class="color-channels">
              <label v-for="(label, index) in channels" :key="label">
                {{ label }}
                <input type="range" min="0" max="255" :value="rgb[index]"
                  @input="changeChannel(index, ($event.target as HTMLInputElement).valueAsNumber)">
                <span>{{ rgb[index] }}</span>
              </label>
            </div>
          </div>
          <label class="color-hex">HEX <input v-model.trim="draft" :aria-invalid="!valid" maxlength="7" spellcheck="false" @keydown.enter.prevent="apply"></label>
          <div class="color-actions">
            <GradientButton type="ghost" @click="dialog?.close()">取消</GradientButton>
            <GradientButton :disabled="!valid" @click="apply">应用</GradientButton>
          </div>
        </div>
      </dialog>
    </Teleport>
  </button>
</template>

<style scoped>
.app-color {
  display: inline-flex;
  align-items: center;
  width: 34px;
  height: 26px;
  padding: 3px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-2);
}
.app-color:hover:not(:disabled) { background: var(--control-hover); }
.app-color:disabled { opacity: .5; cursor: default; }
.app-color > span { width: 100%; height: 100%; border-radius: 3px; box-shadow: inset 0 0 0 1px #0002; }
.color-dialog {
  width: min(340px, calc(100% - 32px));
  padding: 16px;
  background: var(--glass-panel);
  color: var(--color-text-1);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  box-shadow: var(--shadow-lg);
}
.color-dialog::backdrop { background: #0007; }
.color-dialog h2 { font-size: 15px; margin: 0 0 12px; }
.color-editor { display: flex; align-items: stretch; gap: 12px; }
.color-preview { width: 60px; flex: none; border-radius: var(--radius-sm); }
.color-channels { flex: 1; }
.color-channels label { display: grid; grid-template-columns: auto minmax(0, 1fr) 28px; align-items: center; gap: 7px; }
.color-channels span { text-align: end; font-variant-numeric: tabular-nums; }
.color-hex { display: flex; align-items: center; gap: 8px; margin-top: 12px; }
.color-hex input { width: 90px; padding: 4px 6px; background: var(--nb-bg-inset); border: 1px solid var(--color-border); border-radius: var(--radius-sm); }
.color-hex input[aria-invalid='true'] { border-color: var(--color-error); }
.color-actions { display: flex; justify-content: end; gap: 6px; margin-top: 12px; }
</style>
