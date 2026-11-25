import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'

const scratch = await fs.mkdtemp(path.join(os.tmpdir(), 'studio-discovery-'))
const workflows = path.join(scratch, 'workflows')
await fs.mkdir(workflows)
const graph = {
  '23:22': { class_type: 'PrimitiveStringMultiline', inputs: { value: 'demo' } },
  '23:18': { class_type: 'PreviewAny', inputs: { source: ['23:20', 0] } },
  '23:20': { class_type: 'ComfySwitchNode', inputs: { switch: true, on_false: 'unused', on_true: ['23:19', 0] } },
  '23:19': { class_type: 'StringReplace', inputs: { string: ['23:17', 0], replace: ['23:22', 0] } },
  '23:17': { class_type: 'PrimitiveStringMultiline', inputs: { value: 'unrelated' } },
  '5': { class_type: 'MiniMaxH3ImageConditioning', inputs: { prompt: ['23:18', 0] } },
  '8': { class_type: 'SaveImage', inputs: {} }
}
await fs.writeFile(path.join(workflows, 'image.json'), JSON.stringify(graph))
await fs.writeFile(path.join(workflows, 'canvas.json'), JSON.stringify({ nodes: [], links: [] }))
await fs.writeFile(path.join(scratch, 'unrelated.json'), JSON.stringify(graph))

const client = new Client({ name: 'studio-discovery-test', version: '1.0.0' })
const binary = process.env.DRAMLO_SERVER_BINARY || path.resolve('dramlo-server/target/debug',
  process.platform === 'win32' ? 'dramlo-server.exe' : 'dramlo-server')
const transport = new StdioClientTransport({
  command: binary,
  args: ['mcp', 'comfy'],
  env: { ...process.env, DRAMLO_COMFY_WORKFLOW_DIRS: JSON.stringify([workflows]), DRAMLO_COMFY_URL: 'http://127.0.0.1:1' }
})
async function call(name, args) {
  const reply = await client.callTool({ name, arguments: args })
  assert.ok(!reply.isError, JSON.stringify(reply.content))
  return reply.structuredContent
}

try {
  await client.connect(transport)
  const found = await call('comfy_workflows_list', { includeServer: false })
  assert.deepEqual(found.items.map((item) => item.format).sort(), ['api', 'ui'])
  const result = await call('comfy_workflow_read', { source: 'disk', folder: 0, relativePath: 'image.json', includeApiJson: true })
  assert.deepEqual(JSON.parse(result.apiJson), graph)
  assert.equal(result.output, 'image')
  assert.ok(result.candidateSlots.some((slot) => slot.role === 'prompt' && slot.nodeId === '23:22' && slot.field === 'value'))
  const ui = await call('comfy_workflow_read', { source: 'disk', folder: 0, relativePath: 'canvas.json', includeApiJson: true })
  assert.equal(ui.format, 'ui')
  assert.equal(ui.apiJson, undefined)
  const outside = await client.callTool({ name: 'comfy_workflow_read', arguments: { source: 'disk', folder: 0, relativePath: '../unrelated.json' } })
  assert.ok(outside.isError)
  console.log('PASS ComfyUI MCP discovery, linked prompt analysis, API export, and directory boundaries')
} finally {
  await client.close()
  if (path.dirname(scratch) === os.tmpdir() && path.basename(scratch).startsWith('studio-discovery-')) {
    await fs.rm(scratch, { recursive: true, force: true })
  }
}
