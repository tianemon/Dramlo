<script setup lang="ts" generic="Value extends string | number | boolean | null | undefined = string">
import { Comment, computed, nextTick, onBeforeUnmount, ref, shallowRef, useId, watch, type CSSProperties, type VNode } from 'vue'
import HomeIcon from './HomeIcon.vue'

type Choice = {
  value: Value
  label: string
  disabled: boolean
  group?: string
  startsGroup?: boolean
}

const props = withDefaults(defineProps<{
  modelValue?: Value
  value?: Value
  disabled?: boolean
  placeholder?: string
}>(), { disabled: false, placeholder: '选择…' })
const emit = defineEmits<{
  'update:modelValue': [value: Value]
  change: [value: Value]
}>()
const slots = defineSlots<{ default(): VNode[] }>()
const trigger = ref<HTMLButtonElement>()
const popup = ref<HTMLDivElement>()
const portal = shallowRef<string | Element>('body')
const listId = useId()
const open = ref(false)
const activeIndex = ref(-1)
const placement = ref<CSSProperties>({ visibility: 'hidden' })
let intersectionObserver: IntersectionObserver | undefined
let animationFrame = 0
let geometry: number[] = []
let placementDirty = true
let openingId = 0
let typed = ''
let lastTypedAt = 0

function labelFor(node: VNode): string {
  if (typeof node.children === 'string') return node.children
  if (Array.isArray(node.children)) {
    return node.children.map(child => {
      if (typeof child === 'string' || typeof child === 'number') return String(child)
      return child && typeof child === 'object' && 'children' in child ? labelFor(child as VNode) : ''
    }).join('')
  }
  return ''
}

// The option and optgroup slots describe choices. The browser never renders them as controls.
function readChoices(nodes: VNode[], group?: string, disabled = false): Choice[] {
  const result: Choice[] = []
  for (const node of nodes) {
    if (node.type === Comment) continue
    if (node.type === 'option') {
      const label = labelFor(node).trim()
      const value = (node.props && Object.hasOwn(node.props, 'value') ? node.props.value : label) as Value
      result.push({ label, value, group, disabled: disabled || Boolean(node.props?.disabled === '' || node.props?.disabled) })
    } else if (Array.isArray(node.children)) {
      if (node.type === 'optgroup') {
        const members = readChoices(node.children as VNode[], String(node.props?.label ?? ''),
          disabled || Boolean(node.props?.disabled === '' || node.props?.disabled))
        if (members[0]) members[0].startsGroup = true
        result.push(...members)
      } else {
        result.push(...readChoices(node.children as VNode[], group, disabled))
      }
    }
  }
  return result
}

const choices = computed(() => readChoices(slots.default?.() ?? []))
const selectedIndex = computed(() => {
  const value = props.modelValue === undefined ? props.value : props.modelValue
  return choices.value.findIndex(choice => Object.is(choice.value, value))
})
const selectedLabel = computed(() => choices.value[selectedIndex.value]?.label || props.placeholder)
const optionId = (index: number) => `${listId}-${index}`

function enabledFrom(index: number, direction: number, wrap = false) {
  const count = choices.value.length
  for (let step = 0; step < count; step++, index += direction) {
    if (wrap) index = (index + count) % count
    if (index < 0 || index >= count) break
    if (!choices.value[index].disabled) return index
  }
  return -1
}

function revealActive() {
  const menu = popup.value
  const option = menu?.querySelector<HTMLElement>(`[data-option-index="${activeIndex.value}"]`)
  if (!menu || !option) return
  const list = menu.getBoundingClientRect()
  const item = option.getBoundingClientRect()
  if (item.bottom > list.bottom - 3) menu.scrollTop += item.bottom - list.bottom + 3
  if (item.top < list.top + 3) menu.scrollTop -= list.top + 3 - item.top
}

