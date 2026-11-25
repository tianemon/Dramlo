<script setup lang="ts">
import HomeIcon from '@/components/common/HomeIcon.vue'
import { computed, ref } from 'vue'
import { useRouter } from 'vue-router'
import { useProjectStore, useToastStore } from '@/stores'
import GradientButton from '@/components/common/GradientButton.vue'

const router = useRouter()
const project = useProjectStore()
const toast = useToastStore()

const editing = ref(false)
const nameInput = ref('')

const projectName = computed(() => project.current?.name ?? '未命名项目')

function startEdit() {
  editing.value = true
  nameInput.value = projectName.value
}
function commitEdit() {
  if (project.current && nameInput.value.trim()) {
    project.current.name = nameInput.value.trim()
    project.current.updatedAt = Date.now()
    project.save()
  }
  editing.value = false
}
function cancelEdit() {
  editing.value = false
}

function newProject() {
  project.createNamed(
    `项目 ${new Date().toLocaleString('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit' })}`
  ).then(() => {
    toast.success('新项目已创建')
    router.push('/studio')
  })
}
</script>

<template>
  <header class="topbar">
    <div class="left">
      <div v-if="editing" class="rename">
        <input
          v-model="nameInput"
          class="name-input"
          @keydown.enter="commitEdit"
          @keydown.escape="cancelEdit"
          @blur="commitEdit"
          autofocus
        />
      </div>
      <button v-else class="project-name" @click="startEdit" title="点击重命名">
        <span>{{ projectName }}</span>
        <span class="pencil"><HomeIcon name="edit" :size="14" /></span>
      </button>

      <span v-if="project.dirty" class="dirty-dot" title="未保存"></span>
      <span v-else-if="project.lastSavedAt" class="saved">已保存</span>
    </div>

    <div class="right">
      <GradientButton size="sm" type="secondary" icon="plus" @click="newProject">
        新项目
      </GradientButton>
    </div>
  </header>
</template>

<style scoped>
.topbar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  height: 56px;
  padding: 0 var(--space-5);
  background: var(--color-bg-1);
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
  z-index: 10;
}
.left,
.right {
  display: flex;
  align-items: center;
  gap: var(--space-3);
}
.project-name {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 6px 10px;
  font-size: 13px;
  color: var(--color-text-2);
  border-radius: var(--radius-sm);
  transition: all var(--duration-fast);
}
.project-name:hover {
  background: var(--color-fill);
  color: var(--color-text-1);
}
.pencil {
  font-size: 11px;
  opacity: 0.4;
}
.rename {
  display: flex;
  align-items: center;
}
.name-input {
  background: var(--color-bg-2);
  border: 1px solid var(--color-accent);
  border-radius: var(--radius-sm);
  padding: 5px 10px;
  font-size: 13px;
  outline: none;
  color: var(--color-text-1);
  min-width: 200px;
}
.dirty-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--color-warning);
  box-shadow: 0 0 6px var(--color-warning);
}
.saved {
  font-size: 11px;
  color: var(--color-text-4);
}

</style>
