<script setup lang="ts">
import { confirmAction } from '@/composables/useAppConfirmation'
import HomeIcon from '@/components/common/HomeIcon.vue'
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter, RouterLink } from 'vue-router'
import { useProjectStore, useSettingsStore, useAssetsStore, useToastStore, endpointReady } from '@/stores'
import StudioSettings from '@/components/canvas/StudioSettings.vue'
import ExternalAgentSetup from '@/components/settings/ExternalAgentSetup.vue'
import ZealmanSettings from '@/components/settings/ZealmanSettings.vue'
import ComfySettings from '@/components/settings/ComfySettings.vue'
import AutodlSettings from '@/components/settings/AutodlSettings.vue'
import Tag from '@/components/common/Tag.vue'
import { estimateStorage } from '@/services/storage'
import { isFFmpegReady, getFFmpeg, resetFFmpeg } from '@/services/ffmpeg'
import { desktop, isDesktop } from '@/services/desktop'
import {
  parseDirectorSkill,
  pickDirectorSkillFile,
  pickDirectorSkillFolder
} from '@/services/settings/directorSkill'

const router = useRouter()
const route = useRoute()
const localWeb = Boolean(window.desktop?.isLocalWeb)
const project = useProjectStore()
const settings = useSettingsStore()
const assets = useAssetsStore()
const toast = useToastStore()

const storage = ref<{ usage: number; quota: number }>({ usage: 0, quota: 0 })
const ffmpegReady = ref(isFFmpegReady())
const ffmpegLoading = ref(false)
const grantingAccess = ref(false)
const storageRoot = ref('')
const changingStorageRoot = ref(false)
const appVersion = ref('0.1.5')

const diag = ref({
  crossOriginIsolated: false,
  sharedArrayBuffer: false,
  userAgent: '',
  localCore: false,
  jsDelivrReachable: false,
  unpkgReachable: false,
  npmmirrorReachable: false
})

const saving = ref(false)
const importingSkill = ref(false)
const directorSkill = computed(() => settings.config.directorSkill)
const directorPreview = computed(() => {
  const body = directorSkill.value?.body || ''
  return body.length > 180 ? `${body.slice(0, 180)}…` : body
})

async function saveDirectorFile(picked: { text: string; filename: string } | null) {
  if (!picked) return
  const doc = parseDirectorSkill(picked.text, picked.filename)
  settings.update({ directorSkill: doc })
  await settings.flush()
  toast.success(`已导入导演手册「${doc.name}」。拆分镜头和写提示词会按它来。`)
}

async function importDirectorSkill() {
  importingSkill.value = true
  try {
    await saveDirectorFile(await pickDirectorSkillFile())
  } catch (err: any) {
    toast.error(err?.message || '导入手册失败')
  } finally {
    importingSkill.value = false
  }
}

async function importDirectorFolder() {
  importingSkill.value = true
  try {
    await saveDirectorFile(await pickDirectorSkillFolder())
  } catch (err: any) {
    toast.error(err?.message || '导入手册失败')
  } finally {
    importingSkill.value = false
  }
}

async function clearDirectorSkill() {
  settings.update({ directorSkill: null })
  await settings.flush()
  toast.success('已清除导演手册，恢复内置分镜写法')
}

onMounted(async () => {
  await settings.ensureHydrated()
  storage.value = await estimateStorage()
  await assets.refresh()
  if (isDesktop()) {
    try {
      storageRoot.value = await desktop().getRoot()
      appVersion.value = await desktop().getVersion()
    } catch {
      /* desktop info is non-critical */
    }
  }
  if (section.value === 'system') await runDiagnostics()
})

const comfySettings = ref<InstanceType<typeof ComfySettings> | null>(null)

async function saveSettings() {
  if (saving.value) return
  saving.value = true
  try {
    if (section.value === 'comfyui' && comfySettings.value && !comfySettings.value.savePending()) {
      toast.warn('工作流尚未保存，请查看 ComfyUI 页面上的提示。')
      return
    }
    await settings.flush()
    toast.success('配置已保存到本机')
  } catch (e: any) {
    toast.error(e?.message || '保存失败，请再试一次')
  } finally {
    saving.value = false
  }
}

async function openSettingsFolder() {
  if (!isDesktop()) return
  try {
    const root = await desktop().getRoot()
    await desktop().openPath(root)
  } catch {
    toast.error('打不开配置目录')
  }
}

