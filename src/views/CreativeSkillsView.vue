<script setup lang="ts">
import AppSelect from '@/components/common/AppSelect.vue'
import { computed, onMounted, ref, watch } from 'vue'
import { ALL_SKILLS } from '@/skills'
import { useSettingsStore } from '@/stores'

const settings = useSettingsStore()
const titles: Record<string, string> = {
  'ai-short-film-screenwriter': '短片编剧',
  'pipeline-batch': '制作拆批与验收',
  'storyboard-director': '分镜导演',
  'character-consistency': '角色一致性',
  'storyboard-panel': '故事看板',
  'shot-image': '镜头生图',
  'shot-video': '镜头视频',
  'h3-prompt-writing': 'H3 视频提示词',
  'qa-verify': '制作验收'
}
interface LocalSkill { id: string; name: string; description: string; references: string[] }
const local = ref<LocalSkill[]>([])
const localBody = ref('')
const localError = ref('')
const reference = ref('')
async function readLocal(input: Record<string, string> = {}) {
  const url = '/api/dramlo/creative-skills?' + new URLSearchParams(input)
  const response = await fetch(url, { headers: { 'X-Dramlo-UI': '1' } })
  if (!response.ok) throw new Error('无法读取本机 Skill 资料')
  return response.json()
}
onMounted(async () => {
  try { local.value = (await readLocal()).items }
  catch { localError.value = '无法读取本机 Skill 目录' }
})
const catalog = computed(() => {
  const builtins = ALL_SKILLS.map(skill => ({
    ...skill, title: titles[skill.id] ?? skill.name, source: 'Dramlo 内置', references: [] as string[]
  }))
  const bundled = local.value.map(skill => ({ ...skill, title: skill.name, body: '', source: '本机内置' }))
  const imported = settings.config.directorSkill
  return imported
    ? [...builtins, ...bundled, { id: 'studio-imported-director', title: imported.name,
      description: '已导入的导演手册', body: imported.body, source: '用户导入', references: [] as string[] }]
    : [...builtins, ...bundled]
})
const query = ref('')
const active = ref('pipeline-batch')
const feedback = ref('')
const visible = computed(() => {
  const q = query.value.trim().toLowerCase()
  return catalog.value.filter(item => !q || [item.title, item.id, item.description].some(text => text.toLowerCase().includes(q)))
})
const detail = computed(() => visible.value.find(item => item.id === active.value) ?? visible.value[0])
watch([detail, reference], async ([item, selected], _, onCleanup) => {
  localBody.value = ''
  localError.value = ''
  if (item?.source !== '本机内置') return
  let current = true
  onCleanup(() => { current = false })
  try {
    const result = await readLocal({ skillId: item.id, ...(item.references.includes(selected) ? { referencePath: selected } : {}) })
    if (current) localBody.value = result.body
  } catch {
    if (current) localError.value = '无法读取本机 Skill 正文'
  }
})
async function copyId() {
  if (!detail.value) return
  try {
    await navigator.clipboard.writeText(detail.value.id)
    feedback.value = '已复制 Skill ID'
  } catch {
    feedback.value = '请手动复制下方的 Skill ID'
  }
}
</script>

<template>
  <div class="skill-library">
    <header>
      <h1>创作 Skill 库</h1>
      <p>Agent 用 <code>studio_list_skills</code> 查看目录，用 <code>studio_read_skill</code> 按 ID 读取正文。</p>
    </header>
    <div class="library-body">
      <aside aria-label="创作 Skill 目录">
        <input v-model="query" aria-label="搜索创作 Skill" placeholder="搜索名称、ID 或用途" />
        <button v-for="entry in visible" :key="entry.id" type="button" :class="{ active: detail?.id === entry.id }"
          @click="active = entry.id; reference = ''; feedback = ''">
          <strong>{{ entry.title }}</strong><small>{{ entry.source }}</small>
        </button>
        <p v-if="!visible.length">没有匹配的 Skill。</p>
      </aside>
      <article v-if="detail">
        <div class="card-heading">
          <h2>{{ detail.title }}</h2>
          <button type="button" @click="copyId">复制 Skill ID</button>
        </div>
        <code>{{ detail.id }}</code>
        <p>{{ detail.description }}</p>
        <p v-if="feedback" role="status">{{ feedback }}</p>
        <label v-if="detail.references.length">资料
          <AppSelect v-model="reference"><option value="">Skill 正文</option><option v-for="item in detail.references" :key="item">{{ item }}</option></AppSelect>
        </label>
        <p v-if="localError" role="alert">{{ localError }}</p>
        <pre>{{ detail.source === '本机内置' ? localBody : detail.body }}</pre>
      </article>
    </div>
  </div>
</template>

<style scoped>
.skill-library { height: 100%; display: flex; flex-direction: column; min-height: 0; }
header { padding: 18px 22px 12px; }
h1, h2, p { margin: 0; }
h1 { font-size: 20px; }
h2 { font-size: 17px; }
header p, article p, small { color: var(--color-text-3); }
header p { margin-top: 6px; }
.library-body { display: grid; grid-template-columns: minmax(140px, 220px) minmax(0, 1fr); flex: 1; min-height: 0; }
aside, article { overflow: auto; padding: 14px 18px; }
aside { border-right: 1px solid var(--color-border); }
aside input, aside button { width: 100%; }
aside input, button { min-height: 34px; border: 1px solid var(--color-border); border-radius: 6px; padding: 6px 8px; }
aside button { display: flex; align-items: center; justify-content: space-between; gap: 4px; margin-top: 5px; text-align: left; }
aside button.active { background: var(--color-bg-2); border-color: var(--color-accent); }
.card-heading { display: flex; align-items: center; justify-content: space-between; gap: 8px; }
article p { margin-top: 8px; }
pre { white-space: pre-wrap; overflow-wrap: anywhere; font: inherit; line-height: 1.7; }
@media (max-width: 800px) {
  .library-body { grid-template-columns: minmax(114px, 28%) minmax(0, 1fr); }
  aside, article { padding-inline: 8px; }
  aside button { align-items: flex-start; flex-direction: column; }
}
</style>
