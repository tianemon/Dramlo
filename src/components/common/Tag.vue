<script setup lang="ts">
defineProps<{
  /** 标签类型 */
  type?: 'default' | 'success' | 'warning' | 'error' | 'info' | 'accent'
  /** 尺寸 */
  size?: 'sm' | 'md'
}>()
</script>

<template>
  <span class="tag" :class="[`type-${type ?? 'default'}`, `size-${size ?? 'md'}`]">
    <slot />
  </span>
</template>

<style scoped>
.tag {
  display: inline-flex;
  align-items: center;
  padding: 2px 8px;
  border-radius: var(--radius-full);
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.3px;
  line-height: 1.6;
  white-space: nowrap;
}
.size-sm {
  padding: 1px 6px;
  font-size: 10px;
}
.type-default {
  background: var(--color-fill);
  color: var(--color-text-2);
}
.type-success {
  background: var(--nb-success-soft);
  color: var(--color-success);
}
.type-warning {
  background: var(--nb-warning-soft);
  color: var(--color-warning);
}
.type-error {
  background: var(--nb-danger-soft);
  color: var(--color-error);
}
.type-info {
  background: var(--color-fill-hover);
  color: var(--color-info);
}
.type-accent {
  background: var(--color-fill-strong);
  color: var(--color-text-1);
}
</style>
