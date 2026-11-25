<script setup lang="ts">
import HomeIcon from './HomeIcon.vue'

defineProps<{
  type?: 'primary' | 'secondary' | 'ghost' | 'danger'
  size?: 'sm' | 'md' | 'lg'
  loading?: boolean
  disabled?: boolean
  block?: boolean
  icon?: InstanceType<typeof HomeIcon>['$props']['name']
}>()

const emit = defineEmits<{
  click: [evt: MouseEvent]
}>()
</script>

<template>
  <button
    class="grad-btn"
    :class="[
      `type-${type ?? 'primary'}`,
      `size-${size ?? 'md'}`,
      { loading, disabled, block }
    ]"
    :disabled="disabled || loading"
    @click="emit('click', $event)"
  >
    <span v-if="$slots.icon || icon" class="icon">
      <slot name="icon"><HomeIcon v-if="icon" :name="icon" :size="16" /></slot>
    </span>
    <span class="content">
      <slot />
    </span>
    <span v-if="loading" class="spinner spin"></span>
  </button>
</template>

<style scoped>
.grad-btn {
  position: relative;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  font-weight: 600;
  border-radius: var(--radius-md);
  transition: background-color var(--duration-fast) ease, color var(--duration-fast) ease,
    transform var(--duration-fast) ease;
  white-space: nowrap;
  user-select: none;
}

.grad-btn.block {
  width: 100%;
}

.grad-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.size-sm {
  padding: 6px 12px;
  font-size: 12px;
}
.size-md {
  padding: 10px 18px;
  font-size: 13px;
}
.size-lg {
  padding: 14px 28px;
  font-size: 15px;
}

.type-primary {
  background: var(--color-accent);
  color: var(--color-on-accent);
}
.type-primary:hover:not(:disabled) {
  background: var(--glass-primary-hover);
}
.grad-btn:active:not(:disabled) {
  transform: translateY(1px);
}

.type-primary:active:not(:disabled) {
  background: color-mix(in srgb, var(--color-accent) 87%, var(--color-on-accent));
}

.type-secondary {
  background: var(--color-fill);
  color: var(--color-text-1);
  border: 1px solid transparent;
}
.type-secondary:hover:not(:disabled) {
  background: var(--control-hover);
}

.type-ghost {
  background: transparent;
  color: var(--color-text-2);
  border: 1px solid transparent;
}
.type-ghost:hover:not(:disabled) {
  background: var(--control-hover);
  color: var(--color-text-1);
}
.type-secondary:active:not(:disabled),
.type-ghost:active:not(:disabled) {
  background: var(--control-pressed);
}

.type-danger {
  background: var(--nb-danger-soft);
  color: var(--color-error);
  border: 1px solid transparent;
}
.type-danger:hover:not(:disabled),
.type-danger:active:not(:disabled) {
  background: color-mix(in srgb, var(--color-error) 18%, transparent);
}

@media (prefers-reduced-motion: reduce) {
  .grad-btn:active:not(:disabled) { transform: none; }
}

.icon {
  font-size: 1.1em;
  line-height: 1;
}

.spinner {
  width: 14px;
  height: 14px;
  border: 2px solid color-mix(in srgb, currentColor 28%, transparent);
  border-top-color: currentColor;
  border-radius: 50%;
}
.loading .content {
  opacity: 0.5;
}
</style>
