<script setup lang="ts">
import { onMounted, onUnmounted, computed, watch, ref } from 'vue'
import { desktop, isDesktop } from '@/services/desktop'
import { useRoute } from 'vue-router'
import { useProjectStore, useAssetsStore, useSettingsStore } from '@/stores'
import AppLayout from '@/components/layout/AppLayout.vue'
import WorkspaceNavigation from '@/components/layout/WorkspaceNavigation.vue'
import TitleBar from '@/components/layout/TitleBar.vue'
import ToastContainer from '@/components/common/ToastContainer.vue'
import ImageLightbox from '@/components/common/ImageLightbox.vue'
import ExternalAgentSetup from '@/components/settings/ExternalAgentSetup.vue'
import NoticeBanner from '@/components/common/NoticeBanner.vue'
import AppConfirmation from '@/components/common/AppConfirmation.vue'

const route = useRoute()
const introduction = ref<HTMLDialogElement | null>(null)
const introOpen = ref(false)
const hideIntroduction = ref(false)
const introKey = 'dramlo:web-agent-intro:v1'
let introOffered = false
watch([() => route.name, introduction], ([name, dialog]) => {
  if (introOffered || name !== 'home' || !dialog || localStorage.getItem(introKey) === 'hidden') return
  introOffered = true
  introOpen.value = true
  dialog.showModal()
}, { flush: 'post' })
function dismissIntroduction() {
  if (hideIntroduction.value) localStorage.setItem(introKey, 'hidden')
  introOpen.value = false
  introduction.value?.close()
}

const project = useProjectStore()
const assets = useAssetsStore()
const settings = useSettingsStore()

const showLayout = computed(
  () =>
    route.name !== 'home' &&
    route.name !== 'studio' &&
    route.name !== 'edit' &&
    route.name !== 'smart-speech' &&
    route.name !== 'scripts' &&
    route.name !== 'creative-skills' &&
    route.name !== 'extensions' &&
    route.name !== 'settings'
)

watch(
  () => settings.config.theme,
  (theme) => {
    document.documentElement.dataset.theme = theme === 'light' ? 'light' : 'dark'
  },
  { immediate: true }
)

type ProjectRevision = { projectId: string; updatedAt: number }
let syncing = false
let events: EventSource | undefined
let syncTimer: ReturnType<typeof setInterval> | undefined
let retryTimer: ReturnType<typeof setTimeout> | undefined
let pending: ProjectRevision | null = null
async function syncLocalProject() {
  const selected = pending
  if (!selected || syncing || project.dirty) return
  if (selected.projectId === project.current?.id && selected.updatedAt === project.current.updatedAt) return
  const currentId = project.current?.id
  syncing = true
  const shouldApply = () => pending?.projectId === selected.projectId && !project.dirty && project.current?.id === currentId
  try {
    await project.syncFromService(selected.projectId, shouldApply)
    if (project.current?.id === selected.projectId && !project.dirty) await assets.refresh()
  } catch {
    if (retryTimer === undefined) {
      retryTimer = setTimeout(() => {
        retryTimer = undefined
        void syncLocalProject()
      }, 750)
    }
  } finally {
    syncing = false
    if (pending && pending !== selected) void syncLocalProject()
  }
}

async function pollRevision() {
  try {
    pending = await desktop().getActiveProject?.() ?? null
    await syncLocalProject()
  } catch {
    // The service might be restarting.
  }
}

function watchProjectRevisions() {
  events = new EventSource('/api/dramlo/project-events')
  events.addEventListener('project', (message: MessageEvent<string>) => {
    clearInterval(syncTimer)
    syncTimer = undefined
    try {
      pending = JSON.parse(message.data)
      void syncLocalProject()
    } catch {
      // The next server event or reconnect provides a fresh snapshot.
    }
  })
  events.addEventListener('error', () => {
    if (syncTimer === undefined) syncTimer = setInterval(pollRevision, 1500)
  })
}
watch(() => project.dirty, (dirty) => { if (!dirty && isDesktop()) void pollRevision() })

onMounted(async () => {
  await settings.ensureHydrated()
  await project.ensureLoaded()
  await assets.refresh()
  project.startAutoSave()
  if (isDesktop()) watchProjectRevisions()
})
onUnmounted(() => {
  events?.close()
  clearInterval(syncTimer)
  clearTimeout(retryTimer)
})
</script>

<template>
  <div class="app-root">
    <TitleBar />
    <NoticeBanner />
    <ToastContainer />
    <ImageLightbox />
    <AppConfirmation />
    <dialog v-if="isDesktop()" ref="introduction" class="agent-intro"
      aria-label="首次接入 Dramlo" @cancel.prevent="dismissIntroduction">
      <ExternalAgentSetup v-if="introOpen" intro />
      <form class="intro-footer" @submit.prevent="dismissIntroduction">
        <label><input v-model="hideIntroduction" type="checkbox" /> 不再显示</label>
        <button type="submit">进入工作台</button>
      </form>
    </dialog>
    <div class="app-body">
      <WorkspaceNavigation />
      <div class="page">
        <template v-if="showLayout">
          <AppLayout>
            <router-view v-slot="{ Component }">
              <transition name="fade" mode="out-in">
                <component :is="Component" />
              </transition>
            </router-view>
          </AppLayout>
        </template>
        <template v-else>
          <router-view />
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.app-root {
  height: 100%;
  display: flex;
  flex-direction: column;
  min-height: 0;
}
.app-body {
  display: flex;
  flex: 1;
  min-height: 0;
}
.page {
  flex: 1;
  min-width: 0;
  min-height: 0;
  position: relative;
  overflow: hidden;
}

.agent-intro {
  width: min(640px, calc(100% - 32px));
  max-height: calc(100% - 32px);
  padding: 16px;
  border: 1px solid var(--color-border);
  border-radius: 12px;
  background: var(--color-bg-1);
  color: var(--color-text-1);
}
.agent-intro::backdrop { background: rgba(0, 0, 0, .55); }
.intro-footer { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-top: 16px; }
.intro-footer label { display: flex; align-items: center; gap: 6px; }
.intro-footer button { padding: 8px 12px; border: 1px solid var(--color-accent); border-radius: 6px; }
</style>