async function chooseStorageFolder() {
  if (!isDesktop() || changingStorageRoot.value) return
  try {
    const selected = await desktop().pickDirectory()
    if (!selected || selected === storageRoot.value) return
    const ok = await confirmAction(
      `要把项目、素材和配置迁移到这里吗？\n\n${selected}\n\n现有资料会复制到新目录，完成后应用将自动刷新。`
    )
    if (!ok) return
    changingStorageRoot.value = true
    toast.info('正在迁移项目与素材，请不要关闭应用…', 10000)
    storageRoot.value = await desktop().setRoot(selected)
    toast.success('保存位置已修改，正在重新载入…')
    setTimeout(() => window.location.reload(), 700)
  } catch (err: any) {
    toast.error(err?.message || '修改保存位置失败', 8000)
  } finally {
    changingStorageRoot.value = false
  }
}

async function grantFileAccess() {
  if (!isDesktop()) return
  grantingAccess.value = true
  try {
    await desktop().requestFileAccess()
    toast.success('已授权。请完全退出后重新打开「Dramlo」，再试上传参考图。')
  } catch (err: any) {
    toast.error(err?.message || '授权未完成。可用 360 / 火绒把本程序加到信任区。')
  } finally {
    grantingAccess.value = false
  }
}

async function openWindowsSecurity() {
  if (!isDesktop()) return
  try {
    await desktop().openWindowsSecurity()
  } catch {
    toast.error('打不开 Windows 安全中心')
  }
}

async function runDiagnostics() {
  diag.value.userAgent = navigator.userAgent
  diag.value.crossOriginIsolated = self.crossOriginIsolated
  diag.value.sharedArrayBuffer = typeof SharedArrayBuffer !== 'undefined'
  try {
    const r = await fetch('/ffmpeg/ffmpeg-core.js', { method: 'HEAD' })
    diag.value.localCore = r.ok
  } catch {
    diag.value.localCore = false
  }
  diag.value.jsDelivrReachable = await ping('https://cdn.jsdelivr.net/npm/@ffmpeg/core@0.12.6/dist/umd/ffmpeg-core.js')
  diag.value.unpkgReachable = await ping('https://unpkg.com/@ffmpeg/core@0.12.6/dist/umd/ffmpeg-core.js')
  diag.value.npmmirrorReachable = await ping('https://registry.npmmirror.com/@ffmpeg/core@0.12.6/files/dist/umd/ffmpeg-core.js')
}

async function ping(url: string): Promise<boolean> {
  try {
    const ctl = new AbortController()
    const timer = setTimeout(() => ctl.abort(), 5000)
    const r = await fetch(url, { method: 'GET', headers: { Range: 'bytes=0-0' }, signal: ctl.signal })
    clearTimeout(timer)
    try { await r.body?.cancel() } catch {}
    return r.ok || r.status === 206
  } catch {
    return false
  }
}

async function warmupFFmpeg() {
  ffmpegLoading.value = true
  try {
    await getFFmpeg((msg) => toast.info(msg, 1500))
    ffmpegReady.value = true
    toast.success('FFmpeg 已就绪')
    await runDiagnostics()
  } catch (e: any) {
    toast.error('FFmpeg 加载失败：' + e.message, 8000)
  } finally {
    ffmpegLoading.value = false
  }
}

function resetAndRetry() {
  resetFFmpeg()
  ffmpegReady.value = false
  toast.info('已重置，请重新点击「预加载」')
  runDiagnostics()
}

const workspaceBusy = ref(false)

async function exportProjectData() {
  if (workspaceBusy.value) return
  workspaceBusy.value = true
  try {
    if (project.dirty) await project.save()
    const link = document.createElement('a')
    link.href = '/api/dramlo/workspace-backup'
    link.download = 'studio-projects.zip'
    link.click()
  } catch (error) {
    toast.error(error instanceof Error ? error.message : '准备备份失败')
  } finally {
    workspaceBusy.value = false
  }
}

function importProjectData() {
  if (workspaceBusy.value) return
  const input = document.createElement('input')
  input.type = 'file'
  input.accept = '.zip'
  input.onchange = async () => {
    const file = input.files?.[0]
    if (!file || workspaceBusy.value) return
    workspaceBusy.value = true
    try {
      if (project.dirty) await project.save()
      const response = await fetch('/api/dramlo/workspace-backup', {
        method: 'POST', headers: { 'X-Dramlo-UI': '1', 'Content-Type': 'application/zip' }, body: file
      })
      const result = await response.json()
      if (!response.ok) throw new Error(result.error || '无法恢复备份')
      await project.refreshList()
      toast.success(`已恢复 ${result.imported} 个项目`)
    } catch (error) {
      toast.error(error instanceof Error ? error.message : '导入失败')
    } finally {
      workspaceBusy.value = false
    }
  }
  input.click()
}

