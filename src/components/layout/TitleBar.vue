<script setup lang="ts">
import ThemeToggle from '@/components/common/ThemeToggle.vue'
</script>

<template>
  <header class="titlebar">
    <RouterLink class="brand" :to="{ name: 'home' }">
      <img src="/dramlo.svg" alt="" />
      <span>Dramlo</span>
    </RouterLink>
    <ThemeToggle />
  </header>
</template>

<style scoped>
.titlebar {
  height: 44px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0 14px;
  background: var(--glass-frost);
  border-bottom: 1px solid var(--nb-line-1);
}
.brand {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  color: var(--color-text-1);
  font-weight: 650;
  text-decoration: none;
  border-radius: var(--radius-sm);
}
.brand:hover { background: var(--control-hover); }
.brand:active { background: var(--control-pressed); }
.brand img { width: 20px; height: 20px; object-fit: contain; filter: grayscale(1); }
</style>