function place() {
  const anchor = trigger.value?.getBoundingClientRect()
  const menu = popup.value
  if (!anchor || !menu) return
  const viewport = window.visualViewport
  const minX = viewport?.offsetLeft ?? 0
  const minY = viewport?.offsetTop ?? 0
  const maxX = minX + (viewport?.width ?? document.documentElement.clientWidth)
  const maxY = minY + (viewport?.height ?? document.documentElement.clientHeight)
  const nextGeometry = [anchor.left, anchor.top, anchor.width, anchor.height, minX, minY, maxX, maxY]
  if (!placementDirty && nextGeometry.every((value, index) => value === geometry[index])) return
  geometry = nextGeometry
  placementDirty = false
  const gap = 5
  const edge = 8
  const width = Math.min(anchor.width, maxX - minX - edge * 2)
  const left = Math.min(Math.max(anchor.left, minX + edge), maxX - edge - width)
  const roomBelow = maxY - edge - anchor.bottom - gap
  const roomAbove = anchor.top - gap - minY - edge
  menu.style.width = `${width}px`
  const height = Math.min(288, menu.scrollHeight + menu.offsetHeight - menu.clientHeight)
  const above = roomBelow < height && roomAbove > roomBelow
  const maxHeight = Math.max(0, Math.min(height, above ? roomAbove : roomBelow))
  const top = above ? anchor.top - gap - maxHeight : anchor.bottom + gap
  placement.value = {
    left: `${left}px`, top: `${top}px`, width: `${width}px`, maxHeight: `${maxHeight}px`, visibility: 'visible'
  }
}

function followAnchor() {
  if (!open.value) return
  place()
  animationFrame = requestAnimationFrame(followAnchor)
}

function clickOutside(event: PointerEvent) {
  if (!(event.target instanceof Node)) return
  if (!trigger.value?.contains(event.target) && !popup.value?.contains(event.target)) close()
}

function clearListeners() {
  intersectionObserver?.disconnect()
  document.removeEventListener('pointerdown', clickOutside, true)
  cancelAnimationFrame(animationFrame)
  animationFrame = 0
  placementDirty = true
}

function close() {
  openingId++
  typed = ''
  lastTypedAt = 0
  open.value = false
  clearListeners()
}

async function show(direction = 1) {
  if (props.disabled || !choices.value.length || open.value) return
  const id = ++openingId
  portal.value = trigger.value?.closest('dialog[open]') ?? document.fullscreenElement ?? 'body'
  placement.value = { ...placement.value, visibility: 'hidden' }
  const selected = selectedIndex.value
  activeIndex.value = selected >= 0 && !choices.value[selected].disabled
    ? selected
    : enabledFrom(direction > 0 ? 0 : choices.value.length - 1, direction)
  open.value = true
  await nextTick()
  const anchor = trigger.value
  if (id !== openingId || !anchor || !popup.value) return
  place()
  await nextTick()
  if (id !== openingId) return
  revealActive()
  intersectionObserver = new IntersectionObserver(([entry]) => {
    if (id === openingId && entry && !entry.isIntersecting) close()
  })
  intersectionObserver.observe(anchor)
  document.addEventListener('pointerdown', clickOutside, true)
  animationFrame = requestAnimationFrame(followAnchor)
}

function choose(index: number) {
  const choice = choices.value[index]
  if (!choice || choice.disabled) return
  close()
  trigger.value?.focus({ preventScroll: true })
  if (index !== selectedIndex.value) {
    emit('update:modelValue', choice.value)
    emit('change', choice.value)
  }
}

function activate(index: number) {
  if (index < 0) return
  activeIndex.value = index
  revealActive()
}

async function onKey(event: KeyboardEvent) {
  if (event.key === 'Tab') {
    close()
    return
  }
  if (event.key === 'Escape') {
    if (!open.value) return
    event.preventDefault()
    event.stopPropagation()
    close()
    return
  }
  if (['Enter', ' '].includes(event.key)) {
    event.preventDefault()
    if (open.value) choose(activeIndex.value)
    else void show()
    return
  }
  if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    event.preventDefault()
    const direction = event.key === 'ArrowUp' || event.key === 'End' ? -1 : 1
    if (!open.value) await show(direction)
    else if (event.key.startsWith('Arrow')) activate(enabledFrom(activeIndex.value + direction, direction, true))
    if (event.key === 'Home' || event.key === 'End') {
      activate(enabledFrom(direction === 1 ? 0 : choices.value.length - 1, direction))
    }
    return
  }
  if (event.key.length !== 1 || event.ctrlKey || event.metaKey || event.altKey || event.isComposing) return
  event.preventDefault()
  const now = Date.now()
  const letter = event.key.toLowerCase()
  typed = now - lastTypedAt < 650 ? typed + letter : letter
  lastTypedAt = now
  if (!open.value) await show()
  const repeated = typed.length > 1 && [...typed].every(char => char === letter)
  const prefix = repeated ? letter : typed
  const start = activeIndex.value + (repeated || typed.length === 1 ? 1 : 0)
  const options = choices.value
  for (let offset = 0; offset < options.length; offset++) {
    const index = (start + offset + options.length) % options.length
    if (!options[index].disabled && options[index].label.toLowerCase().startsWith(prefix)) {
      activate(index)
      break
    }
  }
}