async function clearAll() {
  if (workspaceBusy.value || !await confirmAction('移除所有项目及其素材？本机工作台会将已移除项目移到可恢复目录。')) return
  let removed = 0
  workspaceBusy.value = true
  try {
    if (project.dirty) await project.save()
    await project.refreshList()
    for (const item of [...project.allProjects]) {
      await project.remove(item.id)
      removed++
    }
    toast.success(`已移除 ${removed} 个项目`)
  } catch (error) {
    toast.error(`已移除 ${removed} 个项目。${error instanceof Error ? error.message : '其余项目未移除'}`)
  } finally {
    workspaceBusy.value = false
  }
}

function formatSize(bytes: number) {
  if (!bytes) return '0 B'
  const units = ['B', 'KB', 'MB', 'GB']
  let i = 0
  let b = bytes
  while (b >= 1024 && i < units.length - 1) {
    b /= 1024
    i++
  }
  return `${b.toFixed(1)} ${units[i]}`
}

const assetStats = computed(() => {
  const stats = { image: 0, video: 0, audio: 0, total: 0 }
  for (const a of assets.all) {
    if (a.kind === 'image') stats.image++
    if (a.kind === 'video') stats.video++
    if (a.kind === 'audio') stats.audio++
    stats.total++
  }
  return stats
})

const storagePct = computed(() => {
  if (!storage.value.quota) return 0
  return (storage.value.usage / storage.value.quota) * 100
})

type SettingsSection = 'llm' | 'vision' | 'image' | 'tts' | 'video' | 'comfyui' | 'autodl' | 'zealman' | 'external' | 'system'
const SECTIONS: SettingsSection[] = ['llm', 'vision', 'image', 'tts', 'video', 'comfyui', 'autodl', 'zealman', 'external', 'system']

const nav = [
  {
    id: 'llm' as const,
    label: '语言模型',
    desc: '手动调用的创作模型',
    lead: '使用外部 Agent 撰写、拆分镜或写提示词时，可以跳过此页。只有在 Dramlo 里直接使用 AI 写剧本、生成分镜或优化提示词时才需要配置。'
  },
  {
    id: 'vision' as const,
    label: '视觉理解',
    desc: '看图与质检',
    lead: '供 Dramlo 内的参考图分析和生成质检使用。由外部 Agent 看图和审校时，可以跳过此页。'
  },
  {
    id: 'image' as const,
    label: '图像生成',
    desc: '云端生图 / 本机 ComfyUI',
    lead: '负责画图的模型。人物设定图、场景底板、故事看板、分镜关键帧都用它生成。Zealman 镜像的地址在「Zealman 镜像」填写，这里选中后可以直接出图。'
  },
  {
    id: 'tts' as const,
    label: '配音',
    desc: '后期口播 TTS',
    lead: '给片子配人声。这里选好音色，台词会按照你写的文案念出来，也可以在角色卡上为每个角色单独设声音。'
  },
  {
    id: 'video' as const,
    label: '视频',
    desc: '图生视频',
    lead: '让静止的画面动起来。这里填图生视频的接口地址、凭证和模型（或工作流）名称。'
  },
  {
    id: 'comfyui' as const,
    label: 'ComfyUI 接入',
    desc: '导入和管理工作流',
    lead: '填写 ComfyUI 地址，导入或拉取工作流，确认提示词、图片、分辨率与时长入口。'
  },
  {
    id: 'autodl' as const,
    label: 'AutoDL 算力',
    desc: 'H3 系列图生视频',
    lead: 'H3 系列（15 秒 / v5 1080p / 首尾帧 / 数字人口播）都走 AutoDL。只需填一个 AutoDL Token，内置工作流自动配好。'
  },
  {
    id: 'zealman' as const,
    label: 'Zealman 镜像',
    desc: '生图和视频只填地址',
    lead: 'Zealman 镜像的文生图和视频都在这一页填写 Base URL。填好后到图像生成、视频里选用对应模型即可。'
  },
  {
    id: 'external' as const,
    label: '外部 Agent 接入',
    desc: 'MCP 与 Skill',
    lead: '把 Dramlo 接入你正在使用的 Agent，或发现可导入的本机 ComfyUI 工作流。'
  },
  {
    id: 'system' as const,
    label: '系统',
    desc: '引擎、存储、导演手册',
    lead: '查看剪辑引擎、存储位置与版本，备份或导入项目。'
  }
]

