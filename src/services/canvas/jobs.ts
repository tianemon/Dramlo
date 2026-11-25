import { ref } from 'vue'

/** 正在生成的画布节点。各节点互不影响，可同时点。 */
export const runningNodeIds = ref<string[]>([])
type ServerVideoActivity = Record<string, 'pending' | 'running'>
const serverVideoActivity = ref<Record<string, { owner: symbol; shots: ServerVideoActivity }>>({})

export function setServerVideoActivity(projectId: string, owner: symbol, shots: ServerVideoActivity) {
  serverVideoActivity.value[projectId] = { owner, shots }
}

export function clearServerVideoActivity(projectId: string, owner: symbol) {
  if (serverVideoActivity.value[projectId]?.owner === owner) delete serverVideoActivity.value[projectId]
}

export function isServerVideoRunning(projectId?: string, shotId?: string) {
  return Boolean(getServerVideoActivity(projectId, shotId))
}

export function getServerVideoActivity(projectId?: string, shotId?: string) {
  const shots = projectId ? serverVideoActivity.value[projectId]?.shots : undefined
  return shots && shotId && Object.hasOwn(shots, shotId) ? shots[shotId] : undefined
}

export function isNodeRunning(id: string) {
  return runningNodeIds.value.includes(id)
}

export async function withNodeJob<T>(id: string, fn: () => Promise<T>): Promise<T> {
  if (runningNodeIds.value.includes(id)) {
    throw new Error('这个节点正在生成')
  }
  runningNodeIds.value = [...runningNodeIds.value, id]
  try {
    return await fn()
  } finally {
    runningNodeIds.value = runningNodeIds.value.filter((x) => x !== id)
  }
}

/** AutoDL 出视频支持并发；批量出片按这个上限同时提交 */
export const VIDEO_GEN_CONCURRENCY = 8

export async function mapPool<T>(
  items: T[],
  limit: number,
  fn: (item: T) => Promise<void>
): Promise<void> {
  if (!items.length) return
  const queue = [...items]
  const n = Math.min(Math.max(1, limit), queue.length)
  await Promise.all(
    Array.from({ length: n }, async () => {
      while (queue.length) {
        const item = queue.shift()
        if (item === undefined) break
        try {
          await fn(item)
        } catch {
          /* 单个失败不影响其它 */
        }
      }
    })
  )
}
