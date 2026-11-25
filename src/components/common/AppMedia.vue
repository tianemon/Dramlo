<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import HomeIcon from './HomeIcon.vue'
import AppSelect from './AppSelect.vue'

const props = withDefaults(defineProps<{
  src: string
  kind?: 'video' | 'audio'
  autoplay?: boolean
  loop?: boolean
  muted?: boolean
  playsinline?: boolean
  preload?: 'none' | 'metadata' | 'auto'
  poster?: string
}>(), { kind: 'video', playsinline: true, preload: 'metadata' })

const root = ref<HTMLElement>()
const media = ref<HTMLMediaElement>()
const playing = ref(false)
const silent = ref(false)
const time = ref(0)
const duration = ref(0)
const speed = ref(1)
const volume = ref(1)
const playbackError = ref('')
const rates = [0.5, 0.75, 1, 1.25, 1.5, 2]
const controlLabel = computed(() => props.kind === 'audio' ? '音频播放控制' : '视频播放控制')

watch(() => props.src, () => {
  time.value = 0
  duration.value = 0
  playing.value = false
  playbackError.value = ''
})

function readMedia(event: Event) {
  const target = event.currentTarget as HTMLMediaElement
  time.value = target.currentTime
  duration.value = Number.isFinite(target.duration) ? target.duration : 0
  playing.value = !target.paused && !target.ended
  silent.value = target.muted || !target.volume
  volume.value = target.volume
  speed.value = target.playbackRate
}

async function togglePlayback() {
  if (!media.value) return
  if (!media.value.paused) {
    media.value.pause()
    return
  }
  try {
    playbackError.value = ''
    await media.value.play()
  } catch {
    playbackError.value = '无法播放此文件'
  }
}

function seek(event: Event) {
  if (media.value) media.value.currentTime = (event.target as HTMLInputElement).valueAsNumber
}

function toggleMute() {
  if (!media.value) return
  const wasSilent = media.value.muted || !media.value.volume
  if (!media.value.volume) media.value.volume = 1
  media.value.muted = !wasSilent
}

function changeVolume(event: Event) {
  if (!media.value) return
  media.value.volume = (event.target as HTMLInputElement).valueAsNumber
  media.value.muted = false
}

function changeSpeed(rate: number) {
  if (media.value) media.value.playbackRate = rate
}

async function toggleFullscreen() {
  if (document.fullscreenElement === root.value) await document.exitFullscreen().catch(() => {})
  else await root.value?.requestFullscreen?.().catch(() => {})
}

function format(value: number) {
  return `${Math.floor(value / 60)}:${String(Math.floor(value % 60)).padStart(2, '0')}`
}
</script>

<template>
  <div ref="root" class="app-media" :class="[`app-media-${kind}`, { 'is-playing': playing }]">
    <component
      :is="kind"
      ref="media"
      :src="src"
      :autoplay="autoplay"
      :loop="loop"
      :muted="muted"
      :playsinline="kind === 'video' ? playsinline : undefined"
      :poster="kind === 'video' ? poster : undefined"
      :preload="preload"
      @click.stop="togglePlayback"
      @loadedmetadata="readMedia"
      @durationchange="readMedia"
      @timeupdate="readMedia"
      @play="readMedia"
      @pause="readMedia"
      @ended="readMedia"
      @volumechange="readMedia"
      @ratechange="readMedia"
    />
    <p v-if="playbackError" class="media-message" role="status">{{ playbackError }}</p>
    <div class="media-controls nodrag nowheel" role="group" :aria-label="controlLabel" @pointerdown.stop @click.stop>
      <button type="button" :aria-label="playing ? '暂停' : '播放'" :title="playing ? '暂停' : '播放'" @click="togglePlayback">
        <HomeIcon :name="playing ? 'pause' : 'play'" :size="14" />
      </button>
      <input type="range" min="0" :max="duration || 1" step="any" :value="time" aria-label="播放进度"
        :aria-valuetext="`${format(time)} / ${format(duration)}`" :disabled="!duration" @input="seek">
      <span class="media-time">{{ format(time) }} / {{ format(duration) }}</span>
      <button type="button" class="media-mute" :aria-label="silent ? '取消静音' : '静音'" @click="toggleMute">
        <HomeIcon name="volume" :size="14" :class="{ muted: silent }" />
      </button>
      <input class="media-volume" type="range" min="0" max="1" step=".05" :value="silent ? 0 : volume"
        aria-label="音量" @input="changeVolume">
      <AppSelect class="media-rate" :value="speed" aria-label="播放速度" @change="changeSpeed">
        <option v-for="rate in rates" :key="rate" :value="rate">{{ rate }}×</option>
      </AppSelect>
      <button v-if="kind === 'video'" type="button" class="media-full" aria-label="切换全屏" title="切换全屏" @click="toggleFullscreen">
        <HomeIcon name="expand" :size="14" />
      </button>
    </div>
  </div>
</template>

<style scoped>
.app-media {
  position: relative;
  display: block;
  min-width: 0;
  max-width: 100%;
  border-radius: inherit;
  overflow: hidden;
}
.app-media-video {
  background: #101010;
  color: #f0f0f0;
}
.app-media-video > video {
  display: block;
  width: 100%;
  height: 100%;
  max-height: inherit;
  object-fit: var(--media-fit, contain);
}
.app-media-video:fullscreen > video { max-height: 100%; }
.app-media-audio > audio { display: none; }
.media-controls {
  display: flex;
  align-items: center;
  min-height: 30px;
  gap: 4px;
  padding: 2px 5px;
  color: inherit;
  container-type: inline-size;
}
.app-media-video .media-controls {
  position: absolute;
  inset: auto 0 0;
  background: #121212d9;
  --control-hover: #ffffff20;
  --control-pressed: #ffffff34;
  --focus-ring: #cecece;
  --color-fill-strong: #ffffff45;
  --color-accent-2: #e6e6e6;
  transition: opacity var(--duration-fast) ease;
}
.app-media-video.is-playing:not(:hover):not(:has(:focus-visible, [aria-expanded="true"])) .media-controls {
  opacity: 0;
  pointer-events: none;
}
.app-media-audio {
  border-radius: var(--radius-sm);
  background: var(--color-fill);
  color: var(--color-text-1);
}
.media-controls > button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex: none;
  min-width: 23px;
  height: 25px;
  padding: 2px 4px;
  border-radius: var(--radius-sm);
}
.media-controls > input { min-width: 12px; width: auto; flex: 1; margin: 0 1px; padding: 0; }
.media-controls > .media-volume { width: 40px; flex: 0 0 40px; }
.media-time { white-space: nowrap; font-size: 10px; font-variant-numeric: tabular-nums; }
.media-controls .media-rate { min-width: 41px; border-color: transparent; background: transparent; color: inherit; font-size: 10px; }
.media-controls .media-rate:hover { background: var(--control-hover); }
.media-mute { position: relative; }
.media-controls .muted { opacity: .55; }
.media-mute:has(.muted)::after {
  content: '';
  position: absolute;
  width: 13px;
  height: 1px;
  background: currentColor;
  transform: rotate(-45deg);
}
.media-message { position: absolute; inset: 0 0 auto; margin: 0; padding: 3px 6px; background: #101010c9; color: white; font-size: 11px; }
@container (max-width: 220px) {
  .media-time, .media-controls > .media-volume { display: none; }
}
@container (max-width: 160px) {
  .media-controls .media-rate, .media-controls .media-mute { display: none; }
}
</style>