const settingItems = (ids: SettingsSection[]) => ids.flatMap(id => nav.filter(item => item.id === id))
const primaryItems = settingItems(['external', 'comfyui', 'image', 'video', 'tts', 'system'])
const advancedSections = ['llm', 'vision', 'autodl', 'zealman'] as const
const advancedItems = settingItems([...advancedSections])
const isAdvanced = (id: SettingsSection) => advancedItems.some(item => item.id === id)

const section = computed<SettingsSection>(() => {
  const fallback: SettingsSection = 'external'
  const raw = String(route.params.section || fallback)
  return SECTIONS.includes(raw as SettingsSection) ? (raw as SettingsSection) : fallback
})

const currentNav = computed(() => nav.find((n) => n.id === section.value) || nav[0])
const advancedOpen = ref(isAdvanced(section.value))
watch(section, (next) => {
  if (isAdvanced(next)) advancedOpen.value = true
  if (next === 'system') void runDiagnostics()
})

function go(id: SettingsSection) {
  router.replace(`/settings/${id}`)
}

function goBack() {
  if (window.history.length > 1) router.back()
  else router.push('/')
}

function navReady(id: SettingsSection) {
  const c = settings.config
  if (id === 'llm') return endpointReady(c.llm)
  if (id === 'vision') return endpointReady(c.vision)
  if (id === 'image') return endpointReady(c.image)
  if (id === 'tts') return endpointReady(c.tts)
  if (id === 'video') return endpointReady(c.video)
  if (id === 'autodl') {
    return Boolean(c.autodlToken?.trim()) &&
      c.videos.some((item) => item.provider === 'autodl' && item.apiKey?.trim() === c.autodlToken.trim())
  }
  if (id === 'zealman') return c.videos.some((item) => item.provider === 'zealman' && Boolean(item.baseUrl))
  return false
}
</script>

