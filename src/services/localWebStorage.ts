import type { DesktopAPI } from './desktop'

const base = '/api/dramlo/'
const headers = { 'X-Dramlo-UI': '1' }

export async function callWorkspace<T>(name: string, input: object): Promise<T> {
  const response = await check(await fetch(base + 'workspace-actions', {
    method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, input })
  }))
  return response.json()
}

export async function removeWebShot(projectId: string, shotId: string, updatedAt: number) {
  return callWorkspace<{ applied: boolean; updatedAt: number }>('studio_remove_shot', {
    projectId, shotId, expectedUpdatedAt: updatedAt, confirmed: true
  })
}

export function localMediaUrl(path: string) {
  return base + 'file?' + new URLSearchParams({ path })
}

async function check(response: Response) {
  if (!response.ok) {
    const problem = await response.json().catch(() => null)
    throw new Error(problem?.error || '本机 Dramlo 服务返回 HTTP ' + response.status)
  }
  return response
}

export async function callWebVideoQueue<T>(name: string, input: object): Promise<T> {
  const response = await check(await fetch(base + 'video-queue', {
    method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' },
    body: JSON.stringify({ name, input })
  }))
  return response.json()
}

function pickFiles(accept: string, multiple: boolean): Promise<File[]> {
  return new Promise((resolve) => {
    const input = document.createElement('input')
    input.type = 'file'
    input.accept = accept
    input.multiple = multiple
    input.addEventListener('change', () => resolve(Array.from(input.files ?? [])), { once: true })
    input.addEventListener('cancel', () => resolve([]), { once: true })
    input.click()
  })
}

export async function connectLocalWebStorage() {
  if (window.desktop || !['127.0.0.1', 'localhost'].includes(location.hostname)) return
  const response = await fetch(base + 'session', { headers, signal: AbortSignal.timeout(1200) }).catch(() => undefined)
  const session = response?.ok ? await response.json().catch(() => undefined) : undefined
  if (session?.service !== 'dramlo-web') return
  const separator = session.platform === 'win32' ? '\\' : '/'
  let currentSettings: string | null = null
  const imported = new Map<string, File>()
  const projectStamps = new Map<string, number>()
  function supported<T = never>(message: string): Promise<T> {
    return Promise.reject(new Error(message))
  }
  async function call<T>(command: string, values = {}): Promise<T> {
    const response = await check(await fetch(base + 'fs', {
      method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' },
      body: JSON.stringify({ ...values, command })
    }))
    return response.json()
  }
  const api: DesktopAPI = {
    isDesktop: true,
    isLocalWeb: true,
    removeIndexed: (name, id, path) => call('removeIndexed', { name, id, path }),
    platform: session.platform,
    getRoot: () => call('getRoot'),
    setRoot: () => supported('请在启动本机 Web 服务时指定工作区目录'),
    getVersion: async () => session.version,
    ensureProjectDir: (name, id) => call('ensureProjectDir', { name, id }),
    listProjectDirs: () => call('listProjectDirs'),
    readText: async (path) => {
      const content = imported.has(path) ? await imported.get(path)!.text() : await call<string>('readText', { path })
      // Storage reads can discover a save made in another tab. Do not advance the expected
      // revision of a loaded project until its project store explicitly reloads it.
      if (/[\\/]project\.json$/i.test(path) && !projectStamps.has(path)) {
        projectStamps.set(path, JSON.parse(content).updatedAt)
      }
      return content
    },
    writeText: async (path, value) => {
      const project = /[\\/]project\.json$/i.test(path)
      const expectedUpdatedAt = project ? projectStamps.get(path) : undefined
      await call('writeText', { path, value, expectedUpdatedAt })
      if (project) projectStamps.set(path, JSON.parse(value).updatedAt)
    },
    patchAssetManifest: (path, id, value) => call('patchAssetManifest', { path, id, value }),
    readBuffer: async (path) => imported.has(path) ? imported.get(path)!.arrayBuffer() :
      (await check(await fetch(localMediaUrl(path), { headers }))).arrayBuffer(),
    writeBuffer: async (path, data) => {
      await check(await fetch(localMediaUrl(path), { method: 'PUT', headers, body: data }))
    },
    exists: (path) => imported.has(path) ? Promise.resolve(true) : call('exists', { path }),
    remove: () => supported('Web 工作区暂未提供删除。可以在文件管理器中处理对应项目。'),
    join: async (...parts) => parts.map((part) => part.replace(/[\\/]/g, separator)).join(separator),
    pickFiles: async (filters) => {
      const selected = await pickFiles(filters?.flatMap((filter) => filter.extensions).map((ext) => '.' + ext).join(',') ?? '', false)
      imported.clear()
      return selected.map((file) => {
        const key = '.dramlo-import/' + crypto.randomUUID() + '/' + file.name
        imported.set(key, file)
        return key
      })
    },
    importLocalFiles: async ({ kind, multiple } = {}) => {
      const accept = kind && kind !== 'media' ? kind + '/*' : 'image/*,audio/*,video/*'
      return Promise.all((await pickFiles(accept, !!multiple)).map(async (file) => ({
        name: file.name, mime: file.type, data: await file.arrayBuffer()
      })))
    },
    requestFileAccess: async () => true,
    openWindowsSecurity: () => supported('请通过系统设置管理文件访问'),
    saveTextFile: async ({ defaultName, text }) => {
      const url = URL.createObjectURL(new Blob([text], { type: 'text/plain;charset=utf-8' }))
      const anchor = document.createElement('a')
      anchor.href = url
      anchor.download = defaultName
      anchor.click()
      setTimeout(() => URL.revokeObjectURL(url), 1000)
      return defaultName
    },
    pickDirectory: async () => null,
    openPath: (path) => call('openPath', { path }),
    autodlFetch: async ({ url, method, body, bodyBase64, contentType, extraHeaders }) => {
      const response = await check(await fetch(base + 'comfy', {
        method: 'POST', headers: { ...headers, 'Content-Type': 'application/json' },
        body: JSON.stringify({ url, method, body, bodyBase64,
          contentType: contentType || extraHeaders?.['Content-Type'] })
      }))
      return response.json()
    },
    llmFetch: () => supported('本机 Web 工作区请通过外部 Agent 处理文本创作'),
    dashscopeFetch: () => supported('此接入尚未由本机 Web 工作区接管'),
    downloadToTemp: () => supported('本机 Web 工作区尚未支持远程临时下载'),
    dbPath: async () => session.root,
    kvGet: async (name) => {
      const value = await call<string | null>('kvGet', { name })
      if (name === 'settings') currentSettings = value
      return value
    },
    kvSet: async (name, value) => {
      if (name !== 'settings') return call('kvSet', { name, value })
      await call('kvSet', { name, value, compareValue: true, expectedValue: currentSettings })
      currentSettings = value
    },
    acceptProjectRevision: (path, revision) => { projectStamps.set(path, revision) },
    setActiveProject: (id) => call('setActiveProject', { id }),
    getActiveProject: () => call('getActiveProject')
  }
  window.desktop = api
}
