<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useAppConfirmation } from '@/composables/useAppConfirmation'
import GradientButton from './GradientButton.vue'

const { pending, finish } = useAppConfirmation()
const dialog = ref<HTMLDialogElement>()
const route = useRoute()

watch(pending, request => {
  if (request && !dialog.value?.open) dialog.value?.showModal()
}, { flush: 'post' })

function answer(accepted: boolean) {
  dialog.value?.close()
  finish(accepted)
}

watch(() => route.fullPath, () => {
  if (pending.value) answer(false)
})
onBeforeUnmount(() => finish(false))
</script>

<template>
  <Teleport to="body">
    <dialog
      ref="dialog"
      class="app-confirmation"
      aria-labelledby="app-confirmation-title"
      aria-describedby="app-confirmation-message"
      @cancel.prevent="answer(false)"
    >
      <h2 id="app-confirmation-title">确认操作</h2>
      <p id="app-confirmation-message">{{ pending?.message }}</p>
      <div class="confirmation-actions">
        <GradientButton type="ghost" autofocus @click="answer(false)">取消</GradientButton>
        <GradientButton @click="answer(true)">确认</GradientButton>
      </div>
    </dialog>
  </Teleport>
</template>

<style scoped>
.app-confirmation {
  width: min(420px, calc(100% - 32px));
  max-height: calc(100% - 32px);
  border: 1px solid var(--color-border);
  border-radius: var(--radius-lg);
  padding: 20px;
  background: var(--glass-panel);
  color: var(--color-text-1);
  box-shadow: var(--shadow-lg);
}
.app-confirmation::backdrop { background: #0007; }
h2 { margin: 0 0 10px; font-size: 16px; }
p { margin: 0; color: var(--color-text-2); white-space: pre-wrap; overflow-wrap: anywhere; }
.confirmation-actions { display: flex; justify-content: end; gap: 8px; margin-top: 18px; }
</style>