<template>
  <div class="settings">
    <header class="top">
      <button type="button" class="back" @click="goBack">返回</button>
      <div class="brand">
        <span class="crumb">设置</span>
        <h1>{{ currentNav.label }}</h1>
      </div>
    </header>

    <div class="body">
      <nav class="side" aria-label="设置分区">
        <p class="group">接入与生成</p>
        <RouterLink
          v-for="item in primaryItems"
          :key="item.id"
          :to="`/settings/${item.id}`"
          replace
          :class="{ on: section === item.id }"
        >
          <span class="lab">{{ item.label }}</span>
          <span v-if="item.id !== 'external' && item.id !== 'system'" class="state" :class="{ ok: navReady(item.id) }">
            {{ navReady(item.id) ? '已配置' : '待配置' }}
          </span>
        </RouterLink>
        <details class="advanced-settings" :open="advancedOpen" @toggle="advancedOpen = ($event.currentTarget as HTMLDetailsElement).open">
          <summary><HomeIcon :name="advancedOpen ? 'chevronDown' : 'chevronRight'" :size="14" />可选模型与供应商</summary>
          <RouterLink
            v-for="item in advancedItems"
            :key="item.id"
            :to="`/settings/${item.id}`"
            replace
            :class="{ on: section === item.id }"
          >
            <span class="lab">{{ item.label }}</span>
            <span class="state" :class="{ ok: navReady(item.id) }">{{ navReady(item.id) ? '已配置' : '待配置' }}</span>
          </RouterLink>
        </details>
      </nav>

      <main class="main">
        <p v-if="section !== 'external'" class="page-lead">
          {{ currentNav.lead }}
        </p>
        <ExternalAgentSetup v-if="section === 'external'" />
        <AutodlSettings v-if="settings.diskReady && section === 'autodl'" />
        <ZealmanSettings v-if="settings.diskReady && section === 'zealman'" />
        <ComfySettings v-if="settings.diskReady && section === 'comfyui'" ref="comfySettings" />
        <StudioSettings
          v-if="settings.diskReady && section !== 'zealman' && section !== 'autodl' && section !== 'system' && section !== 'comfyui' && section !== 'external'"
          :module="section"
        />
        <p v-else-if="!settings.diskReady && section !== 'system' && section !== 'external'" class="loading">
          正在读取本机配置…
        </p>

        <div v-else-if="section === 'system'" class="sys">
          <section class="panel">
            <h2>外观</h2>
            <p class="card-desc">暖白与微冷黑灰共用一致的控件反馈。外观设置应用到整个工作台。</p>
            <div class="theme-choices">
              <button type="button" :class="{ on: settings.config.theme !== 'light' }" @click="settings.update({ theme: 'dark' })">
                <strong>黑灰</strong>
                <span>微冷底色，中性灰控件</span>
              </button>
              <button type="button" :class="{ on: settings.config.theme === 'light' }" @click="settings.update({ theme: 'light' })">
                <strong>暖白</strong>
                <span>纸白外壳，雾灰画布</span>
              </button>
            </div>
          </section>
          <section class="panel">
            <h2>导演手册</h2>
            <p class="card-desc">
              导入一份 SKILL.md 或 README.md。之后拆分镜头、写画面提示词和视频提示词都按这份手册来。没有导入时仍用内置分镜导演手册。
            </p>
            <p v-if="directorSkill" class="path">{{ directorSkill.name }}<span v-if="directorSkill.filename"> · {{ directorSkill.filename }}</span></p>
            <p v-else class="muted">还没有导入。支持带前言的 SKILL，也支持普通 README。</p>
            <p v-if="directorSkill" class="muted small skill-preview">{{ directorPreview }}</p>
            <div class="acts">
              <button type="button" class="mini" :disabled="importingSkill" @click="importDirectorSkill">
                {{ importingSkill ? '正在读取…' : directorSkill ? '更换手册' : '导入手册' }}
              </button>
              <button v-if="isDesktop() && !localWeb" type="button" class="mini" :disabled="importingSkill" @click="importDirectorFolder">
                导入文件夹
              </button>
              <button v-if="directorSkill" type="button" class="link" @click="clearDirectorSkill">清除</button>
            </div>
          </section>

          <section v-if="isDesktop() && !localWeb && desktop().platform === 'win32'" class="panel">
            <h2>本机文件权限</h2>
            <p class="card-desc">
              选参考图、视频素材时走的是系统文件框。正常情况下不需要额外授权；如果被系统拦住了，点下面的按钮放行一次。
            </p>
            <p class="muted">
              放行后请把「Dramlo」和你放素材的文件夹加入安全中心的排除项；用 360 / 火绒的话，把它加到信任区。
            </p>
            <div class="acts">
              <button type="button" class="mini" :disabled="grantingAccess" @click="grantFileAccess">
                {{ grantingAccess ? '等待系统授权…' : '授权访问本机文件' }}
              </button>
              <button type="button" class="mini" @click="openWindowsSecurity">打开 Windows 安全中心</button>
            </div>
          </section>

          <section class="panel">
            <h2>剪辑引擎</h2>
            <p class="card-desc">
              素材拼接、转场、烧字幕和成片导出都靠它。首次用到时会自动下载约 30MB 的组件，也可以现在就加载。
            </p>
            <div class="engine-row">
              <div>
                <div class="engine-name">FFmpeg</div>
                <p class="muted small">状态：{{ ffmpegReady ? '可以开始剪辑' : '还没加载' }}</p>
              </div>
              <button
                v-if="!ffmpegReady"
                type="button"
                class="mini"
                :disabled="ffmpegLoading"
                @click="warmupFFmpeg"
              >
                {{ ffmpegLoading ? '加载中…' : '现在加载' }}
              </button>
              <button v-else type="button" class="link" @click="resetAndRetry">重置</button>
            </div>
            <details class="diag">
              <summary>检测结果（加载不出来时看这里）</summary>
              <div class="diag-row">
                <span>浏览器多线程模式<i>加载失败通常是这项没开启</i></span>
                <Tag :type="diag.crossOriginIsolated ? 'success' : 'error'" size="sm">{{ diag.crossOriginIsolated ? '已开启' : '未开启' }}</Tag>
              </div>
              <div class="diag-row">
                <span>共享内存<i>多线程依赖它</i></span>
                <Tag :type="diag.sharedArrayBuffer ? 'success' : 'error'" size="sm">{{ diag.sharedArrayBuffer ? '支持' : '不支持' }}</Tag>
              </div>
              <div class="diag-row">
                <span>本机组件<i>有它可以离线加载</i></span>
                <Tag :type="diag.localCore ? 'success' : 'warning'" size="sm">{{ diag.localCore ? '已有' : '没有，会从网络下载' }}</Tag>
              </div>
              <div class="diag-row">
                <span>CDN 线路一<i>jsDelivr</i></span>
                <Tag :type="diag.jsDelivrReachable ? 'success' : 'error'" size="sm">{{ diag.jsDelivrReachable ? '通畅' : '连不上' }}</Tag>
              </div>
              <div class="diag-row">
                <span>CDN 线路二<i>unpkg</i></span>
                <Tag :type="diag.unpkgReachable ? 'success' : 'error'" size="sm">{{ diag.unpkgReachable ? '通畅' : '连不上' }}</Tag>
              </div>
              <div class="diag-row">
                <span>国内镜像<i>npmmirror</i></span>
                <Tag :type="diag.npmmirrorReachable ? 'success' : 'error'" size="sm">{{ diag.npmmirrorReachable ? '通畅' : '连不上' }}</Tag>
              </div>
              <button type="button" class="link" @click="runDiagnostics">重新检测</button>
            </details>
          </section>

          <section class="panel">
            <h2>模型配置存在哪</h2>
            <p class="card-desc">
              你填的接口地址和密钥只存在这台电脑里，不会上传到任何服务器。想备份就去下面的复制这个文件夹。
            </p>
            <p v-if="settings.persistPath" class="path">{{ settings.persistPath }}</p>
            <p v-else class="muted">浏览器版保存在浏览器自己的本地存储中。</p>
          </section>

          <section class="panel">
            <h2>保存位置</h2>
            <p class="card-desc">
              项目文件、图片、视频和音频都保存在工作区。
              <template v-if="localWeb">工作区位置由本机启动配置指定。迁移项目可以使用下方的 ZIP 备份。</template>
            </p>
            <p v-if="isDesktop()" class="path storage-path">{{ storageRoot || '正在读取…' }}</p>
            <p v-else class="muted">浏览器版由浏览器自己管理存储位置。</p>
            <div v-if="isDesktop()" class="acts storage-actions">
              <button type="button" class="mini" @click="openSettingsFolder">打开这个文件夹</button>
              <button
                type="button"
                class="mini"
                v-if="!localWeb"
                :disabled="changingStorageRoot"
                @click="chooseStorageFolder"
              >
                {{ changingStorageRoot ? '正在搬家…' : '换一个保存位置' }}
              </button>
            </div>
            <div class="storage-bar"><div class="storage-bar-fill" :style="{ width: storagePct + '%' }" /></div>
            <div class="storage-info">
              <span>已用 {{ formatSize(storage.usage) }} / 总配额 {{ formatSize(storage.quota) }}</span>
              <span>{{ storagePct.toFixed(1) }}%</span>
            </div>
            <div class="ss">
              <div><b>{{ assetStats.image }}</b><span>图片</span></div>
              <div><b>{{ assetStats.video }}</b><span>视频</span></div>
              <div><b>{{ assetStats.audio }}</b><span>音频</span></div>
              <div><b>{{ assetStats.total }}</b><span>合计</span></div>
            </div>
          </section>

          <section v-if="localWeb" class="panel">
            <h2>备份与清理</h2>
            <p class="card-desc">
              ZIP 备份包含项目、素材和项目文档。供应商配置、公共素材库和剧本库独立保存。恢复时遇到同 ID 项目会停止。
            </p>
            <div class="acts">
              <button type="button" class="mini" :disabled="workspaceBusy" @click="exportProjectData">导出所有项目</button>
              <button type="button" class="mini" :disabled="workspaceBusy" @click="importProjectData">从文件导入</button>
              <button type="button" class="danger" :disabled="workspaceBusy" @click="clearAll">移除所有项目</button>
            </div>
          </section>

          <section class="panel">
            <h2>关于</h2>
            <div class="about-row"><span>版本</span><b>v{{ appVersion }}</b></div>
          </section>
        </div>
      </main>
    </div>
    <footer v-if="section !== 'system' && section !== 'external'" class="save-bar">
      <span>所有改动都保存在这台电脑上，换台机器不会同步。改完记得点保存。</span>
      <button type="button" class="save" :disabled="saving || !settings.diskReady" @click="saveSettings">
        {{ saving ? '保存中…' : '保存改动' }}
      </button>
    </footer>
  </div>
