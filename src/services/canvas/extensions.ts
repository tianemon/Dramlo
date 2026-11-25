import type { Project } from '@/types/project'
import { useProjectStore, useAssetsStore } from '@/stores'

export interface CanvasExtensionType {
  id: string
  version: number
  name: string
  description: string
  source: string
  enabled: boolean
  executor: string
  manifest?: Record<string, unknown>
  inputs: { id: string; kind: string }[]
  outputs: { id: string; kind: string }[]
  params?: CanvasExtensionParameter[]
}

export interface CanvasExtensionParameter {
  key: string
  label: string
  type: 'string' | 'integer' | 'number' | 'boolean' | 'model'
  default?: string | number | boolean
  enum?: (string | number | boolean)[]
  min?: number
  max?: number
}

export async function extensionRequest<T>(name: string, input: object): Promise<T> {
  const response = await fetch('/api/dramlo/canvas-extensions', {
    method: 'POST',
    headers: { 'X-Dramlo-UI': '1', 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, input })
  })
  if (!response.ok) {
    const body = await response.text()
    const error = response.headers.get('content-type')?.includes('application/json') ? JSON.parse(body).error : body
    throw new Error(error || '画布扩展操作失败')
  }
  return response.json()
}

export async function listCanvasExtensions() {
  return (await extensionRequest<{ items: CanvasExtensionType[] }>('studio_list_node_types', { projectId: useProjectStore().current?.id })).items
}

export async function mutateCanvasExtension(name: string, input: object) {
  const project = useProjectStore()
  const intendedProject = project.current?.id
  if (!intendedProject) throw new Error('请先打开项目')
  if (project.dirty) await project.save()
  const current = project.current
  if (!current || current.id !== intendedProject || project.dirty) throw new Error('项目状态已改变，请重试')
  await extensionRequest(name, { ...input, projectId: current.id, expectedUpdatedAt: current.updatedAt })
  if (project.current?.id === current.id && !project.dirty) {
    await project.load(current.id)
    await useAssetsStore().refresh()
  }
}

export function linkedFirstFrame(project: Project, target: string): { connected: boolean; assetId?: string } {
  const edge = project.canvas?.edges.find(item => item.target === target && item.targetHandle === 'first-frame')
  if (!edge) return { connected: false }
  const node = project.canvas?.nodes.find(item => item.id === edge.source)
  if (node?.data.extensionExecutor === 'studio.mcp') {
    return { connected: true, assetId: node.data.outputAssetId }
  }
  const video = project.canvas?.edges.find(item => item.target === node?.id)
  const upstream = project.canvas?.nodes.find(item => item.id === video?.source)
  const source = upstream?.type === 'shotVideo'
    ? project.shots.find(shot => shot.id === upstream.data.shotId)?.videoAssetId
    : video ? upstream?.data.outputAssetId : node?.data.extensionVideoAssetId
  return {
    connected: true,
    assetId: source && node?.data.extensionSourceAssetId === source ? node.data.outputAssetId : undefined
  }
}
