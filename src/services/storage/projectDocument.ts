import type { Project } from '@/types'

export function isProjectDocument(value: unknown): value is Project {
  if (!value || typeof value !== 'object') return false
  const project = value as Partial<Project>
  return typeof project.id === 'string' && project.id.length > 0 &&
    typeof project.name === 'string' && Number.isFinite(project.createdAt) && Number.isFinite(project.updatedAt) &&
    Array.isArray(project.shots) && Array.isArray(project.timeline) &&
    Array.isArray(project.canvas?.nodes) && Array.isArray(project.canvas.edges) &&
    Array.isArray(project.canvas.groups) && isObject(project.canvas.viewport) &&
    isObject(project.settings?.captionStyle) && Number.isFinite(project.settings?.width) &&
    Number.isFinite(project.settings?.height) && Number.isFinite(project.settings?.fps)
}

function isObject(value: unknown): boolean {
  return Boolean(value) && typeof value === 'object' && !Array.isArray(value)
}