</template>

<style scoped>
.settings {
  height: 100%;
  display: flex;
  flex-direction: column;
  min-height: 0;
  background: transparent;
  color: var(--nb-t1);
}
.top {
  display: flex;
  align-items: center;
  gap: 14px;
  min-height: 60px;
  padding: 0 20px;
  border-bottom: 1px solid var(--color-border);
  background: var(--nb-panel-sheen), var(--glass-frost);
  box-shadow: var(--glass-highlight);
  -webkit-backdrop-filter: var(--glass-filter);
  backdrop-filter: var(--glass-filter);
  flex-shrink: 0;
}
.back {
  flex-shrink: 0;
  margin: 0;
  padding: 6px 12px;
  border: none;
  border-radius: 4px;
  background: transparent;
  color: var(--color-text-2);
  font-size: 13px;
  cursor: pointer;
}
.back:hover {
  color: var(--color-text-1);
  background: var(--color-fill-hover);
}
.brand {
  display: flex;
  align-items: baseline;
  gap: 10px;
  min-width: 0;
}
.brand h1 {
  margin: 0;
  font-size: 19px;
  font-weight: 680;
  letter-spacing: -0.015em;
}
.crumb {
  font-size: 13px;
  color: var(--color-text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.body {
  flex: 1;
  min-height: 0;
  display: flex;
}
.side {
  width: clamp(170px, 22%, 216px);
  flex-shrink: 0;
  border-right: 1px solid var(--color-border);
  padding: 14px 10px;
  overflow-y: auto;
  background: var(--nb-panel-sheen), var(--glass-frost);
  box-shadow: var(--glass-highlight);
}
.advanced-settings { margin-top: 12px; }
.advanced-settings summary {
  display: flex;
  align-items: center;
  gap: 5px;
  list-style: none;
  padding: 8px 10px;
  font-size: 12px;
  color: var(--color-text-3);
  cursor: pointer;
}
.advanced-settings summary::-webkit-details-marker { display: none; }
.advanced-settings[open] summary { margin-bottom: 4px; }
.group {
  margin: 14px 10px 6px;
  font-size: 12px;
  font-weight: 650;
  color: var(--color-text-2);
}
.group:first-child {
  margin-top: 0;
}
.side a {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin: 0 0 2px;
  padding: 8px 10px;
  border: none;
  min-height: 38px;
  border-radius: 8px;
  background: transparent;
  color: var(--color-text-2);
  font-size: 13px;
  text-align: left;
  text-decoration: none;
  transition: background 0.18s ease, color 0.18s ease;
}
.side .lab { white-space: nowrap; }
.side a:hover {
  background: var(--color-fill-hover);
  color: var(--color-text-1);
}
.side a.on {
  background: var(--nb-accent-soft);
  color: var(--color-accent);
  font-weight: 600;
}
.state {
  flex-shrink: 0;
  padding: 2px 6px;
  border-radius: 3px;
  border: none;
  background: var(--color-fill);
  color: var(--color-text-2);
  font-size: 11px;
  font-weight: 500;
}
.state.ok {
  background: var(--nb-success-soft);
  color: var(--color-success);
}
.diag-row span i {
  display: block;
  font-style: normal;
  font-size: 10px;
  color: var(--color-text-4);
  margin-top: 1px;
}
.main {
  flex: 1;
  min-width: 0;
  overflow: auto;
  padding: 24px clamp(16px, 2.8vw, 40px) 40px;
}
.page-lead {
  max-width: 720px;
  margin: 0 0 18px;
  padding: 0;
  border: none;
  background: transparent;
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--color-text-2);
}
.loading {
  margin: 12px 0;
  font-size: 13px;
  color: var(--color-text-4);
}
.save-bar {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 13px 20px;
  border-top: none;
  background: var(--glass-frost-solid);
}
.save-bar span {
  font-size: 12px;
  color: var(--color-text-4);
}
.save {
  margin: 0;
  padding: 8px 16px;
  border: none;
  border-radius: 4px;
  background: var(--color-accent);
  color: var(--color-on-accent);
  font-size: 12.5px;
  font-weight: 650;
  cursor: pointer;
  transition: background 0.18s ease;
}
.save:hover:not(:disabled) {
  background: var(--glass-primary-hover);
}
.save:active:not(:disabled) { background: var(--color-accent-2); }
.save:disabled {
  opacity: 0.5;
  cursor: default;
}
.sys {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(320px, 1fr));
  gap: 14px;
  max-width: 940px;
}
.panel {
  padding: 16px;
  border: 1px solid var(--nb-line-1);
  border-radius: var(--radius-lg);
  background: var(--glass-panel);
  color: var(--nb-t1);
}
.theme-choices {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}
.theme-choices button {
  display: grid;
  gap: 4px;
  padding: 12px;
  border: 1px solid transparent;
  border-radius: 10px;
  background: var(--nb-bg-3);
  color: var(--nb-t2);
  text-align: left;
  cursor: pointer;
}
.theme-choices button strong {
  color: var(--nb-t1);
  font-size: 13px;
}
.theme-choices button span {
  color: var(--nb-t3);
  font-size: 11px;
}
.theme-choices button.on {
  background: var(--control-selected);
  box-shadow: inset 0 0 0 1px var(--nb-line-4);
}
.theme-choices button:hover { background: var(--control-hover); }
.theme-choices button:active { background: var(--control-pressed); }
.panel h2 {
  margin: 0 0 6px;
  font-size: 14px;
  font-weight: 680;
  letter-spacing: -0.01em;
}
.card-desc,
.muted {
  margin: 0 0 12px;
  font-size: 12px;
  color: var(--color-text-3);
  line-height: 1.6;
}
.muted.small {
  margin: 3px 0 0;
  font-size: 11px;
}
.skill-preview {
  white-space: pre-wrap;
  max-height: 4.8em;
  overflow: hidden;
}
.engine-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 12px 14px;
  border: none;
  border-radius: 4px;
  background: var(--nb-bg-inset);
  margin-bottom: 10px;
}
.engine-name {
  font-size: 13px;
  font-weight: 620;
}
.mini,
.danger,
.link {
  margin: 0;
  padding: 7px 12px;
  border-radius: 4px;
  font-size: 12px;
  font-weight: 550;
  cursor: pointer;
  transition: background 0.18s ease, color 0.18s ease;
}
.mini {
  border: none;
  background: var(--color-fill);
  color: var(--color-text-2);
}
.mini:hover:not(:disabled), .link:hover:not(:disabled) {
  background: var(--control-hover);
}
.mini:active:not(:disabled), .link:active:not(:disabled) { background: var(--control-pressed); }
.mini:disabled {
  opacity: 0.5;
  cursor: default;
}
.link {
  color: var(--color-accent);
  background: transparent;
}
.danger {
  border: none;
  background: transparent;
  color: var(--color-error);
}
.danger:hover {
  background: var(--nb-danger-soft);
}
.diag {
  margin-top: 10px;
  font-size: 12px;
}
.diag summary {
  cursor: pointer;
  padding: 6px 0;
  color: var(--color-text-3);
  font-size: 11.5px;
}
.diag-row {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 0;
  border-bottom: none;
  color: var(--color-text-2);
  font-size: 12px;
}
.acts {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.storage-bar {
  height: 6px;
  background: var(--color-bg-2);
  border-radius: 99px;
  overflow: hidden;
  margin: 14px 0 6px;
}
.storage-bar-fill {
  height: 100%;
  background: var(--color-accent-2);
}
.path {
  margin: 0 0 12px;
  padding: 9px 11px;
  font-size: 11px;
  line-height: 1.5;
  word-break: break-all;
  color: var(--color-text-2);
  background: var(--nb-bg-inset);
  border: none;
  border-radius: 4px;
}
.storage-info {
  display: flex;
  justify-content: space-between;
  font-size: 11px;
  color: var(--color-text-4);
  margin-bottom: 12px;
}
.ss {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 8px;
}
.ss div {
  text-align: center;
  padding: 10px 6px;
  border: none;
  border-radius: 4px;
  background: var(--nb-bg-inset);
}
.ss b {
  display: block;
  font-size: 17px;
  font-weight: 680;
}
.ss span {
  font-size: 11px;
  color: var(--color-text-4);
}
.about-row {
  display: flex;
  justify-content: space-between;
  padding: 8px 0;
  border-bottom: none;
  font-size: 12px;
  color: var(--color-text-3);
}
.about-row:last-child {
  border-bottom: none;
}
.about-row b {
  color: var(--color-text-1);
  font-weight: 600;
}
.settings :deep(input),
.settings :deep(.app-select),
.settings :deep(textarea) {
  border-color: var(--color-border);
  border-radius: 8px;
  color: var(--color-text-1);
  outline: 1px solid transparent;
  outline-offset: -1px;
  transition: outline-color 0.2s ease, background-color 0.2s ease;
}
.settings :deep(input:focus),
.settings :deep(.app-select:focus),
.settings :deep(textarea:focus) {
  border-color: transparent;
  outline-color: var(--color-accent-2);
}
.side a:focus-visible, .advanced-settings summary:focus-visible, .back:focus-visible {
  outline: 2px solid var(--color-accent);
  outline-offset: -2px;
}
@media (max-width: 640px) {
  .settings .body { flex-direction: column; }
  .side {
    width: 100%;
    max-height: 28vh;
    border-right: 0;
    border-bottom: 1px solid var(--color-border);
    padding: 8px;
    display: flex;
    flex-wrap: wrap;
    align-content: flex-start;
    gap: 4px;
  }
  .side .group, .side .advanced-settings { flex-basis: 100%; }
  .side .advanced-settings { margin-top: 0; }
  .side a { margin: 0; padding-inline: 8px; }
  .top { min-height: 48px; padding-inline: 10px; }
  .main { padding-inline: 12px; }
  .sys { grid-template-columns: minmax(0, 1fr); }
}
</style>