watch(() => props.disabled, disabled => { if (disabled) close() })
watch(choices, () => {
  if (!open.value) return
  if (!choices.value.length) close()
  else {
    if (!choices.value[activeIndex.value] || choices.value[activeIndex.value]?.disabled) {
      activeIndex.value = enabledFrom(0, 1)
    }
    placementDirty = true
  }
})
onBeforeUnmount(clearListeners)
</script>

<template>
  <button
    ref="trigger"
    type="button"
    class="app-select nodrag"
    role="combobox"
    aria-haspopup="listbox"
    :aria-expanded="open"
    :aria-controls="open ? listId : undefined"
    :aria-activedescendant="open && activeIndex >= 0 ? optionId(activeIndex) : undefined"
    :disabled="disabled"
    @click.stop="open ? close() : show()"
    @keydown="onKey"
    @blur="close"
  >
    <span class="app-select-value">{{ selectedLabel }}</span>
    <HomeIcon class="app-select-arrow" name="chevronDown" :size="12" />
    <Teleport v-if="open" :to="portal">
      <div
        :id="listId"
        ref="popup"
        class="app-select-menu nowheel"
        role="listbox"
        :aria-label="String($attrs['aria-label'] || $attrs.title || '可选项')"
        :style="placement"
        @mousedown.prevent
        @pointerdown.stop
        @wheel.stop
        @click.stop
      >
        <template v-for="(choice, index) in choices" :key="index">
          <div v-if="choice.startsGroup" class="app-select-group" role="presentation">{{ choice.group }}</div>
          <div
            :id="optionId(index)"
            class="app-select-option"
            :class="{ highlighted: activeIndex === index }"
            role="option"
            :data-option-index="index"
            :aria-label="choice.group ? `${choice.group} · ${choice.label}` : undefined"
            :aria-selected="selectedIndex === index"
            :aria-disabled="choice.disabled || undefined"
            @pointermove="!choice.disabled && (activeIndex = index)"
            @click="choose(index)"
          >
            <span>{{ choice.label }}</span>
            <HomeIcon v-if="selectedIndex === index" name="check" :size="13" />
          </div>
        </template>
      </div>
    </Teleport>
  </button>
</template>

<style scoped>
.app-select {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
  padding: 4px 8px;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-sm);
  background: var(--color-bg-2);
  color: var(--color-text-1);
  font: inherit;
  line-height: inherit;
  text-align: left;
  vertical-align: middle;
}
.app-select:hover:not(:disabled) { background: var(--color-bg-3); }
.app-select[aria-expanded="true"] { border-color: var(--focus-ring); }
.app-select:disabled { opacity: 0.55; cursor: default; }
.app-select-value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.app-select-arrow { flex: none; color: var(--color-text-3); }
.app-select[aria-expanded="true"] .app-select-arrow { rotate: 180deg; }
.app-select-menu {
  position: fixed;
  z-index: 10020;
  margin: 0;
  padding: 4px;
  overflow-y: auto;
  overscroll-behavior: contain;
  border: 1px solid var(--color-border);
  border-radius: var(--radius-md);
  background: var(--glass-panel);
  box-shadow: var(--shadow-lg);
  color: var(--color-text-1);
  font: 12px/1.4 var(--font-sans);
}
.app-select-group {
  padding: 7px 7px 4px;
  color: var(--color-text-3);
  font-size: 11px;
  overflow-wrap: anywhere;
}
.app-select-option {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 6px;
  min-height: 28px;
  padding: 5px 7px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  overflow-wrap: anywhere;
}
.app-select-option svg { flex: none; }
.app-select-option[aria-selected="true"] { background: var(--control-selected); }
.app-select-option.highlighted { background: var(--control-pressed); }
.app-select-option[aria-disabled="true"] { color: var(--color-text-4); cursor: default; }
</style>
