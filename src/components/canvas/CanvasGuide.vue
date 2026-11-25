<script setup lang="ts">
import { computed, ref } from 'vue'

const props = defineProps<{
  captureOpen: boolean
  captureWidth: number
  rightWidth: number
}>()

const emit = defineEmits<{ (e: 'done'): void }>()

const steps = [
  {
    title: '连接外部 Agent',
    sub: '在熟悉的 Agent 客户端发指令，在这里看结果',
    items: [
      '在左侧打开「设置 → 外部 Agent 接入」，复制连接说明给 Agent。',
      '首次可以让 Agent 把剧本拆成分镜，写入当前项目，先不生成。',
      '让 Agent 修改具体镜头时，提供镜头号或节点 ID。'
    ]
  },
  {
    title: '在画布上手动制作',
    sub: '选中节点后，在右侧检查并调整内容',
    items: [
      '在空白处右键可以添加节点。拖动节点只调整画布位置。',
      '节点可以单独生成或重跑；连线为下游节点提供参考。',
      '在画布上方可以进入制作流程，右侧可以新建画布或打开项目目录。'
    ]
  }
]

const index = ref(0)
const step = computed(() => steps[index.value])
const highlight = computed(() => index.value === 1 ? {
  left: props.captureOpen ? props.captureWidth + 'px' : '0',
  right: (props.rightWidth + 46) + 'px',
  width: 'auto'
} : null)

function next() {
  if (index.value < steps.length - 1) index.value++
  else finish()
}
function prev() {
  if (index.value > 0) index.value--
}
function finish() {
  emit('done')
}
</script>

<template>
  <div class="guide">
    <template v-if="highlight">
      <div class="hl" :style="highlight">
        <div class="card">
          <div class="step-dot">
            <i v-for="(s, i) in steps" :key="s.title" :class="{ on: i === index, past: i < index }" />
          </div>
          <h3>{{ step.title }}</h3>
          <p class="sub">{{ step.sub }}</p>
          <ul>
            <li v-for="item in step.items" :key="item">{{ item }}</li>
          </ul>
          <div class="foot">
            <span class="count">{{ index + 1 }} / {{ steps.length }}</span>
            <div class="btns">
              <button v-if="index > 0" type="button" class="ghost" @click="prev">上一步</button>
              <button type="button" class="ghost" @click="finish">跳过</button>
              <button type="button" class="primary" @click="next">
                {{ index === steps.length - 1 ? '开始使用' : '下一步' }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </template>
    <div v-else class="card float">
      <div class="step-dot">
        <i v-for="(s, i) in steps" :key="s.title" :class="{ on: i === index, past: i < index }" />
      </div>
      <h3>{{ step.title }}</h3>
      <p class="sub">{{ step.sub }}</p>
      <ul>
        <li v-for="item in step.items" :key="item">{{ item }}</li>
      </ul>
      <div class="foot">
        <span class="count">{{ index + 1 }} / {{ steps.length }}</span>
        <div class="btns">
          <button v-if="index > 0" type="button" class="ghost" @click="prev">上一步</button>
          <button type="button" class="ghost" @click="finish">关闭</button>
          <button type="button" class="primary" @click="next">{{ index === steps.length - 1 ? '开始使用' : '下一步' }}</button>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.guide {
  position: absolute;
  inset: 0;
  z-index: 60;
  pointer-events: none;
}
.hl {
  position: absolute;
  top: 0;
  bottom: 0;
  border-radius: 16px;
  border: 1px solid var(--focus-ring);
  box-shadow: 0 0 0 9999px rgba(6, 7, 10, 0.66);
  pointer-events: auto;
  animation: hl-in 0.28s ease;
}
@keyframes hl-in {
  from {
    opacity: 0.4;
  }
  to {
    opacity: 1;
  }
}
.card {
  position: absolute;
  left: 50%;
  top: 50%;
  transform: translate(-50%, -50%);
  width: min(440px, calc(100% - 40px));
  max-height: calc(100% - 40px);
  overflow: auto;
  padding: 18px 20px 16px;
  border: 1px solid var(--color-border);
  border-radius: 14px;
  background: var(--color-bg-1);
  box-shadow: 0 24px 48px rgba(0, 0, 0, 0.45);
  pointer-events: auto;
  animation: card-in 0.24s ease;
}
.card.float {
  box-shadow: 0 24px 48px rgba(0, 0, 0, 0.55);
}
@keyframes card-in {
  from {
    opacity: 0;
    transform: translate(-50%, calc(-50% + 10px));
  }
  to {
    opacity: 1;
    transform: translate(-50%, -50%);
  }
}
.step-dot {
  display: flex;
  gap: 5px;
  margin-bottom: 11px;
}
.step-dot i {
  width: 18px;
  height: 3px;
  border-radius: 99px;
  background: var(--color-fill-strong);
}
.step-dot i.past {
  background: var(--color-border-strong);
}
.step-dot i.on {
  background: var(--color-accent-2);
}
h3 {
  margin: 0 0 3px;
  font-size: 16px;
  font-weight: 680;
  letter-spacing: -0.01em;
  color: var(--color-text-1);
}
.sub {
  margin: 0 0 12px;
  font-size: 12px;
  color: var(--color-text-4);
}
ul {
  margin: 0 0 16px;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
li {
  position: relative;
  padding-left: 16px;
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--color-text-2);
}
li::before {
  content: '';
  position: absolute;
  left: 3px;
  top: 8px;
  width: 5px;
  height: 5px;
  border-radius: 50%;
  background: var(--color-accent-2);
}
.foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding-top: 12px;
  border-top: 1px solid var(--color-border);
}
.count {
  font-size: 11px;
  color: var(--color-text-4);
  font-variant-numeric: tabular-nums;
}
.btns {
  display: flex;
  gap: 8px;
}
.btns button {
  margin: 0;
  padding: 7px 14px;
  border-radius: 9px;
  font-size: 12px;
  font-weight: 600;
  cursor: pointer;
}
.ghost {
  border: 1px solid var(--color-border);
  background: transparent;
  color: var(--color-text-3);
}
.ghost:hover {
  color: var(--color-text-1);
  border-color: var(--color-border-strong);
}
.primary {
  border: 1px solid var(--focus-ring);
  background: var(--control-selected);
  color: var(--color-text-1);
}
.primary:hover {
  background: var(--control-hover);
}
</style>
