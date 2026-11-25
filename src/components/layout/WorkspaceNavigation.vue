<script setup lang="ts">
import { computed } from 'vue'
import { useRoute } from 'vue-router'
import HomeIcon from '@/components/common/HomeIcon.vue'

const route = useRoute()
const links = [
  { id: 'home', label: '首页', icon: 'home', to: { name: 'home' } },
  { id: 'projects', label: '项目管理', icon: 'folder', to: { name: 'home', query: { section: 'projects' } } },
  { id: 'studio', label: '项目画布', icon: 'canvas', to: { name: 'studio' } },
  { id: 'library', label: '素材库', icon: 'library', to: { name: 'home', query: { section: 'library' } } },
  { id: 'scripts', label: '剧本管理', icon: 'script', to: { name: 'scripts' } },
  { id: 'extensions', label: '扩展库', icon: 'wand', to: { name: 'extensions' } },
  { id: 'creative-skills', label: '创作 Skill 库', icon: 'script', to: { name: 'creative-skills' } },
  { id: 'templates', label: '图库模板', icon: 'image', to: { name: 'home', query: { section: 'templates' } } },
  { id: 'tools', label: '工具集', icon: 'wand', to: { name: 'home', query: { section: 'tools' } } },
  { id: 'settings', label: '设置', icon: 'settings', to: { name: 'settings', params: { section: 'external' } } }
] as const
const availableLinks = links.filter(link => link.id !== 'extensions' || window.desktop)
const homeSections = ['projects', 'library', 'templates', 'tools']
const active = computed(() => {
  if (route.name === 'home') {
    const section = route.query.section
    return typeof section === 'string' && homeSections.includes(section) ? section : 'home'
  }
  if (route.name === 'drafts') return 'library'
  if (route.meta.step) return 'studio'
  return route.name
})
</script>

<template>
  <nav class="workspace-nav" aria-label="工作台导航">
    <RouterLink v-for="item in availableLinks" :key="item.id" :to="item.to" custom v-slot="{ href, navigate }">
      <a
        :href="href"
        :aria-current="active === item.id ? 'page' : undefined"
        :class="{ current: active === item.id, 'settings-link': item.id === 'settings' }"
        :aria-label="item.label"
        :title="item.label"
        @click="navigate"
      >
        <HomeIcon :name="item.icon" :size="18" />
        <span>{{ item.label }}</span>
      </a>
    </RouterLink>
  </nav>
</template>

<style scoped>
.workspace-nav {
  display: flex;
  flex: 0 0 144px;
  flex-direction: column;
  gap: 4px;
  padding: 10px 8px;
  overflow: auto;
  background: var(--glass-frost);
  border-right: 1px solid var(--nb-line-1);
}
.workspace-nav a {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 0 10px;
  min-height: 36px;
  border-radius: 7px;
  text-decoration: none;
  white-space: nowrap;
  color: var(--color-text-2);
}
.workspace-nav .settings-link { margin-top: auto; }
.workspace-nav a:focus-visible { outline-offset: -2px; }
.workspace-nav a:hover, .workspace-nav a:focus-visible {
  background: var(--control-hover);
  color: var(--color-text-1);
}
.workspace-nav a.current {
  background: var(--control-selected);
  color: var(--color-text-1);
}
.workspace-nav a:active {
  background: var(--control-pressed);
}
@media (max-width: 960px) {
  .workspace-nav { flex-basis: 56px; padding-inline: 6px; }
  .workspace-nav a { justify-content: center; padding: 0; }
  .workspace-nav span { display: none; }
}
</style>
