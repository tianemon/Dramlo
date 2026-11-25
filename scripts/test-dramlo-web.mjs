import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'
import { execFile, spawn } from 'node:child_process'
import { promisify } from 'node:util'
import { once } from 'node:events'
import { fileURLToPath } from 'node:url'
import { createServer } from 'node:http'
import { createServer as reservePort } from 'node:net'
import { randomBytes, randomUUID } from 'node:crypto'
import { ToolListChangedNotificationSchema } from '@modelcontextprotocol/sdk/types.js'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js'

const cwd = fileURLToPath(new URL('../', import.meta.url))
const binary = process.env.DRAMLO_SERVER_BINARY || path.join(cwd, 'dramlo-server', 'target', 'debug',
  process.platform === 'win32' ? 'dramlo-server.exe' : 'dramlo-server')
const frontend = process.env.DRAMLO_TEST_FRONTEND || path.join(cwd, 'dist')
await fs.access(binary)
const scratch = await fs.mkdtemp(path.join(os.tmpdir(), 'dramlo-web-test-'))
const root = path.join(scratch, 'projects')
await fs.mkdir(root)

const token = randomBytes(32).toString('hex')
const env = { ...process.env, DRAMLO_MCP_BRIDGE_TOKEN: token }
let submits = 0
let ready = false
const uploads = []
const graphs = []
let movie = Buffer.from('0000ftypisom0000')
const openaiRequests = []
let onHeldOpenAIResponse
const pngFixture = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAACklEQVR4nGNgAAAAAgABSK+kcQAAAABJRU5ErkJggg==', 'base64')
let extraServerWorkflow = false
let serveCompactVideo = false
const onlineWorkflow = {
  '1':{class_type:'CLIPTextEncode',inputs:{text:'prompt'}},
  '2':{class_type:'CheckpointLoaderSimple',inputs:{ckpt_name:'fixture.safetensors'}},
  '3':{class_type:'SaveImage',inputs:{filename_prefix:'fixture'}},
  '4':{class_type:'ResolutionSelector',inputs:{aspect_ratio:'16:9 (Widescreen)',megapixels:4}}
}
const mock = createServer(async (req, res) => {
  if (['/v1/images/generations', '/v1/images/edits'].includes(req.url)) {
    const parts = []
    for await (const chunk of req) parts.push(chunk)
    const body = Buffer.concat(parts)
    openaiRequests.push({route:req.url, method:req.method, headers:req.headers, body})
    if (body.includes('rejected-moderation') || body.includes('rejected-size')) {
      const moderation=body.includes('rejected-moderation')
      return res.writeHead(400, {'Content-Type':'application/json','x-request-id':'req_test_fixture'})
        .end(JSON.stringify({error:{code:moderation?'moderation_blocked':'invalid_value',
          param:moderation?'prompt':'size',message:'private upstream echo '+req.headers.authorization}}))
    }
    if (body.includes('uncertain-provider-result')) return res.writeHead(502).end('upstream timeout')
    const respond = () => res.writeHead(200, {'Content-Type':'application/json'})
      .end(JSON.stringify({data:[{b64_json:pngFixture.toString('base64')}]}))
    if (body.includes('hold-for-approval-review') && onHeldOpenAIResponse) return onHeldOpenAIResponse(respond)
    return respond()
  }
  if (req.url === '/system_stats') return res.end(JSON.stringify({ system: {}, devices: [] }))
  if (decodeURIComponent(req.url || '') === '/object_info/UNETLoader') return res.end(JSON.stringify({
    UNETLoader:{input:{required:{unet_name:[['original.safetensors','alternate.safetensors']]}}}
  }))
  if (decodeURIComponent(req.url || '') === '/object_info/ResolutionSelector') return res.end(JSON.stringify({
    ResolutionSelector:{input:{required:{
      aspect_ratio:[['16:9 (Widescreen)','9:16 (Portrait Widescreen)']],
      megapixels:['FLOAT',{min:0.1,max:16,step:0.1}]
    }}}
  }))
  if (req.url?.startsWith('/userdata?')) {
    const names = ['workflows/online.json']
    if (serveCompactVideo) names.push('workflows/compact-video.json')
    if (extraServerWorkflow) names.push({path:'same-name.json'}, {path:'server-only.json'})
    return res.end(JSON.stringify(names))
  }
  if (serveCompactVideo && req.url?.startsWith('/userdata/') && decodeURIComponent(req.url).includes('compact-video.json')) {
    return res.end(JSON.stringify({
      '1': { class_type: 'CR Prompt Text', inputs: { prompt: 'workflow base prompt' } },
      '2': { class_type: 'PrimitiveFloat', inputs: { value: 5 }, _meta: { title: 'duration' } },
      '3': { class_type: 'VHS_VideoCombine', inputs: { images: ['6', 0] } },
      '4': { class_type: 'LoadImage', inputs: { image: 'first-template.png' } },
      '5': { class_type: 'LoadImage', inputs: { image: 'ref-template.png' } },
      '6': { class_type: 'MiniMaxH3Video', inputs: { prompt: ['1', 0], first_frame: ['4', 0],
        'ref_images.ref_image_0': ['5', 0] } }
    }))
  }
  if (req.url?.startsWith('/userdata/')) return res.end(JSON.stringify(onlineWorkflow))

  if (req.url === '/prompt') {
    const chunks = []
    for await (const chunk of req) chunks.push(chunk)
    const payload = JSON.parse(Buffer.concat(chunks))
    submits++
    graphs.push(payload.prompt)
    if (submits === 1) assert.equal(payload.prompt['1'].inputs.text, 'manual revision')
    return res.end(JSON.stringify({ prompt_id: submits === 1 ? 'fixture-task' : 'fixture-task-' + submits }))
  }
  if (req.url?.startsWith('/history/')) {
    const id = decodeURIComponent(req.url.slice('/history/'.length))
    return res.end(JSON.stringify(ready ? { [id]: { status: { completed: true }, outputs: { '2': { videos: [{ filename: 'fixture.mp4' }] } } } } : {}))
  }
  if (req.url === '/upload/image') {
    const chunks = []
    for await (const chunk of req) chunks.push(chunk)
    uploads.push(Buffer.concat(chunks))
    return res.end(JSON.stringify({ name: 'upload-' + uploads.length + '.png' }))
  }
  if (req.url?.startsWith('/view?')) return res.end(movie)
  res.writeHead(404).end()
})
mock.listen(0, '127.0.0.1')
await once(mock, 'listening')
let child
let coldOrigin
async function start(workspace = root) {
  for (let attempt = 0; attempt < 5; attempt++) {
    child = spawn(binary, [workspace, frontend, '0'], { windowsHide: true, env })
    const [chunk] = await once(child.stdout, 'data')
    const origin = chunk.toString().trim()
    assert.match(origin, /^http:\/\/127\.0\.0\.1:\d+$/)
    try {
      await (await fetch(origin + '/')).body.cancel()
      return origin
    } catch (error) {
      if (error.cause?.message !== 'bad port') throw error
      // Windows can allocate a port that browser-compatible fetch refuses to use.
      await stop()
    }
  }
  throw new Error('Could not allocate a browser-compatible test port')
}

async function stop() {
  if (!child || child.exitCode !== null || child.killed) return
  const exited = once(child, 'exit')
  child.kill()
  await exited
}

async function commands(origin) {
  const headers = { 'X-Dramlo-UI': '1', 'Content-Type': 'application/json' }
  const raw = (command, input = {}) => fetch(origin + '/api/dramlo/fs', {
    method: 'POST', headers, body: JSON.stringify({ command, ...input })
  })
  const ok = async (command, input) => {
    const result = await raw(command, input)
    assert.ok(result.ok, await result.clone().text())
    return result.json()
  }
  const file = (target) => origin + '/api/dramlo/file?' + new URLSearchParams({ path: target })
  return { raw, ok, file, headers }
}

async function testDirectBrowsePage(origin, testPort) {
  if (!process.env.DRAMLO_TEST_BROWSER) return
  let scopedScans = 0
  const relay = createServer(async (req, res) => {
    try {
      const headers = new Headers()
      for (const name of ['content-type', 'x-dramlo-ui']) {
        if (req.headers[name]) headers.set(name, req.headers[name])
      }
      let body
      if (req.method === 'POST') {
        const chunks = []
        for await (const chunk of req) chunks.push(chunk)
        body = Buffer.concat(chunks)
        if (req.url === '/api/dramlo/comfy-instances') {
          const request = JSON.parse(body)
          if (request.name === 'studio_comfy_discover') {
            scopedScans++
            request.input = { roots: [], ports: [testPort] }
            body = JSON.stringify(request)
          }
        }
      }
      const response = await fetch(origin + req.url, { method: req.method, headers, body })
      res.writeHead(response.status, { 'content-type': response.headers.get('content-type') || 'application/octet-stream' })
      res.end(Buffer.from(await response.arrayBuffer()))
    } catch (error) {
      res.writeHead(500).end(String(error))
    }
  })
  relay.listen(0, '127.0.0.1')
  await once(relay, 'listening')
  const dir = path.join(scratch, 'direct-comfy-browser')
  const chrome = spawn(process.env.DRAMLO_TEST_BROWSER, ['--headless=new', '--no-first-run',
    '--disable-background-networking', '--remote-debugging-port=0', '--user-data-dir=' + dir, 'about:blank'],
    { windowsHide:true, stdio:'ignore' })
  const wait = ms => new Promise(resolve => setTimeout(resolve, ms))
  let socket
  try {
    let port
    for (let i=0; i<60 && !port; i++) {
      try { port = (await fs.readFile(path.join(dir,'DevToolsActivePort'),'utf8')).split(/\s/)[0] }
      catch { await wait(100) }
    }
    assert.ok(port)
    const tabs = await (await fetch('http://127.0.0.1:' + port + '/json/list')).json()
    socket = new WebSocket(tabs.find(tab => tab.type === 'page').webSocketDebuggerUrl)
    await new Promise((resolve,reject) => { socket.addEventListener('open',resolve,{once:true}); socket.addEventListener('error',reject,{once:true}) })
    let sequence = 0
    function command(method,params) {
      return new Promise((resolve,reject) => {
        const id = ++sequence
        const listener = event => {
          const data = JSON.parse(event.data)
          if (data.id !== id) return
          clearTimeout(timer)
          socket.removeEventListener('message',listener)
          data.error ? reject(new Error(data.error.message)) : resolve(data.result)
        }
        const timer = setTimeout(() => { socket.removeEventListener('message',listener); reject(new Error(method)) },5000)
        socket.addEventListener('message',listener)
        socket.send(JSON.stringify({id,method,params}))
      })
    }
    const evaluate = async expression => (await command('Runtime.evaluate',{expression,returnByValue:true})).result.value
    async function until(expression) {
      for (let i=0; i<70; i++) {
        if (await evaluate(expression)) return
        await wait(100)
      }
      const page = await evaluate("document.querySelector('.comfy-managed')?.innerText")
      throw new Error('ComfyUI browser assertion: ' + expression + '\n' + page)
    }
    await command('Page.navigate',{url:'http://127.0.0.1:' + relay.address().port + '/#/settings/comfyui'})
    const click = label => `[...document.querySelectorAll('.comfy-managed button')].find(button => button.textContent.trim() === '${label}' && !button.disabled)?.click()`
    await until("[...document.querySelectorAll('.comfy-managed button')].some(button => button.textContent.trim() === '查看工作流' && !button.disabled)")
    await evaluate(`[...document.querySelectorAll('.comfy-managed .result')]
      .find(row => row.textContent.includes('http://127.0.0.1:${testPort}'))
      ?.querySelector('button')?.click()`)
    await until("[...document.querySelectorAll('.comfy-managed button')].some(button => button.textContent.trim() === '查看槽位' && !button.disabled)")
    await until("[...document.querySelectorAll('.comfy-managed .result')].some(row => row.textContent.includes('online.json') && row.textContent.includes('已配置'))")
    await evaluate(click('查看槽位'))
    await until("document.querySelector('.comfy-managed')?.textContent.includes('fixture.safetensors')")
    await evaluate(click('导入'))
    await until("document.querySelector('.comfy-managed')?.textContent.includes('此工作流已导入')")
    assert.equal(scopedScans, 1)
    console.log('PASS browser direct ComfyUI browse, slot details and import without registration')
  } finally {
    socket?.close()
    if (chrome.exitCode === null && !chrome.killed) {
      const exited = once(chrome,'exit')
      chrome.kill()
      await exited
    }
    await new Promise(resolve => relay.close(resolve))
  }
}

async function testLiveDiscovery() {
  const name = 'studio_fixture_future_tool'
  let description = 'Initial live schema'
  const upstream = createServer(async (request, response) => {
    response.setHeader('Content-Type', 'application/json')
    if (request.url === '/api/dramlo/mcp-capabilities') {
      return response.end(JSON.stringify({ tools: [name], toolDefinitions: [
        { name, description, inputSchema: { type: 'object' } }
      ] }))
    }
    const chunks = []
    for await (const chunk of request) chunks.push(chunk)
    const body = Buffer.concat(chunks)
    if (JSON.parse(body).input.plainConflict) return response.writeHead(409, { 'Content-Type': 'text/plain' }).end('conflict')
    response.end(body)
  })
  upstream.listen(0, '127.0.0.1')
  await once(upstream, 'listening')
  const client = new Client({ name: 'studio-live-discovery', version: '1' })
  try {
    await client.connect(new StdioClientTransport({ command: binary, args: ['mcp', 'studio'],
      env: { ...env, DRAMLO_WEB_MCP_URL: 'http://127.0.0.1:' + upstream.address().port } }))
    assert.equal(client.getServerCapabilities()?.tools?.listChanged, true)
    assert.match(client.getInstructions(), /studio_get_guide/)
    assert.match(client.getInstructions(), /影视短剧制作 Skill 库.*禁止把 Dramlo Skill 安装/)
    assert.equal((await client.listTools()).tools.find(tool => tool.name === name)?.description, description)
    let timer
    try {
      const changed = new Promise((resolve, reject) => {
        timer = setTimeout(() => reject(new Error('Missing MCP tools/list_changed notification')), 7000)
        client.setNotificationHandler(ToolListChangedNotificationSchema, resolve)
      })
      description = 'Updated live schema'
      await changed
    } finally {
      clearTimeout(timer)
    }
    assert.equal((await client.listTools()).tools.find(tool => tool.name === name)?.description, description)
    const call = await client.callTool({ name, arguments: { sample: true } })
    assert.deepEqual(call.structuredContent, { name, input: { sample: true } })
    const rejected = await client.callTool({ name, arguments: { plainConflict: true } })
    assert.equal(rejected.isError, true)
    assert.equal(rejected.structuredContent.code, 'conflict')
  } finally {
    await client.close()
    await new Promise(resolve => upstream.close(resolve))
  }
  console.log('PASS live MCP discovery, same-connection tool update notification and newly discovered tool routing')
}

async function testCompactMcp() {
  const workspace = path.join(scratch, 'compact-projects')
  await fs.mkdir(workspace)
  const origin = await start(workspace)
  await stop()
  coldOrigin = origin
  const compact = new Client({ name: 'studio-compact-binding', version: '1' })
  try {
    serveCompactVideo = true
    await compact.connect(new StdioClientTransport({ command: binary, args: ['mcp', 'studio-compact'],
      env: { ...env, DRAMLO_WEB_MCP_URL: origin, DRAMLO_WEB_WORKSPACE: workspace, DRAMLO_WEB_FRONTEND: frontend } }))
    const request = (name, args = {}) => compact.callTool({ name, arguments: args })
    async function compactCall(name, args = {}) {
      const reply = await request('studio_call', { name, arguments: args })
      assert.ok(!reply.isError, JSON.stringify(reply.content))
      return reply.structuredContent
    }
    const listed = (await compact.listTools()).tools
    assert.deepEqual(listed.map(tool => tool.name).sort(),
      ['studio_call', 'studio_find', 'studio_pending_jobs', 'studio_project', 'studio_web_status'])
    assert.ok(Buffer.byteLength(JSON.stringify(listed)) <
      (await fs.stat(path.join(cwd, 'dramlo-server', 'mcp-tools.json'))).size * 0.15)
    assert.equal((await request('studio_web_status')).structuredContent.ready, false)
    assert.equal((await request('studio_find', { query: 'studio_start_web' })).structuredContent.definition.name, 'studio_start_web')
    assert.equal((await compactCall('studio_start_web')).ready, true)
    assert.equal((await request('studio_find', { query: '给第8镜换人物', includeDefinition: true }))
      .structuredContent.definition.name, 'studio_update_shot_inputs')
    const { projectId } = await compactCall('studio_create_project', { name: 'Compact binding fixture' })
    const revision = { projectId, expectedUpdatedAt: 'auto' }
    await compactCall('studio_import_script', { ...revision, title: '测试剧本', content: '第一场。人物走进房间。' })
    const people = Array.from({ length: 3 }, randomUUID)
    const props = Array.from({ length: 3 }, randomUUID)
    const sceneId = randomUUID()
    const shotId = randomUUID()
    const card = (id, type) => ({ id, type, name: id, description: '测试资产',
      ...(type === 'subject' ? { kind: 'person' } : {}) })
    await compactCall('studio_upsert_project_assets', { ...revision, items: [
      ...people.map(id => card(id, 'subject')), ...props.map(id => card(id, 'prop')), card(sceneId, 'scene')
    ] })
    await compactCall('studio_append_shots', { ...revision, sceneId,
      shots: [{ id: shotId, title: '测试镜头', visualPrompt: '场面', videoPrompt: '测试表演稿', duration: 5,
        cameraMovement: 'static', voiceover: '', audioDescription: '', subjectIds: [] }] })
    const invalidReply = await request('studio_call', { name: 'studio_update_shot_inputs',
      arguments: { ...revision, shotId, propIds: 'prop-1' } })
    assert.equal(invalidReply.isError, true)
    await compactCall('studio_update_shot_inputs', { ...revision, shotId,
      subjectBindingMode: 'explicit', subjectIds: people, propIds: props })
    const bindingSources = async () => (await compactCall('studio_canvas_graph', { projectId }))
      .edges.filter(edge => edge.targetHandle === 'binding' && edge.target === 'shotVideo:' + shotId)
      .map(edge => edge.source).sort()
    assert.deepEqual(await bindingSources(), [
      ...people.map(id => 'subject:' + id), ...props.map(id => 'prop:' + id), 'scene:' + sceneId
    ].sort())
    await compactCall('studio_batch_edit_shots', { ...revision, edits: [
      { kind: 'inputs', shotId, subjectIds: [], propIds: [], sceneId: null }
    ] })
    await compactCall('studio_open_project', { projectId })
    assert.deepEqual(await bindingSources(), [])
    await compactCall('studio_batch_edit_shots', { ...revision, edits: [{
      kind: 'shot-fields', shotId, title: '01｜新镜头', cutIn: 'mid-action',
      audienceGain: '看到道具的用途', propFocus: '手中扇子'
    }] })
    const editedShot = (await compactCall('studio_shots', { projectId, shotId })).items[0]
    assert.equal(editedShot.cutIn, 'mid-action')
    assert.equal(editedShot.audienceGain, '看到道具的用途')
    assert.equal(editedShot.propFocus, '手中扇子')
    const updatedGraph = await compactCall('studio_canvas_graph', { projectId, nodeId: 'shotVideo:' + shotId })
    assert.equal(updatedGraph.focus.node.label, '01｜新镜头')
    const badEdition = await request('studio_call', { name: 'studio_batch_edit_shots', arguments: {
      ...revision, edits: [{ kind: 'shot-fields', shotId, cutIn: 'wrong' }]
    } })
    assert.equal(badEdition.isError, true)
    const { running } = await compactCall('studio_comfy_discover', { roots: [], ports: [mock.address().port] })
    const target = running.find(instance => instance.target.port === mock.address().port).target
    const { items: workflows } = await compactCall('studio_comfy_workflows', { target })
    const workflow = workflows.find(item => item.relativePath === 'online.json')
    const { profileId } = await compactCall('studio_comfy_import', workflow.readInput)
    const beforePrepare = submits
    const preview = await compactCall('studio_prepare_image', {
      ...revision, requestId: randomUUID(), targetType: 'shot-image', targetId: shotId, profileId
    })
    assert.equal(preview.job.promptSource, 'visualPrompt')
    assert.equal(submits, beforePrepare)
    const videoWorkflow = workflows.find(item => item.relativePath === 'compact-video.json')
    const video = await compactCall('studio_comfy_import', videoWorkflow.readInput)
    assert.equal(video.kind, 'video')
    await compactCall('studio_set_video_prompt', { ...revision, shotId, prompt: '手写视频定稿' })
    const shotAfterPrompt = await compactCall('studio_shots', { projectId, shotId, fields: ['compiledFromVideoPrompt'] })
    assert.equal(shotAfterPrompt.items[0].compiledFromVideoPrompt, '测试表演稿')
    const importFrame = () => compactCall('studio_import_image_asset', {
      projectId, requestId: randomUUID(), dataBase64: pngFixture.toString('base64')
    })
    const first = await importFrame()
    const reference = await importFrame()
    const requestId = randomUUID()
    const preparedVideo = await compactCall('studio_prepare_video', {
      ...revision, requestId, shots: [{ shotId, profileId: video.profileId, mode: 'first-frame',
        firstFrameAssetId: first.assetId, referenceAssetIds: [reference.assetId] }]
    })
    assert.equal(preparedVideo.batch.steps[0].prompt, '手写视频定稿')
    assert.deepEqual(preparedVideo.batch.steps[0].mediaInputs, [
      { role: 'reference', nodeId: '5', assetId: reference.assetId },
      { role: 'first-frame', nodeId: '4', assetId: first.assetId }
    ])
    assert.equal(submits, beforePrepare)
    await compactCall('studio_update_shot_inputs', { ...revision, shotId, subjectIds: [people[0]] })
    const stale = await request('studio_call', { name: 'studio_confirm_video',
      arguments: { projectId, requestId, confirmed: true } })
    assert.equal(stale.isError, true)
    assert.equal(submits, beforePrepare)
    await compactCall('studio_bind_shot_asset', { ...revision, shotId, assetId: first.assetId, slot: 'image' })
    assert.equal((await compactCall('studio_shots', { projectId, shotId, fields: ['status'] }))
      .items[0].status, 'image-generated')
    const { items: skills } = await compactCall('studio_list_skills')
    assert.ok(skills.some(skill => skill.id === 'drama-flow-explore' && !skill.body))
    assert.ok((await compactCall('studio_read_skill', { skillId: 'drama-flow-explore' })).body)
    const { project } = (await request('studio_project', { projectId })).structuredContent
    assert.equal((await compactCall('studio_save_exit', {
      projectId, expectedUpdatedAt: project.updatedAt, confirmed: true
    })).exitRequested, true)
    for (let attempt = 0; attempt < 30; attempt++) {
      if (!(await request('studio_web_status')).structuredContent.ready) break
      await new Promise(resolve => setTimeout(resolve, 100))
    }
    assert.equal((await request('studio_web_status')).structuredContent.ready, false)
    coldOrigin = undefined
    console.log('PASS compact MCP startup, shot bindings, ComfyUI import and preflight, and on-demand Skills')
  } finally {
    serveCompactVideo = false
    await compact.close()
  }
}

try {
  await testCompactMcp()
  if (!process.argv.includes('--compact-only')) {
  await testLiveDiscovery()
  let origin = await start()
  await assert.rejects(
    promisify(execFile)(binary, [root, frontend, '0'], { windowsHide: true, env, timeout: 3000 }),
    (error) => error.code === 1 && error.stderr.includes('此工作区已有 Dramlo Web 服务正在运行')
  )
  assert.equal((await fetch(origin + '/')).status, 200)
  assert.equal((await fetch(origin + '/api/dramlo/session')).status, 200)
  assert.equal((await fetch(origin + '/api/dramlo/session', {
    headers: { Origin: origin }
  })).status, 403)
  const metadata = await (await fetch(origin + '/api/dramlo/session', { headers: { 'X-Dramlo-UI': '1' } })).json()
  if (env.DRAMLO_FFMPEG_BIN) assert.equal(metadata.mcpFfmpegBin, env.DRAMLO_FFMPEG_BIN)
  assert.equal((await fetch(origin + '/api/dramlo/session', { headers: {
    'X-Dramlo-UI': '1', Origin: 'http://external.invalid'
  } })).status, 403)
  let { raw, ok, file, headers } = await commands(origin)
  const projectDir = await ok('ensureProjectDir', { name: '隔离项目', id: 'fixture' })
  const projectPath = path.join(projectDir, 'project.json')
  const projectDefaults = JSON.parse(await fs.readFile(new URL('../src/types/projectDefaults.json', import.meta.url), 'utf8'))
  const original = {
    id: 'fixture', createdAt: 1, updatedAt: 1, name: 'Fixture', preservedExtensionField: true,
    script: null, timeline: [], settings: projectDefaults,
    canvas: { nodes: [], edges: [], groups: [], viewport: { x: 0, y: 0, zoom: 1 } },
    shots: [{ id: 'shot-1', compiledVideoPrompt: 'original', videoAssetId: 'keep-video', duration: 5 }]
  }
  await ok('writeText', { path: projectPath, value: JSON.stringify(original) })
  assert.deepEqual(await ok('listProjectDirs'), [projectDir])
  assert.equal((await raw('readText', { path: '../outside.json' })).status, 400)
  const next = { ...original, updatedAt: 2, name: 'Edited' }
  await ok('writeText', { path: projectPath, value: JSON.stringify(next), expectedUpdatedAt: 1 })
  assert.equal((await raw('writeText', { path: projectPath, value: JSON.stringify(original), expectedUpdatedAt: 1 })).status, 409)
  const manifestPath = path.join(projectDir,'assets','manifest.json')
  await fs.mkdir(path.dirname(manifestPath),{recursive:true})
  await Promise.all(['archive-a','import-b'].map(id=>ok('patchAssetManifest',{
    path:manifestPath,id,value:JSON.stringify({id,kind:'image'})
  })))
  assert.deepEqual(JSON.parse(await ok('readText',{path:manifestPath})).map(asset=>asset.id).sort(),['archive-a','import-b'])
  assert.equal((await raw('writeText',{path:manifestPath,value:'[]'})).status,400)
  await Promise.all(['archive-a','import-b'].map(id=>ok('patchAssetManifest',{path:manifestPath,id})))
  assert.deepEqual(JSON.parse(await ok('readText',{path:manifestPath})),[])
  console.log('PASS concurrent asset updates survive and stale manifest overwrites are rejected')
  await ok('kvSet', { name: 'test-key', value: 'persisted' })
  const videoPath = path.join(projectDir, 'assets', 'videos', 'fixture.mp4')
  assert.ok((await fetch(file(videoPath), { method: 'PUT', body: '0123456789', headers })).ok)
  const range = await fetch(file(videoPath), { headers: { Range: 'bytes=2-5' } })
  assert.equal(range.status, 206)
  assert.equal(await range.text(), '2345')
  const localCapabilities = await fetch(origin + '/api/dramlo/mcp-capabilities')
  assert.equal(localCapabilities.status, 200)
  const browserCapabilities = await fetch(origin + '/api/dramlo/mcp-capabilities', { headers: { Origin: origin } })
  assert.equal(browserCapabilities.status, 403)
  assert.equal(browserCapabilities.headers.get('content-type'), 'application/json; charset=utf-8')
  assert.match((await browserCapabilities.json()).error, /验证失败/)
  const mcpHeaders = { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' }
  const capabilities = await fetch(origin + '/api/dramlo/mcp-capabilities', { headers: mcpHeaders })
  assert.equal(capabilities.headers.get('content-type'), 'application/json; charset=utf-8')
  const { tools: capabilityTools, stdioOnlyTools } = await capabilities.json()
  assert.deepEqual(stdioOnlyTools,['studio_web_status','studio_start_web'])
  assert.ok(stdioOnlyTools.every(name=>!capabilityTools.includes(name)))
  const schemaTools = JSON.parse(await fs.readFile(new URL('../dramlo-server/mcp-tools.json', import.meta.url), 'utf8'))
  assert.deepEqual([...capabilityTools].sort(), schemaTools.map(tool => tool.name).sort())
  for (const name of ['studio_describe_model', 'studio_describe_extension_node', 'studio_update_extension_node', 'studio_replace_shots', 'studio_remove_model']) {
    assert.ok(capabilityTools.includes(name), 'Runtime capability missing: ' + name)
  }
  console.log('PASS deployed service advertises new tools (' + capabilityTools.length + ' total)')
  const malformed = await fetch(origin + '/api/dramlo/mcp', {
    method: 'POST', headers: mcpHeaders, body: JSON.stringify({ name: 'studio_project' })
  })
  assert.equal(malformed.status, 422)
  assert.equal(malformed.headers.get('content-type'), 'application/json; charset=utf-8')
  assert.match((await malformed.json()).error, /missing field/)
  const fixturePlugin = path.join(scratch, 'extension-plugin.cjs')
  await fs.writeFile(fixturePlugin, `
    const readline = require('node:readline').createInterface({ input: process.stdin });
    readline.on('line', line => {
      const request = JSON.parse(line);
      let result;
      if (request.method === 'initialize') {
        result = { protocolVersion: request.params.protocolVersion, capabilities: { tools: {} } };
      } else if (request.method === 'tools/list') {
        result = { tools: [{ name: 'shot_check', inputSchema: { type: 'object', properties: { shot: { type: 'string' } } } }] };
      } else if (request.method === 'tools/call' && request.params.name === 'shot_check') {
        const content = request.params.arguments.shot === 'image'
          ? [{ type: 'image', mimeType: 'image/png', data: 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aJ8sAAAAASUVORK5CYII=' }]
          : [{ type: 'text', text: 'inspection complete' }];
        result = { content,
          structuredContent: { checked: request.params.arguments.shot, inheritedToken: Boolean(process.env.DRAMLO_MCP_BRIDGE_TOKEN) } };
      }
      if (result) console.log(JSON.stringify({ jsonrpc: '2.0', id: request.id, result }));
    });
  `)
  const pluginBrowser = (name, input) => fetch(origin + '/api/dramlo/canvas-extensions', {
    method: 'POST', headers, body: JSON.stringify({ name, input })
  })
  const provider = { id: 'fixture-mcp', command: process.execPath, args: [fixturePlugin], tools: ['shot_check'] }
  assert.equal((await pluginBrowser('configure_mcp_provider', { provider, confirmed: false })).status, 400)
  assert.equal((await pluginBrowser('configure_mcp_provider', { provider, confirmed: true })).status, 200)
  const client = new Client({ name: 'dramlo-web-test', version: '1' })
  const transport = new StdioClientTransport({ command: binary, args: ['mcp', 'studio'],
    env: { ...env, DRAMLO_WEB_MCP_URL: origin } })
  let saved
  try {
    await client.connect(transport)
    const { tools } = await client.listTools()
    assert.deepEqual(tools.map(tool => tool.name).sort(), [...capabilityTools, ...stdioOnlyTools].sort())
    for (const name of ['studio_describe_model', 'studio_describe_extension_node', 'studio_update_extension_node', 'studio_replace_shots', 'studio_remove_model']) {
      assert.ok(tools.some(tool => tool.name === name), 'stdio tool missing: ' + name)
    }
    console.log('PASS packaged MCP client discovers new tools (' + tools.length + ' total)')
    assert.ok(tools.some(({ name }) => name === 'studio_set_video_prompt'))
    assert.ok(tools.some(({ name }) => name === 'studio_confirm_video'))
    const call = (name, args = {}) => client.callTool({ name, arguments: args })
    const guide = (await call('studio_get_guide')).structuredContent
    const liveStatus = (await call('studio_web_status')).structuredContent
    assert.match(liveStatus.toolRevision, /^[0-9a-f]{16}$/)
    assert.equal(liveStatus.guideRevision, guide.revision)
    assert.deepEqual(guide.sections.slice(0, 2).map(section => section.id), ['startup','models'])
    assert.equal(guide.sections.length, 7)
    const shotsSchema = tools.find(tool => tool.name === 'studio_shots').inputSchema
    assert.ok(!shotsSchema.required?.includes('offset') && !shotsSchema.required?.includes('limit'))
    const itemsSchema = tools.find(tool => tool.name === 'studio_project_items').inputSchema
    assert.deepEqual(itemsSchema.required,['projectId','type'])
    for (const { id } of guide.sections) {
      const chapter = (await call('studio_get_guide', { section: id })).structuredContent
      assert.equal(chapter.section, id)
      assert.equal(chapter.revision, guide.revision)
      assert.ok(chapter.body.length > 0)
    }
    const { items: skills, usage } = (await call('studio_list_skills')).structuredContent
    assert.match(usage, /影视短剧制作 Skill 库.*studio_read_skill.*禁止把 Dramlo Skill 安装/)
    assert.match(tools.find(tool => tool.name === 'studio_read_skill')?.description, /禁止在外部 Agent/)
    assert.ok(skills.length > 0)
    assert.ok(skills.every(skill => skill.stage && skill.trigger))
    assert.deepEqual(skills.filter(skill => skill.source === 'studio-built-in').map(skill => skill.id).sort(), [
      'ai-short-film-screenwriter', 'character-consistency', 'drama-flow-explore', 'h3-prompt-writing', 'pipeline-batch',
      'qa-verify', 'shot-image', 'shot-video', 'storyboard-director', 'storyboard-panel'
    ])
    for (const entry of skills.filter(item => item.source === 'studio-built-in')) {
      const {structuredContent: fresh} = await call('studio_read_skill',{skillId:entry.id})
      assert.equal(fresh.trigger,entry.trigger)
      assert.doesNotMatch(fresh.body,/\bstudio_[a-z_]+|\b(?:generate_[a-z_]+|extract_cast|approve_play)\b/,entry.id)
    }
    const drama = skills.find(item => item.id === 'drama-flow-explore')
    const dramaRoot = path.join(cwd, 'src', 'skills', 'drama-flow-explore')
    const expectedReferences = (await fs.readdir(path.join(dramaRoot, 'references')))
      .filter(name => name.endsWith('.md')).map(name => 'references/' + name).sort()
    assert.deepEqual([...drama.references].sort(), expectedReferences)
    const uiSkillResponse = await fetch(origin + '/api/dramlo/creative-skills?skillId=drama-flow-explore', {
      headers: { 'X-Dramlo-UI': '1' }
    })
    assert.equal(uiSkillResponse.status, 200)
    const uiSkill = await uiSkillResponse.json()
    assert.deepEqual(uiSkill.references, drama.references)
    for (const referencePath of drama.references) {
      const { structuredContent: reference } = await call('studio_read_skill', { skillId: drama.id, referencePath })
      assert.equal(reference.referencePath, referencePath)
      assert.equal(reference.body, await fs.readFile(path.join(dramaRoot, referencePath), 'utf8'))
      assert.equal(reference.revision, drama.revision)
    }
    const productionGuide = (await call('studio_get_guide',{section:'production'})).structuredContent.body
    assert.match(productionGuide,/一次确认启动整份已核对的计划/)
    assert.match(productionGuide,/结果未知时保留请求 ID/)

    const skill = (await call('studio_read_skill', { skillId: skills[0].id })).structuredContent
    assert.match(skill.revision, /^[0-9a-f]{16}$/)
    assert.equal(skill.revision, skills[0].revision)
    assert.equal(skill.usage, usage)
    const pluginCall = async (name, args) => {
      const reply = await call(name, args)
      assert.ok(!reply.isError, JSON.stringify(reply.content))
      return reply.structuredContent
    }
    const pluginManifest = { formatVersion: 1, id: 'fixture.shot-check', version: 1,
      name: 'Fixture MCP', description: 'Checks a shot', executor: 'studio.mcp', providerId: 'fixture-mcp', tool: 'shot_check' }
    const installPlugin = await pluginCall('studio_extension_install', { manifest: pluginManifest })
    assert.equal(installPlugin.tool, 'shot_check')
    const zipKey = { typeId: pluginManifest.id, version: 1 }
    const archive = await pluginCall('studio_extension_package_export', zipKey)
    assert.equal(Buffer.from(archive.packageBase64, 'base64').subarray(0, 2).toString(), 'PK')
    assert.equal((await pluginCall('studio_extension_package_preview', archive)).items[0].id, pluginManifest.id)
    await pluginCall('studio_extension_package_install', archive)
    assert.ok((await call('studio_extension_package_preview', { packageBase64: 'aW52YWxpZA==' })).isError)
    const traversalZip = Buffer.from(archive.packageBase64, 'base64')
    const originalName = Buffer.from('extension.json')
    const replacement = Buffer.from('../escape.json')
    assert.equal(originalName.length, replacement.length)
    for (let offset = traversalZip.indexOf(originalName); offset >= 0; offset = traversalZip.indexOf(originalName, offset + replacement.length)) {
      replacement.copy(traversalZip, offset)
    }
    assert.ok((await call('studio_extension_package_preview', { packageBase64: traversalZip.toString('base64') })).isError)
    assert.ok((await call('studio_extension_preview', { manifest: { ...pluginManifest, tool: 'other_tool' } })).isError)

    const pluginKey = { ...zipKey, projectId: 'fixture' }
    const args = { ...pluginKey, arguments: { shot: 'fixture-shot' } }
    assert.ok((await call('studio_call_extension_tool', args)).isError)
    await pluginCall('studio_extension_enable', { ...pluginKey, enabled: true })
    const described = await pluginCall('studio_describe_extension_tool', pluginKey)
    assert.equal(described.inputSchema.properties.shot.type, 'string')
    const result = await pluginCall('studio_call_extension_tool', args)
    assert.deepEqual(result.structuredContent, { checked: 'fixture-shot', inheritedToken: false })
    await pluginCall('studio_extension_enable', { ...pluginKey, enabled: false })
    assert.ok((await call('studio_call_extension_tool', args)).isError)
    const mediaProject = await pluginCall('studio_create_project', { name: 'MCP image fixture' })
    const imagePlugin = { ...pluginManifest, id: 'fixture.shot-image', outputKind: 'image' }
    await pluginCall('studio_extension_install', { manifest: imagePlugin })
    const imageKey = { projectId: mediaProject.projectId, typeId: imagePlugin.id, version: 1 }
    await pluginCall('studio_extension_enable', { ...imageKey, enabled: true })
    const imageNode = await pluginCall('studio_add_extension_node', { ...imageKey,
      nodeId: randomUUID(), expectedUpdatedAt: mediaProject.updatedAt })
    const imageOutput = await pluginCall('studio_run_extension_node', { projectId: imageKey.projectId,
      nodeId: imageNode.node.id, expectedUpdatedAt: imageNode.updatedAt, arguments: { shot: 'image' } })
    assert.equal(imageOutput.result.content[0].assetId, imageOutput.assetId)
    const media = await pluginCall('studio_project_items', { projectId: imageKey.projectId, type: 'assets' })
    assert.equal(media.items[0].mimeType, 'image/png')
    await pluginCall('studio_extension_remove', { typeId: imagePlugin.id, version: 1, confirmed: true })
    assert.equal((await pluginCall('studio_project_items', { projectId: imageKey.projectId, type: 'nodes' })).items[0].data.outputAssetId, imageOutput.assetId)
    assert.equal((await pluginBrowser('remove_mcp_provider', { providerId: provider.id, confirmed: true })).status, 200)
    assert.equal((await pluginBrowser('configure_mcp_provider', { provider, confirmed: true })).status, 400)
    await pluginCall('studio_open_project', { projectId: 'fixture' })
    console.log('PASS authorized local MCP process, project scope, secret environment isolation and ZIP round-trip')

    assert.equal((await call('studio_project')).structuredContent.project.id, 'fixture')
    assert.equal((await call('studio_project', { projectId: 'fixture' })).structuredContent.project.id, 'fixture')
    const wrongProject = await call('studio_project', { projectId: imageKey.projectId })
    assert.equal(wrongProject.isError, true)
    assert.match(wrongProject.content[0].text, /请先打开指定项目/)
    assert.equal((await call('studio_project')).structuredContent.project.id, 'fixture')
    const spare = await pluginCall('studio_create_project',{name:'仅用于移除验证'})
    assert.equal((await pluginCall('studio_delete_project',{projectId:spare.projectId})).blocked,true)
    await pluginCall('studio_open_project',{projectId:'fixture'})
    const removal = await pluginCall('studio_delete_project',{projectId:spare.projectId})
    assert.equal(removal.removed,false)
    assert.equal(removal.blocked,false)
    const autoRemoval = await call('studio_delete_project',{projectId:spare.projectId,expectedUpdatedAt:'auto',confirmed:true})
    assert.equal(autoRemoval.isError,true)
    assert.equal(autoRemoval.structuredContent.code,'invalid')
    assert.equal((await pluginCall('studio_delete_project',{projectId:spare.projectId})).updatedAt,removal.updatedAt)
    const deleted = await pluginCall('studio_delete_project',{projectId:spare.projectId,expectedUpdatedAt:removal.updatedAt,confirmed:true})
    assert.equal(deleted.removed,true)
    assert.equal(JSON.parse(await fs.readFile(path.join(deleted.recoveryPath,'project.json'),'utf8')).id,spare.projectId)
    assert.ok(!(await pluginCall('studio_list_projects',{})).items.some(item=>item.id===spare.projectId))
    const editing = await pluginCall('studio_create_project',{name:'分镜批量编辑夹具'})
    const editKey={projectId:editing.projectId}
    let editVersion=(await pluginCall('studio_import_script',{...editKey,expectedUpdatedAt:editing.updatedAt,title:'夹具剧本',content:'示例场景。'})).updatedAt
    const editScene=randomUUID()
    editVersion=(await pluginCall('studio_upsert_project_assets',{...editKey,expectedUpdatedAt:editVersion,items:[
      {id:editScene,type:'scene',name:'测试场景',description:'可见的测试场景'}
    ]})).updatedAt
    const shotIds=[randomUUID(),randomUUID()]
    editVersion=(await pluginCall('studio_append_shots',{...editKey,expectedUpdatedAt:editVersion,sceneId:editScene,shots:[
      {id:shotIds[0],title:'01｜0:09-0:12｜第一镜',visualPrompt:'原稿一',duration:3},
      {id:shotIds[1],title:'02｜0:12-0:15｜第二镜',visualPrompt:'原稿二',duration:3}
    ]})).updatedAt
    const batchKey={...editKey,expectedUpdatedAt:editVersion}
    const failedEdit=await call('studio_batch_edit_shots',{...batchKey,edits:[
      {kind:'video-prompt',shotId:'#01',prompt:'不应保存'},
      {kind:'inputs',shotId:'#03',duration:5}
    ]})
    assert.equal(failedEdit.isError,true)
    assert.match(failedEdit.content[0].text,/第 2 项/)
    const beforeBatch=await pluginCall('studio_shots',{...editKey,shotId:'#01',fields:['compiledVideoPrompt','duration']})
    assert.equal(beforeBatch.items[0].compiledVideoPrompt,null)
    assert.equal(beforeBatch.updatedAt,editVersion)
    const savedBatch=await pluginCall('studio_batch_edit_shots',{...batchKey,edits:[
      {kind:'inputs',shotId:'#01',duration:5},
      {kind:'video-prompt',shotId:'#01',prompt:'视频最终稿'},
      {kind:'image-prompt',shotId:'#01',visualPrompt:'独立关键帧'},
      {kind:'image-prompt',shotId:'#02',visualPrompt:'同步分镜正文',syncScript:true}
    ]})
    assert.equal(savedBatch.items.length,4)
    const overview=await pluginCall('studio_shots',{...editKey,detail:'overview'})
    assert.equal(overview.items[0].hasImagePrompt,true)
    assert.equal(overview.items[0].hasVideoPrompt,true)
    assert.equal(overview.items[0].hasImage,false)
    assert.equal(overview.items[0].hasVideo,false)
    assert.equal(overview.items[1].plannedStartSec,5)
    assert.equal(overview.items[1].title,'02｜0:05-0:08｜第二镜')
    assert.ok(!Object.hasOwn(overview.items[0],'compiledVideoPrompt'))
    const projected=await pluginCall('studio_shots',{...editKey,fields:['visualPrompt','imagePrompt','duration'],shotId:'#02'})
    assert.deepEqual(projected.items,[{id:shotIds[1],shotNumber:2,visualPrompt:'同步分镜正文',imagePrompt:'同步分镜正文',duration:3}])
    assert.equal((await pluginCall('studio_shots',{...editKey,shotId:'#01'})).items[0].visualPrompt,'原稿一')
    const misplacedBatch=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
      body:JSON.stringify({name:'studio_set_asset_prompt',input:{...editKey,expectedUpdatedAt:'auto',items:[]}})})
    assert.equal(misplacedBatch.status,400)
    assert.match((await misplacedBatch.json()).error,/1 至 100/)
    for (let page=0;page<3;page++) {
      const added=await pluginCall('studio_append_shots',{...editKey,expectedUpdatedAt:
        (await pluginCall('studio_project',editKey)).project.updatedAt,sceneId:editScene,
        shots:Array.from({length:9},(_,index)=>({id:randomUUID(),title:'镜头 '+(page*9+index+3),visualPrompt:'画面',duration:3}))})
      assert.ok(added.updatedAt)
    }
    const bulk=await pluginCall('studio_batch_edit_shots',{...editKey,expectedUpdatedAt:'auto',edits:
      Array.from({length:29},(_,index)=>({kind:'image-prompt',shotId:'#'+(index+1),visualPrompt:'第 '+(index+1)+' 镜正文'}))})
    assert.equal(bulk.items.length,29)
    const overviewPage=await pluginCall('studio_shots',{...editKey,detail:'overview'})
    assert.equal(overviewPage.items.length,29)
    assert.equal(overviewPage.nextOffset,null)
    assert.equal(overviewPage.items[28].shotNumber,29)
    const overviewOnly=await pluginCall('studio_project_overview',editKey)
    assert.equal(overviewOnly.items.length,29)
    assert.ok(Buffer.byteLength(JSON.stringify(overviewOnly))<10000)
    assert.deepEqual(Object.keys(overviewOnly.items[0]).sort(),['duration','hasImage','hasImagePrompt','hasPrompt','hasVideo','plannedStart','shotNumber','subjects'])
    const itemsReply=await pluginCall('studio_set_asset_prompt',{...editKey,expectedUpdatedAt:'auto',items:
      Array.from({length:5},(_,index)=>({targetType:'shot-image',targetId:'#'+(index+1),visualPrompt:'批量调整 '+index}))})
    assert.equal(itemsReply.items.length,5)
    assert.equal((await pluginCall('studio_shots',{...editKey,shotId:'#05'})).items[0].imagePrompt,'批量调整 4')
    assert.deepEqual(overviewPage.items[28].boundSubjects,[])
    assert.equal((await pluginCall('studio_shots',editKey)).items.length,20)
    await pluginCall('studio_open_project',{projectId:'fixture'})
    console.log('PASS global shot selectors, partial fields, production overview, batch rollback, prompt sync and timed titles')
    const testEndpoint = 'https://models.example.invalid/v1'
    const {profileId:credentialProfile} = await pluginCall('studio_add_model',{kind:'llm',name:'Credential fixture',
      baseUrl:testEndpoint,model:'fixture-text',select:false,adapterType:'openai-compatible'})
    const credential = {kind:'llm',profileId:credentialProfile,expectedBaseUrl:testEndpoint,apiKey:randomBytes(16).toString('hex')}
    assert.deepEqual(await pluginCall('studio_set_credential',credential),{kind:'llm',profileId:credentialProfile,hasCredential:true})
    assert.equal((await call('studio_set_credential',{...credential,expectedBaseUrl:'https://other.example.invalid'})).isError,true)
    const credentialCheck = await pluginCall('studio_describe_model',{kind:'llm',profileId:credentialProfile})
    assert.equal(credentialCheck.hasCredential,true)
    assert.ok(!JSON.stringify(credentialCheck).includes(credential.apiKey))
    assert.equal((await pluginCall('studio_set_credential',{...credential,apiKey:null})).hasCredential,false)
    console.log('PASS project removal with recovery and write-only provider credential')

    const edit = { projectId: 'fixture', shotId: 'shot-1', prompt: 'manual revision', expectedUpdatedAt: 2 }
    const badRevision = await call('studio_set_video_prompt', {...edit,expectedUpdatedAt:'typo'})
    assert.equal(badRevision.isError,true)
    assert.equal(badRevision.structuredContent.code,'invalid')
    assert.equal((await pluginCall('studio_project',{projectId:'fixture'})).project.updatedAt,edit.expectedUpdatedAt)
    const changed = await call('studio_set_video_prompt', {...edit,expectedUpdatedAt:'auto'})
    assert.ok(!changed.isError, JSON.stringify(changed.content))
    const conflicted = await call('studio_set_video_prompt', edit)
    assert.equal(conflicted.isError, true)
    assert.equal(conflicted.structuredContent.code, 'conflict')
    assert.equal(conflicted.structuredContent.currentUpdatedAt,changed.structuredContent.updatedAt)
    assert.equal(conflicted.structuredContent.expectedUpdatedAt,2)
    assert.match(conflicted.content[0].text,new RegExp(String(changed.structuredContent.updatedAt)))
    const staleDefaults = await fetch(origin + '/api/dramlo/mcp',{
      method:'POST',headers:mcpHeaders,body:JSON.stringify({name:'studio_update_project_defaults',
        input:{projectId:'fixture',expectedUpdatedAt:2,aspectRatio:'9:16'}})
    })
    assert.equal(staleDefaults.status,409)
    const conflict = await staleDefaults.json()
    assert.equal(conflict.currentUpdatedAt,changed.structuredContent.updatedAt)
    assert.equal(conflict.expectedUpdatedAt,2)
    const compact = (await call('studio_shots', { projectId:'fixture', detail:'summary' })).structuredContent
    const fullShots = (await call('studio_shots', { projectId:'fixture' })).structuredContent
    assert.equal(compact.items[0].id, fullShots.items[0].id)
    assert.equal(compact.items[0].compiledVideoPrompt, undefined)
    assert.equal(compact.items[0].plannedEndSec, 5)
    assert.equal(fullShots.items[0].compiledVideoPrompt, 'manual revision')

    saved = JSON.parse(await ok('readText', { path: projectPath }))
    assert.equal(saved.shots[0].compiledVideoPrompt, 'manual revision')
    assert.equal(saved.shots[0].videoAssetId, 'keep-video')
    assert.equal((await ok('getActiveProject')).updatedAt, changed.structuredContent.updatedAt)
    await ok('kvSet', { name: 'settings', value: JSON.stringify({
      activeVideoId: 'fixture-video', videos: [{
        id: 'fixture-video', baseUrl: 'http://127.0.0.1:' + mock.address().port,
        adapter: {
          type: 'comfyui-prompt',
          workflowTemplate: JSON.stringify({ '1': { class_type: 'ExampleVideo', inputs: { text: 'old', seconds: 2 } } }),
          inputMap: { bindings: [{ role: 'prompt', nodeId: '1', field: 'text' },
            { role: 'length', nodeId: '1', field: 'seconds', durationUnit: 'seconds' }] }
        }
      }]
    }) })
    const modelSummary = (await call('studio_project')).structuredContent.project
    assert.equal(modelSummary.effectiveVideo.id, 'fixture-video')
    assert.equal(modelSummary.effectiveVideo.apiKey, undefined)
    assert.equal(modelSummary.effectiveImage, null)
    const addInactive = (await call('studio_add_model', {kind:'image', name:'Inactive image fixture',
      baseUrl:'http://127.0.0.1:9999',model:'fixture',select:false,providerName:'Custom fixture',adapterType:'openai-images'})).structuredContent
    assert.equal(addInactive.activeId,null)
    const configuredInactive = addInactive.profiles.find(item => item.id === addInactive.profileId)
    assert.equal(configuredInactive.adapter, 'openai-images')
    assert.equal(configuredInactive.providerName, 'Custom fixture')
    assert.equal(configuredInactive.serverExecutionSupported, true)
    const inactiveDiagnosis = await call('studio_describe_model', {kind:'image',profileId:addInactive.profileId})
    assert.ok(!inactiveDiagnosis.isError, JSON.stringify(inactiveDiagnosis.content))
    assert.equal(inactiveDiagnosis.structuredContent.active,false)
    assert.equal(inactiveDiagnosis.structuredContent.ready,false)
    assert.equal(inactiveDiagnosis.structuredContent.endpointIssue,null)
    assert.match(inactiveDiagnosis.structuredContent.inputIssue,/密钥/)
    assert.equal(inactiveDiagnosis.structuredContent.connectionChecked,false)
    assert.equal(inactiveDiagnosis.structuredContent.missingMapping,null)
    const videoDiagnosis = await call('studio_describe_model', {kind:'video',profileId:'fixture-video'})
    assert.ok(!videoDiagnosis.isError, JSON.stringify(videoDiagnosis.content))
    assert.equal(videoDiagnosis.structuredContent.ready,true)
    assert.deepEqual(videoDiagnosis.structuredContent.missingMapping,[])
    assert.deepEqual(videoDiagnosis.structuredContent.workflow.missingMapping,videoDiagnosis.structuredContent.missingMapping)
    assert.equal(videoDiagnosis.structuredContent.inputMap.bindings.length,2)
    assert.ok(!Object.hasOwn(videoDiagnosis.structuredContent,'apiKey'))
    assert.ok(!Object.hasOwn(videoDiagnosis.structuredContent,'workflowTemplate'))

    const patchInactive = (await call('studio_model_update', {kind:'image',profileId:addInactive.profileId,
      model:'updated',name:'Renamed inactive fixture',provider:'openai',providerName:null})).structuredContent
    assert.equal(patchInactive.activeId,null)
    const renamed = patchInactive.profiles.find(p=>p.id===addInactive.profileId)
    assert.equal(renamed.model,'updated')
    assert.equal(renamed.name,'Renamed inactive fixture')
    assert.equal(renamed.provider,'openai')
    assert.equal(renamed.providerName,null)
    const deleteFixture = {kind:'image',profileId:addInactive.profileId}
    const deletionPreview = (await call('studio_remove_model',deleteFixture)).structuredContent
    assert.equal(deletionPreview.blocked,false)
    assert.equal(deletionPreview.removed,false)
    assert.equal(deletionPreview.ready,false)
    assert.equal(deletionPreview.missingMapping,null)
    assert.equal(deletionPreview.mappingChecked,false)
    assert.equal(deletionPreview.hasCredential,false)
    assert.equal((await call('studio_remove_model',{...deleteFixture,confirmed:true})).structuredContent.removed,true)
    assert.ok(!(await call('studio_list_models',{kind:'image'})).structuredContent.profiles.some(p=>p.id===addInactive.profileId))
    const protectedDefault = (await call('studio_remove_model',{kind:'video',profileId:'fixture-video',confirmed:true})).structuredContent
    assert.equal(protectedDefault.active,true)
    assert.equal(protectedDefault.removed,false)

    const requestId = randomUUID()
    const key = { projectId: 'fixture', requestId }
    const discardedVideo = { ...key, requestId:randomUUID() }
    const videoProbe={...discardedVideo,shots:[{shotId:'shot-1',mode:'text'}]}
    const staleVideo=await call('studio_prepare_video',{...videoProbe,expectedUpdatedAt:saved.updatedAt-1})
    assert.equal(staleVideo.isError,true)
    assert.equal(staleVideo.structuredContent.code,'conflict')
    assert.equal(staleVideo.structuredContent.expectedUpdatedAt,saved.updatedAt-1)
    assert.equal(staleVideo.structuredContent.currentUpdatedAt,saved.updatedAt)
    const disposal = await call('studio_prepare_video', { ...videoProbe, expectedUpdatedAt:'auto' })
    assert.equal(disposal.structuredContent.batch.status,'prepared')
    assert.equal(disposal.structuredContent.batch.revision,saved.updatedAt)
    assert.equal(submits,0)
    assert.equal((await call('studio_discard_video_plan',discardedVideo)).structuredContent.batch.status,'discarded')
    assert.equal((await call('studio_discard_video_plan',discardedVideo)).structuredContent.reused,true)
    assert.equal((await call('studio_confirm_video',{...discardedVideo,confirmed:true})).isError,true)
    assert.ok(!(await call('studio_remove_model',{kind:'video',profileId:'fixture-video'}))
      .structuredContent.jobs.some(job=>job.requestId===discardedVideo.requestId))
    const prepared = await call('studio_prepare_video', { ...key, expectedUpdatedAt: saved.updatedAt,
      shots: [{ shotId: 'shot-1', mode: 'text' }] })
    assert.ok(!prepared.isError, JSON.stringify(prepared.content))
    assert.equal(submits, 0)
    const started = await call('studio_confirm_video', { ...key, confirmed: true })
    assert.ok(!started.isError, JSON.stringify(started.content))
    let status
    for (let attempt = 0; attempt < 50; attempt++) {
      status = (await call('studio_video_status', key)).structuredContent.batch
      if (status.steps[0].promptId === 'fixture-task') break
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    assert.equal(status.steps[0].promptId, 'fixture-task', JSON.stringify(status))
    const cancelFromWeb = async (input) => fetch(origin + '/api/dramlo/video-queue', {
      method: 'POST', headers: { 'X-Dramlo-UI': '1', 'Content-Type': 'application/json' },
      body: JSON.stringify({ name: 'studio_cancel_video_shot', input: { ...key, confirmed: true, ...input } })
    })
    assert.equal((await cancelFromWeb({})).status, 400)
    const staleStop = await cancelFromWeb({ expectedPromptId: 'previous-shot' })
    assert.equal(staleStop.status, 409)
    assert.match((await staleStop.json()).error, /当前镜头已变化/)
    assert.equal((await call('studio_video_status', key)).structuredContent.batch.pauseRequested, false)
    await stop()
    origin = await start()
    ;({ ok } = await commands(origin))
    ready = true
    const invoke = async (name, input) => {
      const result = await fetch(origin + '/api/dramlo/mcp', { method: 'POST',
        headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
        body: JSON.stringify({ name, input }) })
      assert.ok(result.ok, await result.clone().text())
      return result.json()
    }
    assert.equal((await invoke('studio_video_status', key)).batch.status, 'paused')
    const discardedStatus = (await invoke('studio_video_status', discardedVideo)).batch
    assert.equal(discardedStatus.status, 'discarded')
    assert.ok(discardedStatus.steps.every(step=>step.status==='discarded'))
    const queuedRemoval = { projectId: 'fixture', shotId: 'shot-1', expectedUpdatedAt: saved.updatedAt }
    assert.equal((await invoke('studio_remove_shot', queuedRemoval)).queue.blocked, true)
    const deniedRemoval = await fetch(origin + '/api/dramlo/mcp', {
      method: 'POST', headers: mcpHeaders, body: JSON.stringify({
        name: 'studio_remove_shot', input: { ...queuedRemoval, confirmed: true }
      })
    })
    assert.equal(deniedRemoval.status, 400)
    assert.match((await deniedRemoval.json()).error, /暂停/)

    await invoke('studio_resume_video', { ...key, confirmed: true })
    for (let attempt = 0; attempt < 50; attempt++) {
      status = (await invoke('studio_video_status', key)).batch
      if (status.status !== 'running') break
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    assert.equal(status.status, 'succeeded', JSON.stringify(status))
    assert.equal(submits, 1)
    const located = await invoke('studio_locate_asset', { projectId: 'fixture', assetId: status.steps[0].assetId })
    assert.deepEqual(await fs.readFile(located.path), movie)
    saved = JSON.parse(await ok('readText', { path: projectPath }))
    assert.equal(saved.shots[0].videoAssetId, 'keep-video')
    assert.deepEqual(saved.shots[0].videoHistoryIds, [status.steps[0].assetId])
    assert.deepEqual(status.steps[0].bind, {
      slot:'video',applied:false,reason:'existing_asset',current:'keep-video',isCurrent:false
    })
    const used = await invoke('studio_bind_shot_asset',{
      projectId:'fixture',shotId:'shot-1',assetId:status.steps[0].assetId,slot:'video',expectedUpdatedAt:saved.updatedAt
    })
    assert.equal(used.bind.previous,'keep-video')
    const rechecked = await invoke('studio_confirm_video',{...key,confirmed:true})
    assert.equal(rechecked.reused,true)
    assert.equal(rechecked.batch.steps[0].bind.applied,false)
    assert.equal(rechecked.batch.steps[0].bind.isCurrent,true)
    assert.equal(submits,1)
    saved = JSON.parse(await ok('readText', { path: projectPath }))

    const localUrl = 'http://127.0.0.1:' + mock.address().port
    const priorSettings = await ok('kvGet', { name: 'settings' })
    const model = await invoke('studio_add_model', { kind: 'video', name: 'mapped fixture', baseUrl: localUrl, model: 'test' })
    assert.notEqual(model.profileId, 'fixture-video')
    const rejected = await fetch(origin + '/api/dramlo/fs', {
      method: 'POST', headers,
      body: JSON.stringify({ command: 'kvSet', name: 'settings', compareValue: true,
        expectedValue: priorSettings, value: priorSettings })
    })
    assert.equal(rejected.status, 409)
    await invoke('studio_select_model', { kind: 'video', profileId: 'fixture-video' })

    if (env.DRAMLO_FFMPEG_BIN) {
      const run = promisify(execFile)
      const sample = path.join(scratch, 'tail-fixture.mp4')
      const png = path.join(scratch, 'ref.png')
      await run(env.DRAMLO_FFMPEG_BIN, ['-v', 'error', '-y', '-f', 'lavfi', '-i', 'color=c=blue:s=64x64:d=1.5',
        '-c:v', 'libx264', '-pix_fmt', 'yuv420p', sample], { windowsHide: true })
      await run(env.DRAMLO_FFMPEG_BIN, ['-v', 'error', '-y', '-i', sample, '-frames:v', '1', '-threads', '1', png], { windowsHide: true })
      movie = await fs.readFile(sample)
      const work = await invoke('studio_create_project', { name: '两场三镜夹具' })
      const projectId = work.projectId
      const firstScene = randomUUID()
      const nextScene = randomUUID()
      const s1 = randomUUID(), s2 = randomUUID(), s3 = randomUUID()
      const propId = randomUUID()
      let revision = (await invoke('studio_import_script', { projectId, expectedUpdatedAt: work.updatedAt,
        title: '测试', content: '一场内接续，随后切换场地。' })).updatedAt
      revision = (await invoke('studio_upsert_project_assets', { projectId, expectedUpdatedAt: revision, items: [
        { id: firstScene, type: 'scene', name: '甲场', description: '内景' },
        { id: nextScene, type: 'scene', name: '乙场', description: '外景' },
        { id: propId, type: 'prop', name: '样例道具', description: '等待设计图' }
      ] })).updatedAt
      const draft = (id) => ({ id, title: id, visualPrompt: '蓝色测试镜头', duration: 5, cameraMovement: 'static' })
      revision = (await invoke('studio_append_shots', { projectId, expectedUpdatedAt: revision,
        sceneId: firstScene, shots: [{ ...draft(s1), propIds: [propId] }, draft(s2)] })).updatedAt
      revision = (await invoke('studio_append_shots', { projectId, expectedUpdatedAt: revision,
        sceneId: nextScene, shots: [draft(s3)] })).updatedAt
      const deniedEvents = await fetch(origin + '/api/dramlo/project-events', { headers: { 'sec-fetch-site': 'cross-site' } })
      assert.equal(deniedEvents.status,403)
      const eventStream = await fetch(origin + '/api/dramlo/project-events',{
        headers:{'sec-fetch-site':'same-origin'},signal:AbortSignal.timeout(6000)})
      assert.equal(eventStream.status,200)
      assert.match(eventStream.headers.get('content-type'),/text\/event-stream/)
      const reader = eventStream.body.getReader()
      let eventBuffer = ''
      async function eventRevision() {
        while (!eventBuffer.includes('\n\n')) {
          const next = await reader.read()
          assert.equal(next.done,false)
          eventBuffer += Buffer.from(next.value).toString()
        }
        const end = eventBuffer.indexOf('\n\n')
        const body = eventBuffer.slice(0,end)
        eventBuffer = eventBuffer.slice(end+2)
        return JSON.parse(body.split('\n').find(line => line.startsWith('data:')).slice(5))
      }
      try {
        assert.equal((await eventRevision()).projectId,projectId)
        const selected = await invoke('studio_shots',{projectId,shotIds:['#03','#02'],fields:['title']})
        assert.deepEqual(selected.items.map(shot=>shot.id),[s2,s3])
        const ignoredSelector = await fetch(origin + '/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
          body:JSON.stringify({name:'studio_shots',input:{projectId,shotNumber:2}})})
        assert.equal(ignoredSelector.status,400)
        const write = await invoke('studio_batch_edit_shots',{projectId,expectedUpdatedAt:'auto',edits:[
          {kind:'script-prompt',shotId:'#03',visualPrompt:'蓝色测试镜头'},
          {kind:'image-prompt',shotId:s3,visualPrompt:'单幅分镜视觉正文'},
          {kind:'inputs',shotId:s2,duration:5}]})
        revision = write.updatedAt
        let revisionEvent = await eventRevision()
        while (revisionEvent.projectId === projectId && revisionEvent.updatedAt < revision) {
          revisionEvent = await eventRevision()
        }
        assert.equal(revisionEvent.projectId,projectId)
        assert.equal(revisionEvent.updatedAt,revision)
        const [shot] = (await invoke('studio_shots',{projectId,shotId:'#03'})).items
        assert.equal(shot.visualPrompt,'蓝色测试镜头')
        assert.equal(shot.imagePrompt,'单幅分镜视觉正文')
      } finally { await reader.cancel() }
      const notReady = await invoke('studio_check_video_readiness', { projectId, shots: [{ shotId: s1, mode: 'reference' }] })
      assert.equal(notReady.ready, false)
      assert.match(notReady.items[0].issue, /最终视频提示词/)
      assert.equal(notReady.updatedAt, revision)
      const defaults = await invoke('studio_project_settings', { projectId, shotId: s1 })
      assert.equal(defaults.shotInputs.effectiveAudioMode, 'none')
      assert.equal(defaults.shotInputs.missingPropImages[0].id, propId)
      assert.equal(defaults.shotInputs.subjectBindingMode, 'infer')
      const updatedInputs = await invoke('studio_update_shot_inputs', {
        projectId, shotId: s1, expectedUpdatedAt: revision, audioMode: 'none', subjectBindingMode: 'explicit',
        subjectIds: [], h3GenMode: 'fl2va', videoStillMode: 'keyframe'
      })
      assert.equal(updatedInputs.shotInputs.canvasInputMode, 'first-last-frame')
      assert.equal(updatedInputs.shotInputs.stillMode, 'keyframe')
      assert.equal(updatedInputs.shotInputs.subjectBindingMode, 'explicit')
      assert.deepEqual(updatedInputs.shotInputs.subjectIds, [])
      revision = updatedInputs.updatedAt

      for (const id of [s1, s2, s3]) {
        revision = (await invoke('studio_set_video_prompt', { projectId, expectedUpdatedAt: revision, shotId: id, prompt: 'test ' + id })).updatedAt
      }
      const active = await invoke('studio_project_settings', { projectId })
      const assets = path.join(active.folderPath, 'assets')
      await fs.mkdir(path.join(assets, 'images'), { recursive: true })
      await fs.copyFile(png, path.join(assets, 'images', 'ref.png'))
      await fs.writeFile(path.join(assets, 'manifest.json'), JSON.stringify([
        { id: 'ref', kind: 'image', fileRelPath: 'assets/images/ref.png' }
      ]))
      const template = (firstFrame) => ({
        '1': { class_type: 'CR Prompt Text', inputs: { prompt: 'old' } },
        '2': { class_type: 'PrimitiveFloat', inputs: { value: 3 }, _meta: { title: 'duration' } },
        '3': { class_type: 'LoadImage', inputs: { image: 'old.png' } },
        '4': { class_type: 'ResizeImage', inputs: { image: ['3', 0] } },
        '5': { class_type: 'MiniMaxH3Example', inputs: { [firstFrame ? 'first_frame' : 'ref_images.ref_image_0']: ['4', 0] } }
      })
      const profiles = []
      for (const firstFrame of [false, true]) {
        const config = await invoke('studio_add_model', { kind: 'video', name: firstFrame ? 'I2VA' : 'Ref2VA', baseUrl: localUrl, model: 'test' })
        await invoke('studio_model_update', { kind: 'video', profileId: config.profileId, adapterType: 'comfyui-prompt',
          workflowTemplate: JSON.stringify(template(firstFrame)) })
        profiles.push(config.profileId)
      }
      const refDiagnosis = await invoke('studio_describe_model',{kind:'video',profileId:profiles[0]})
      assert.deepEqual(refDiagnosis.workflow.referenceSlots.map(({nodeId,pictureNumber})=>[nodeId,pictureNumber]),[['3',1]])
      const firstDiagnosis = await invoke('studio_describe_model',{kind:'video',profileId:profiles[1]})
      assert.deepEqual(firstDiagnosis.workflow.referenceSlots,[])
      assert.equal(firstDiagnosis.workflow.firstFrameSlot.nodeId,'3')
      const badTemplate = template(true)
      badTemplate['3'].inputs.unrelated = 'untouched'
      await invoke('studio_model_update', { kind: 'video', profileId: profiles[1],
        workflowTemplate: JSON.stringify(badTemplate), inputMap: { bindings: [
          { role: 'prompt', nodeId: '1', field: 'prompt' },
          { role: 'length', nodeId: '2', field: 'value', durationUnit: 'seconds' },
          { role: 'image', nodeId: '3', field: 'unrelated' }
        ] } })
      const wrongSlot = await fetch(origin + '/api/dramlo/mcp', { method: 'POST',
        headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
        body: JSON.stringify({ name: 'studio_prepare_video', input: {
          projectId, requestId: randomUUID(), expectedUpdatedAt: revision,
          shots: [{ shotId: s2, profileId: profiles[1], mode: 'first-frame', firstFrameAssetId: 'ref' }]
        } }) })
      assert.equal(wrongSlot.status, 400)
      assert.match((await wrongSlot.json()).error, /first_frame/)
      await invoke('studio_model_update', { kind: 'video', profileId: profiles[1],
        workflowTemplate: JSON.stringify(template(true)), inputMap: { bindings: [] } })
      const pinned = await invoke('studio_update_shot_inputs', {
        projectId, shotId:s1, expectedUpdatedAt:revision, videoProfileId:profiles[0], duration:6
      })
      assert.equal(pinned.shotInputs.videoProfileId,profiles[0])
      const pinnedRemoval = await invoke('studio_remove_model',{kind:'video',profileId:profiles[0],confirmed:true})
      assert.equal(pinnedRemoval.removed,false)
      assert.ok(pinnedRemoval.projects.some(project=>project.shotIds.includes(s1)))
      assert.equal(pinned.shot.videoProfileId,profiles[0])
      assert.equal(pinned.shotInputs.duration,6)
      const pinnedPlan = await invoke('studio_check_video_readiness', {
        projectId, shots:[{shotId:s1, mode:'reference', referenceAssetIds:['ref']}]
      })
      assert.equal(pinnedPlan.items[0].profileId,profiles[0],JSON.stringify(pinnedPlan))
      assert.equal((await invoke('studio_list_models',{kind:'video'})).activeId,profiles[1])
      revision = (await invoke('studio_update_shot_inputs', {
        projectId, shotId:s1, expectedUpdatedAt:pinned.updatedAt, videoProfileId:null, duration:5
      })).updatedAt
      const cleared = await invoke('studio_project_settings',{projectId,shotId:s1})
      assert.equal(cleared.shotInputs.videoProfileId,null)
      const mixed = template(true)
      mixed['5'].inputs.task_type = 'I2VA'
      mixed['5'].inputs['ref_images.ref_image_0'] = ['4', 0]
      delete mixed['5'].inputs.first_frame
      mixed['6'] = { class_type:'LoadImage', inputs:{image:'template-costume.png'} }
      mixed['5'].inputs['ref_images.ref_image_1'] = ['6', 0]
      mixed['7'] = {class_type:'KSampler',inputs:{steps:20,seed:1}}
      mixed['8'] = {class_type:'UNETLoader',inputs:{unet_name:'original.safetensors'}}
      const mixedProfile = (await invoke('studio_add_model', { kind:'video', name:'Mixed I2VA', baseUrl:localUrl, model:'test' })).profileId
      await invoke('studio_model_update', { kind:'video', profileId:mixedProfile, adapterType:'comfyui-prompt',
        workflowTemplate:JSON.stringify(mixed), inputMap:{bindings:[{role:'prompt',nodeId:'1',field:'prompt'}]} })
      const mixedDiagnosis = await invoke('studio_describe_model',{kind:'video',profileId:mixedProfile})
      assert.deepEqual(mixedDiagnosis.workflow.referenceSlots.map(({nodeId,index,pictureNumber})=>[nodeId,index,pictureNumber]),[['6',0,2]])
      assert.deepEqual(mixedDiagnosis.workflow.referenceSlots[0].consumers,[{nodeId:'5',field:'ref_images.ref_image_1'}])
      const mixedTake = { shotId:s2, profileId:mixedProfile, mode:'first-frame', firstFrameAssetId:'ref' }
      const checkMix = (take) => invoke('studio_check_video_readiness', { projectId, shots:[take] })
      assert.match((await checkMix(mixedTake)).items[0].issue, /template-costume.png/)
      const board = await invoke('studio_import_image_asset',{projectId,requestId:randomUUID(),dataBase64:(await fs.readFile(png)).toString('base64')})
      const markedBoard = await invoke('studio_bind_shot_asset',{projectId,shotId:s1,assetId:board.assetId,slot:'board',
        expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt})
      const boardCheck = (await checkMix({...mixedTake,firstFrameAssetId:board.assetId,referenceAssetIds:['ref']})).items[0]
      assert.equal(boardCheck.ready,false)
      assert.equal(boardCheck.issues[0].code,'board_in_frame_slot')
      const boardFrame=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
        name:'studio_attach_image_asset',input:{projectId,expectedUpdatedAt:markedBoard.updatedAt,targetType:'shot-first',
          targetId:s2,assetId:board.assetId}})})
      assert.equal(boardFrame.status,400)
      assert.equal((await boardFrame.json()).issues[0].code,'board_in_frame_slot')
      const invalidInput=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
        name:'studio_update_shot_inputs',input:{projectId,shotId:s2,expectedUpdatedAt:'auto',lastFrameAssetId:board.assetId}})})
      assert.equal(invalidInput.status,400)
      assert.equal((await invalidInput.json()).issues[0].code,'board_in_frame_slot')
      const invalidBind=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
        name:'studio_bind_shot_asset',input:{projectId,shotId:s2,expectedUpdatedAt:'auto',slot:'first',assetId:board.assetId}})})
      assert.equal(invalidBind.status,400)
      assert.equal((await invalidBind.json()).issues[0].code,'board_in_frame_slot')
      const snapshot=await invoke('studio_project_settings',{projectId})
      const boardProjectPath=path.join(snapshot.folderPath,'project.json')
      const boardProject=JSON.parse(await fs.readFile(boardProjectPath,'utf8'))
      boardProject.shots.find(shot=>shot.id===s2).firstFrameAssetId=board.assetId
      const browserBoard=await fetch(origin+'/api/dramlo/fs',{method:'POST',headers,body:JSON.stringify({
        command:'writeText',path:boardProjectPath,expectedUpdatedAt:markedBoard.updatedAt,value:JSON.stringify(boardProject)})})
      assert.equal(browserBoard.status,400)
      assert.equal((await browserBoard.json()).issues[0].code,'board_in_frame_slot')
      const cropKey={projectId,shotId:'#01',expectedUpdatedAt:'auto',requestId:randomUUID(),panel:3}
      const cropped=await invoke('studio_crop_shot_frame',cropKey)
      const cropPath=(await invoke('studio_locate_asset',{projectId,assetId:cropped.assetId})).path
      const cropBytes=await fs.readFile(cropPath)
      assert.deepEqual([cropBytes.readUInt32BE(16),cropBytes.readUInt32BE(20)],[32,32])
      assert.equal((await invoke('studio_crop_shot_frame',cropKey)).reused,true)
      assert.equal((await checkMix({...mixedTake,firstFrameAssetId:cropped.assetId,referenceAssetIds:['ref']})).items[0].ready,true)
      for (const slot of ['first','last']) {
        const frame = await invoke('studio_bind_shot_asset',{
          projectId,shotId:s3,expectedUpdatedAt:'auto',slot,assetId:cropped.assetId
        })
        assert.equal(frame.bind.current,cropped.assetId)
      }
      const frameShot = (await invoke('studio_shots',{projectId,shotIds:[s3],fields:['firstFrameAssetId','lastFrameAssetId']})).items[0]
      assert.equal(frameShot.firstFrameAssetId,cropped.assetId)
      assert.equal(frameShot.lastFrameAssetId,cropped.assetId)
      revision=(await invoke('studio_update_shot_inputs',{
        projectId,shotId:s3,expectedUpdatedAt:'auto',firstFrameAssetId:null,lastFrameAssetId:null
      })).updatedAt
      const allowFixed = (await checkMix({ ...mixedTake, allowTemplateImages:true })).items[0]
      assert.equal(allowFixed.ready,true,JSON.stringify(allowFixed))
      assert.equal(allowFixed.templateImages[0].filename,'template-costume.png')
      const mixedReady = (await checkMix({ ...mixedTake, referenceAssetIds:['ref'] })).items[0]
      assert.equal(mixedReady.ready,true,JSON.stringify(mixedReady))
      assert.deepEqual(mixedReady.templateImages,[])
      assert.match((await checkMix({ ...mixedTake, mode:'first-last-frame' })).items[0].issue,/没有尾帧槽/)
      const declaredGraph=template(false)
      declaredGraph['6']={class_type:'LoadImage',inputs:{image:'old-board.png'}}
      declaredGraph['7']={class_type:'LoadImage',inputs:{image:'old-scene.png'}}
      declaredGraph['8']={class_type:'ResolutionSelector',inputs:{aspect_ratio:'16:9 (Widescreen)',megapixels:2}}
      declaredGraph['5'].inputs['ref_images.ref_image_1']=['6',0]
      declaredGraph['5'].inputs['ref_images.ref_image_2']=['7',0]
      const referenceConfig={mode:'ordered-optional',neutralAssetId:'ref',boardInFrameSlot:'block',slots:[
        {index:1,nodeId:'3',role:'identity',required:false,fallback:'neutral'},
        {index:2,nodeId:'6',role:'storyboard',required:false,fallback:'neutral'},
        {index:3,nodeId:'7',role:'scene',required:true}]}
      const outputConfig={aspectRatios:['16:9','9:16'],defaultAspectRatio:'16:9',
        resolutionByRatio:{'16:9':'1536x864','9:16':'864x1536'},durationNode:'2',durationRange:[1,15]}
      const declaredProfile=(await invoke('studio_add_model',{kind:'video',name:'Explicit three slots',select:false,baseUrl:localUrl,model:'test',
        adapterType:'comfyui-prompt',workflowTemplate:JSON.stringify(declaredGraph),referenceConfig,outputConfig})).profileId
      const declared=await invoke('studio_describe_model',{kind:'video',profileId:declaredProfile})
      assert.equal(declared.minReferences,1)
      assert.equal(declared.maxReferences,3)
      assert.deepEqual(declared.referenceConfig,referenceConfig)
      assert.deepEqual(declared.outputConfig,outputConfig)
      const slotTake={shotId:s3,mode:'reference',profileId:declaredProfile,aspectRatio:'9:16'}
      const missingScene=(await invoke('studio_check_video_readiness',{projectId,shots:[slotTake]})).items[0]
      assert.equal(missingScene.issues[0].role,'scene')
      const neutralKey={projectId,requestId:randomUUID()}
      const beforeNeutral={submits,uploads:uploads.length}
      const neutralPlan=(await invoke('studio_prepare_video',{...neutralKey,expectedUpdatedAt:'auto',
        shots:[{...slotTake,referenceAssignments:[{index:3,assetId:cropped.assetId}]}]})).batch
      assert.equal(neutralPlan.status,'prepared')
      assert.deepEqual(neutralPlan.steps[0].referenceAssignments.map(item=>[item.role,item.fallback]),
        [['identity',true],['storyboard',true],['scene',false]])
      assert.equal(neutralPlan.steps[0].output.appliedBindings[0].value,'9:16 (Portrait Widescreen)')
      assert.equal(neutralPlan.steps[0].output.size.height,1536)
      assert.equal(neutralPlan.steps[0].output.size.mode,'selector-target')
      assert.equal(neutralPlan.workload.modelRequests,1)
      assert.equal(neutralPlan.workload.imageInputs,3)
      assert.equal(neutralPlan.workload.videoSeconds,5)
      assert.equal(neutralPlan.workload.money,null)
      assert.deepEqual({submits,uploads:uploads.length},beforeNeutral)
      const fallbackPlan='explicit-fallback-workload'
      await invoke('studio_save_plan',{projectId,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,
        planId:fallbackPlan,name:'参考图补位预检',steps:[{id:'video',kind:'video',input:{...slotTake,referenceAssignments:[{index:3,assetId:cropped.assetId}]}}]})
      const fallbackGraph=await invoke('studio_canvas_graph',{projectId,planId:fallbackPlan})
      assert.equal(fallbackGraph.execution[0].state,'ready')
      assert.deepEqual(fallbackGraph.execution[0].requires.map(slot=>[slot.workflowInput.role,slot.state,slot.required]),
        [['identity','fallback',false],['storyboard','fallback',false],['scene','bound',true]])
      assert.equal(fallbackGraph.workload.modelRequests,1)
      assert.equal(fallbackGraph.workload.imageInputs,3)
      const blockedPlan='declared-board-edge'
      await invoke('studio_save_plan',{projectId,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,
        planId:blockedPlan,name:'分格接入首帧反例',steps:[
          {id:'board',kind:'image',input:{targetType:'shot-board',targetId:s2,visualPrompt:'四格看板'}},
          {id:'video',kind:'video',input:{...mixedTake,firstFrameAssetId:undefined},sources:[{stepId:'board',slot:'firstFrame'}]}
        ]})
      const doomed=(await invoke('studio_canvas_graph',{projectId,planId:blockedPlan})).execution[1]
      assert.equal(doomed.state,'blocked')
      assert.equal(doomed.reason.issues[0].code,'board_in_frame_slot')
      await invoke('studio_discard_video_plan',neutralKey)
      await invoke('studio_model_update',{kind:'video',profileId:declaredProfile,outputConfig:{durationRange:[1,4]}})
      const afterPatch=await invoke('studio_describe_model',{kind:'video',profileId:declaredProfile})
      assert.deepEqual(afterPatch.referenceConfig,referenceConfig)
      assert.deepEqual(afterPatch.outputConfig.resolutionByRatio,outputConfig.resolutionByRatio)
      const tooLong=(await invoke('studio_check_video_readiness',{projectId,
        shots:[{...slotTake,referenceAssignments:[{index:3,assetId:cropped.assetId}]}]})).items[0]
      assert.match(tooLong.issue,/时长范围/)
      const staleBinding=await invoke('studio_model_update',{kind:'video',profileId:declaredProfile,inputMap:{bindings:[
        {role:'prompt',nodeId:'old-id',field:'prompt'}]}})
      assert.equal(staleBinding.brokenBindings[0].reason,'missing_node')
      const brokenPatch=await invoke('studio_patch_workflow',{kind:'video',profileId:declaredProfile,
        expectedWorkflowRevision:(await invoke('studio_describe_model',{kind:'video',profileId:declaredProfile})).workflowRevision,
        patches:[{nodeId:'1',field:'prompt',value:'fixture'}]})
      assert.equal(brokenPatch.brokenBindings[0].nodeId,'old-id')
      const readyToShoot = await invoke('studio_check_video_readiness', { projectId, shots: [
        { shotId: s1, profileId: profiles[0], mode: 'reference', referenceAssetIds: ['ref'] },
        { shotId: s2, profileId: profiles[1], mode: 'previous-tail' }
      ] })
      assert.equal(readyToShoot.ready, true, JSON.stringify(readyToShoot))
      assert.equal(readyToShoot.items[0].inputMode, 'reference')
      assert.equal(readyToShoot.items[0].missingPropImages[0].id, propId)
      assert.equal(readyToShoot.items[1].frameSources.previousShotId, s1)

      assert.equal((await invoke('studio_current_video_batch', { projectId })).batch, null)
      const order = { projectId, requestId: randomUUID() }
      const phase = await invoke('studio_prepare_video', { ...order, expectedUpdatedAt: revision, shots: [
        { shotId: s3, profileId: profiles[0], mode: 'reference', referenceAssetIds: ['ref'] },
        { shotId: s1, profileId: profiles[0], mode: 'reference', referenceAssetIds: ['ref'] },
        { shotId: s2, profileId: profiles[1], mode: 'previous-tail' }
      ] })
      assert.deepEqual(phase.batch.steps.map((step) => step.shotId), [s1, s2, s3])
      await invoke('studio_confirm_video', { ...order, confirmed: true })
      for (let attempt = 0; attempt < 100; attempt++) {
        status = (await invoke('studio_video_status', order)).batch
        if (status.status !== 'running') break
        await new Promise((resolve) => setTimeout(resolve, 100))
      }
      assert.equal(status.status, 'succeeded', JSON.stringify(status))
      assert.equal(submits, 4)
      assert.ok(status.steps.every(step=>step.bind.applied && step.bind.isCurrent && step.bind.reason==='empty_slot'))
      assert.equal(uploads.length, 3)
      assert.ok(uploads[1].includes(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a])))
      assert.ok(graphs[2]['5'].inputs.first_frame)
      assert.equal(graphs[3]['5'].inputs.first_frame, undefined)
      const linked = await invoke('studio_project_items', { projectId, type: 'assets' })
      assert.equal(linked.items.find((asset) => asset.shotId === s2).videoGeneration.previousAssetId, status.steps[0].assetId)
      const custom = { formatVersion: 1, id: 'fixture.video-tail', version: 2, name: 'Fixture tail',
        description: 'Extract fixture video tail', executor: 'studio.video-tail' }
      assert.equal((await invoke('studio_extension_preview', { manifest: custom })).id, custom.id)
      const listLibrary = () => invoke('studio_extension_library', { projectId })
      assert.ok(!(await listLibrary()).items.some(item => item.id === custom.id))
      await invoke('studio_extension_install', { manifest: custom })
      assert.equal((await listLibrary()).items.find(item => item.id === custom.id).enabled, false)
      const setEnabled = enabled => invoke('studio_extension_enable', { projectId, typeId: custom.id, version: 2, enabled })
      await setEnabled(true)
      const customNodes = (await invoke('studio_list_node_types', { projectId })).items
      assert.equal(customNodes.find(item => item.id === custom.id).version, 2)
      assert.ok(!(await invoke('studio_list_node_types', { projectId: 'fixture' })).items.some(item => item.id === custom.id))
      const nodeCatalog = await invoke('studio_list_node_types', {})
      assert.equal(nodeCatalog.items[0].source, 'built-in')
      const canvasNodes = (await invoke('studio_project_items', { projectId, type: 'nodes' })).items
      const source = canvasNodes.find(node => node.type === 'shotVideo' && node.data.shotId === s1)
      assert.ok(source)
      const current = await invoke('studio_open_project', { projectId })
      const projectKey = { projectId }
      const extensionId = randomUUID()
      const added = await invoke('studio_add_extension_node', { ...projectKey, typeId: nodeCatalog.items[0].id,
        nodeId: extensionId, expectedUpdatedAt: current.updatedAt })
      const edge = await invoke('studio_connect_extension_node', { ...projectKey, sourceId: source.id,
        targetId: extensionId, expectedUpdatedAt: added.updatedAt })
      assert.equal(edge.edge.targetHandle, 'video')
      const staleFrame = await fetch(origin + '/api/dramlo/canvas-extensions', { method: 'POST', headers,
        body: JSON.stringify({ name: 'studio_run_extension_node', input: {
          ...projectKey, nodeId: extensionId, expectedUpdatedAt: added.updatedAt
        } }) })
      assert.equal(staleFrame.status, 409)
      const frame = await fetch(origin + '/api/dramlo/canvas-extensions', { method: 'POST', headers,
        body: JSON.stringify({ name: 'studio_run_extension_node', input: {
          ...projectKey, nodeId: extensionId, expectedUpdatedAt: edge.updatedAt
        } }) })
      assert.ok(frame.ok, await frame.clone().text())
      const output = await frame.json()
      const frameAsset = (await invoke('studio_project_items', { projectId, type: 'assets' })).items.find(asset => asset.id === output.assetId)
      assert.equal(frameAsset.kind, 'image')
      assert.equal(frameAsset.derivedFromAssetId, status.steps[0].assetId)
      const pngBytes = await fs.readFile(path.join(active.folderPath, frameAsset.fileRelPath))
      assert.deepEqual(pngBytes.subarray(0, 8), Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]))
      const graphFile = path.join(active.folderPath, 'project.json')
      const graphProject = JSON.parse(await ok('readText', { path: graphFile }))
      const downstreamId = randomUUID()
      graphProject.canvas.nodes.push({ id: downstreamId, type: 'videoGen', position: { x: 100, y: 200 },
        data: { h3GenMode: 'fl2va' } })
      graphProject.updatedAt++
      await ok('writeText', { path: graphFile, value: JSON.stringify(graphProject), expectedUpdatedAt: output.updatedAt })
      const wired = await invoke('studio_connect_extension_node', { ...projectKey, sourceId: extensionId,
        targetId: downstreamId, expectedUpdatedAt: graphProject.updatedAt })
      const savedGraph = JSON.parse(await ok('readText', { path: graphFile }))
      assert.equal(wired.edge.targetHandle, 'first-frame')
      assert.ok(savedGraph.canvas.edges.some(item => item.id === wired.edge.id))
      assert.equal(savedGraph.canvas.nodes.find(item => item.id === extensionId).data.outputAssetId, output.assetId)
      const customNode = await invoke('studio_add_extension_node', { ...projectKey, typeId: custom.id, version: 2,
        nodeId: randomUUID(), videoAssetId: status.steps[0].assetId, expectedUpdatedAt: wired.updatedAt })
      await setEnabled(false)
      const disabledFrame = await fetch(origin + '/api/dramlo/canvas-extensions', { method: 'POST', headers,
        body: JSON.stringify({ name: 'studio_run_extension_node', input: {
          ...projectKey, nodeId: customNode.node.id, expectedUpdatedAt: customNode.updatedAt
        } }) })
      assert.equal(disabledFrame.status, 404)
      assert.ok((await invoke('studio_project_items', { projectId, type: 'nodes' })).items.some(item => item.id === customNode.node.id))
      console.log('PASS extension library preview, install, per-project activation, version pinning and safe disable')


      assert.equal(submits, 4)
      console.log('PASS extension catalog, MCP canvas node creation and connection, revision guard, browser FFmpeg run and asset registration')
      const workflow = { ...custom, id: 'fixture.comfy-video', executor: 'studio.comfy-video', profileId: mixedProfile,
        inputs:[{id:'costume',kind:'image'}],outputs:[{id:'render',kind:'video'}],
        params:[
          {key:'steps',label:'采样步数',type:'integer',min:1,max:100,default:24,target:{nodeId:'7',field:'steps'}},
          {key:'seed',label:'随机种子',type:'integer',min:0,target:{nodeId:'7',field:'seed'}},
          {key:'weight',label:'模型权重',type:'model',target:{nodeId:'8',field:'unet_name'}}
        ] }
      await invoke('studio_extension_install', { manifest: workflow })
      const extensionRemoval = await invoke('studio_remove_model',{kind:'video',profileId:mixedProfile,confirmed:true})
      assert.equal(extensionRemoval.removed,false)
      assert.ok(extensionRemoval.extensions.some(entry=>entry.typeId===workflow.id))
      await invoke('studio_extension_enable', { projectId, typeId: workflow.id, version: 2, enabled: true })
      const comfyNode = await invoke('studio_add_extension_node', { projectId, typeId: workflow.id, version: 2,
        nodeId: randomUUID(), expectedUpdatedAt: customNode.updatedAt, params:{seed:40} })
      const sibling = await invoke('studio_add_extension_node', {projectId,typeId:workflow.id,version:2,
        nodeId:randomUUID(),expectedUpdatedAt:comfyNode.updatedAt})
      const nodeKey = {projectId,nodeId:comfyNode.node.id}
      const beforeParams = await invoke('studio_describe_extension_node',{...nodeKey,includeChoices:true})
      assert.deepEqual(beforeParams.params,{steps:24,seed:40})
      assert.deepEqual(beforeParams.choices.weight.items,['original.safetensors','alternate.safetensors'])
      const rejected = await fetch(origin + '/api/dramlo/canvas-extensions', {method:'POST',headers,
        body:JSON.stringify({name:'studio_update_extension_node',input:{...nodeKey,expectedUpdatedAt:sibling.updatedAt,
          params:{weight:'../alternate.safetensors'}}})})
      assert.equal(rejected.status,400)
      const linkedCostume = await invoke('studio_connect_extension_node',{projectId,expectedUpdatedAt:sibling.updatedAt,
        sourceId:extensionId,sourceHandle:'image',targetId:comfyNode.node.id,targetHandle:'costume'})
      assert.equal(linkedCostume.edge.targetHandle,'costume')
      const shot = { shotId:s2,mode:'first-frame',firstFrameAssetId:'ref' }
      const dryNodeId=randomUUID()
      const dryNode=await invoke('studio_prepare_comfy_node',{projectId,nodeId:comfyNode.node.id,requestId:dryNodeId,dryRun:true,
        expectedUpdatedAt:'auto',shot})
      assert.equal(dryNode.dryRun,true)
      assert.equal(dryNode.ready,true)
      const dryStatus=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
        body:JSON.stringify({name:'studio_video_status',input:{projectId,requestId:dryNodeId}})})
      assert.equal(dryStatus.ok,false)
      const staleJob = {projectId,requestId:randomUUID()}
      await invoke('studio_prepare_comfy_node',{...staleJob,nodeId:comfyNode.node.id,expectedUpdatedAt:linkedCostume.updatedAt,shot})
      const updatedParams = await invoke('studio_update_extension_node',{...nodeKey,expectedUpdatedAt:linkedCostume.updatedAt,
        params:{steps:33,seed:91,weight:'alternate.safetensors'}})
      const stale = await fetch(origin + '/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
        body:JSON.stringify({name:'studio_confirm_video',input:{...staleJob,confirmed:true}})})
      assert.equal(stale.status,409)
      assert.deepEqual((await invoke('studio_describe_extension_node',{projectId,nodeId:sibling.node.id})).params,{steps:24})
      const workflowJob = { projectId, requestId: randomUUID() }
      const workflowPlan = await invoke('studio_prepare_comfy_node', { ...workflowJob, nodeId: comfyNode.node.id,
        expectedUpdatedAt: updatedParams.updatedAt, shot })
      assert.equal(workflowPlan.batch.status, 'prepared')
      assert.deepEqual(workflowPlan.batch.steps[0].extensionState.referenceAssetIds,[output.assetId])
      assert.equal(submits, 4)
      await invoke('studio_confirm_video', { ...workflowJob, confirmed: true })
      let workflowStatus
      for (let attempt = 0; attempt < 100; attempt++) {
        workflowStatus = (await invoke('studio_video_status', workflowJob)).batch
        if (workflowStatus.status !== 'running') break
        await new Promise(resolve => setTimeout(resolve, 100))
      }
      assert.equal(workflowStatus.status, 'succeeded', JSON.stringify(workflowStatus))
      const finishedNode = (await invoke('studio_project_items', { projectId, type: 'nodes' })).items.find(item => item.id === comfyNode.node.id)
      assert.equal(finishedNode.data.outputAssetId, workflowStatus.steps[0].assetId)
      assert.equal(submits, 5)
      assert.equal(graphs[4]['5'].inputs.task_type, 'I2VA')
      assert.notEqual(graphs[4]['6'].inputs.image, 'template-costume.png')
      assert.notEqual(graphs[4]['4'].inputs.image, graphs[4]['6'].inputs.image)
      assert.deepEqual(graphs[4]['7'].inputs,{steps:33,seed:91})
      assert.equal(graphs[4]['8'].inputs.unet_name,'alternate.safetensors')
      const afterWorkflow = await invoke('studio_describe_extension_node',nodeKey)
      const reset = await invoke('studio_update_extension_node',{...nodeKey,expectedUpdatedAt:afterWorkflow.updatedAt,params:{steps:null}})
      assert.deepEqual(reset.params,{steps:24,seed:91,weight:'alternate.safetensors'})
      ready = false
      const delayedJob = {projectId, requestId:randomUUID()}
      await invoke('studio_prepare_comfy_node',{...delayedJob,nodeId:comfyNode.node.id,expectedUpdatedAt:reset.updatedAt,shot})
      await invoke('studio_confirm_video',{...delayedJob,confirmed:true})
      let delayed
      for (let i = 0; i < 80; i++) {
        delayed = (await invoke('studio_video_status',delayedJob)).batch
        if (delayed.steps[0].status === 'waiting') break
        await new Promise(resolve => setTimeout(resolve,50))
      }
      assert.equal(delayed.steps[0].status,'waiting')
      await invoke('studio_update_extension_node',{...nodeKey,expectedUpdatedAt:reset.updatedAt,params:{seed:100}})
      ready = true
      for (let i = 0; i < 100; i++) {
        delayed = (await invoke('studio_video_status',delayedJob)).batch
        if (delayed.status !== 'running') break
        await new Promise(resolve => setTimeout(resolve,100))
      }
      assert.equal(delayed.status,'succeeded',JSON.stringify(delayed))
      const retainedNode = (await invoke('studio_project_items',{projectId,type:'nodes'})).items.find(item => item.id === comfyNode.node.id)
      assert.equal(retainedNode.data.outputAssetId,finishedNode.data.outputAssetId)
      assert.ok((await invoke('studio_project_items',{projectId,type:'assets'})).items.some(item=>item.id===delayed.steps[0].assetId))
      if (process.env.DRAMLO_TEST_BROWSER) {
        await invoke('studio_open_project',{projectId})
        const { stdout } = await promisify(execFile)(process.env.DRAMLO_TEST_BROWSER, [
          '--headless', '--no-first-run', '--disable-background-networking', '--disable-gpu',
          '--user-data-dir=' + path.join(scratch,'extension-parameters-browser'), '--virtual-time-budget=6500',
          '--dump-dom', origin + '/#/studio'
        ], {windowsHide:true,timeout:30000,maxBuffer:4*1024*1024})
        assert.match(stdout,/采样步数/)
        assert.match(stdout,/模型权重/)
        assert.match(stdout,/data-handleid="costume"/)
        console.log('PASS browser rendering of extension parameter controls and custom port')
      }

      console.log('PASS imported ComfyUI extension prepares without generation, confirms, and archives video to its canvas node')
      const pngOutput = await fs.readFile(png)
      const imageGraphs = []
      const refUploads = []
      let rejectPlanPrompt = false
      let holdPlanHistory = false
      let planHistorySeen = false
      const imageMock = createServer(async (req, res) => {
        if (req.url?.startsWith('/view?')) return res.writeHead(200, { 'Content-Type': 'image/png' }).end(pngOutput)
        if (req.url?.startsWith('/history/')) {
          const id = decodeURIComponent(req.url.slice('/history/'.length))
          if (holdPlanHistory) {
            planHistorySeen = true
            return res.end('{}')
          }
          return res.end(JSON.stringify({ [id]: { status: { completed: true }, outputs: {
            '3': { images: [{ filename: 'frame.png', type: 'output' }] }
          } } }))
        }
        const parts = []
        for await (const part of req) parts.push(part)
        if (req.url === '/upload/image') {
          refUploads.push(Buffer.concat(parts))
          return res.end(JSON.stringify({ name: 'scene-ref.png' }))
        }
        if (req.url === '/prompt') {
          const graph = JSON.parse(Buffer.concat(parts)).prompt
          imageGraphs.push(graph)
          if (rejectPlanPrompt && graph['23:24']?.inputs.value === 'plan retry') return res.writeHead(400).end()
          return res.end(JSON.stringify({ prompt_id: 'img-' + imageGraphs.length }))
        }
        res.writeHead(404).end()
      })
      imageMock.listen(0, '127.0.0.1')
      await once(imageMock, 'listening')
      try {
        const address = 'http://127.0.0.1:' + imageMock.address().port
        async function imageProfile(usesRef) {
          const { profileId } = await invoke('studio_add_model', { kind:'image', name: usesRef ? 'fixture with reference' : 'fixture text', baseUrl:address, model:'comfyui' })
          const graph = { '1': usesRef ? { class_type:'CLIPTextEncode', inputs:{text:'replace me'} }
            : { class_type:'MiniMaxH3Image', inputs:{prompt:'replace me'} },
            '3': { class_type:'SaveImage', inputs:{filename_prefix:'fixture'} } }
          if (usesRef) {
            graph['2'] = { class_type:'LoadImage', inputs:{image:'replace.png'} }
            graph['3'].inputs.images = ['2',0]
          }
          else {
            graph['1'] = { class_type:'StringReplace', inputs:{find:'{input text}',replace:['23:24',0]} }
            graph['23:24'] = { class_type:'PrimitiveStringMultiline', inputs:{value:'sample prompt'} }
            graph['2'] = { class_type:'ResolutionSelector', inputs:{megapixels:2,aspect_ratio:'1:1 (Square)'} }
          }
          await invoke('studio_model_update', { kind:'image', profileId, adapterType:'comfyui-prompt',
            workflowTemplate:JSON.stringify(usesRef ? {prompt:graph} : graph), inputMap:{ bindings:[
              ...(usesRef ? [{ role:'prompt', nodeId:'1', field:'text' }]
                : [{ role:'megapixels', nodeId:'2', field:'megapixels' }])
            ] } })
          return profileId
        }
        async function makeImage(parameters) {
          const current = await invoke('studio_open_project', { projectId })
          const key = { projectId, requestId: randomUUID() }
          const before = imageGraphs.length
          const plan = await invoke('studio_prepare_image', { ...key, expectedUpdatedAt:current.updatedAt, ...parameters })
          assert.equal(plan.job.status, 'prepared')
          assert.ok(!plan.job.warnings.some(message=>message.includes('画面内文字')))
          if(parameters.promptLayout) assert.deepEqual(plan.job.promptLayout,parameters.promptLayout)
          assert.equal(imageGraphs.length, before)
          await invoke('studio_confirm_image', { ...key, confirmed:true })
          await invoke('studio_confirm_image', { ...key, confirmed:true })
          for (let attempt = 0; attempt < 100; attempt++) {
            const { job } = await invoke('studio_image_status', key)
            if (job.status === 'succeeded') {
              assert.equal(job.selected, true)
              assert.equal(imageGraphs.length, before + 1)
              return job.assetId
            }
            assert.ok(['starting','submitting','waiting'].includes(job.status), JSON.stringify(job))
            await new Promise(resolve => setTimeout(resolve, 100))
          }
          throw new Error('image job did not finish')
        }
        const textImageProfile = await imageProfile(false)
        const discardedImage = {projectId,requestId:randomUUID()}
        const imageRevision = (await invoke('studio_open_project',{projectId})).updatedAt
        const taggedPrompt = '【16:9】「测试地点」场景锚点图。\n四格按这个清单，不够四项就补齐。'
        const tagged = await invoke('studio_prepare_image',{...discardedImage,expectedUpdatedAt:imageRevision,
          targetType:'scene',targetId:firstScene,profileId:textImageProfile,prompt:taggedPrompt})
        assert.equal(tagged.job.prompt,taggedPrompt)
        assert.equal(tagged.job.aspect.matchesProject,false)
        assert.equal(tagged.job.aspect.projectAspectRatio,'16:9')
        assert.deepEqual(tagged.job.aspect.workflowSelectors,[{nodeId:'2',value:'1:1 (Square)',ratio:'1:1'}])
        assert.ok(tagged.job.warnings.some(message=>message.includes('画幅标签') && message.includes('补齐指令')))
        assert.equal((await invoke('studio_discard_image_plan',discardedImage)).job.status,'discarded')
        assert.equal((await invoke('studio_discard_image_plan',discardedImage)).reused,true)
        assert.equal((await invoke('studio_current_image_job',{projectId,targetType:'scene',targetId:firstScene})).job,null)
        assert.equal(imageGraphs.length,0)
        const aspectProfile = await imageProfile(false)
        const aspectConfig = await invoke('studio_describe_model',{kind:'image',profileId:aspectProfile})
        const patch = {kind:'image',profileId:aspectProfile,expectedWorkflowRevision:aspectConfig.workflowRevision,
          patches:[{nodeId:'2',field:'megapixels',value:4},{nodeId:'2',field:'aspect_ratio',value:'4:3 (Standard)'}]}
        const patched = await invoke('studio_patch_workflow',patch)
        assert.notEqual(patched.workflowRevision,aspectConfig.workflowRevision)
        assert.equal(patched.patched[0].mappedRole,'megapixels')
        assert.deepEqual((await invoke('studio_describe_model',{kind:'image',profileId:aspectProfile})).inputMap,aspectConfig.inputMap)
        const stalePatch = await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
          body:JSON.stringify({name:'studio_patch_workflow',input:patch})})
        assert.equal(stalePatch.status,409)
        async function inspectAspect(ratio) {
          const key = {projectId,requestId:randomUUID()}
          const {job} = await invoke('studio_prepare_image',{...key,expectedUpdatedAt:(await invoke('studio_open_project',{projectId})).updatedAt,
            profileId:aspectProfile,targetId:firstScene,targetType:'scene',visualPrompt:'Aspect probe'})
          assert.equal(job.aspect.workflowSelectors[0].value,ratio)
          await invoke('studio_discard_image_plan',key)
        }
        await inspectAspect('4:3 (Standard)')
        await invoke('studio_model_update',{kind:'image',profileId:aspectProfile,inputMap:{bindings:[
          {role:'aspect_ratio',nodeId:'2',field:'aspect_ratio'},{role:'megapixels',nodeId:'2',field:'megapixels'}]}})
        await inspectAspect('16:9 (Widescreen)')
        async function checkSavedPrompt(targetType,targetId,promptSource,body,extra={}) {
          const key = {projectId,requestId:randomUUID()}
          const {job} = await invoke('studio_prepare_image',{...key,targetType,targetId,profileId:textImageProfile,
            expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,...extra})
          assert.equal(job.prompt,body)
          assert.equal(job.promptSource,promptSource)
          await invoke('studio_discard_image_plan',key)
          return job
        }
        await checkSavedPrompt('shot-image',s2,'visualPrompt','蓝色测试镜头')
        const layout = {title:'仅用于项目定位',aspectRatio:'16:9',rows:2,columns:2}
        const stored = await invoke('studio_set_asset_prompt',{projectId,
          expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,
          targetType:'scene',targetId:firstScene,visualPrompt:'scene anchor',promptLayout:layout})
        assert.deepEqual(stored.promptLayout,layout)
        let promptVersion = stored.updatedAt
        for(const [targetType,targetId,body,promptField] of [
          ['shot-image',s2,'saved shot prompt','imagePrompt'],['prop',propId,'saved prop prompt','sheetPrompt']
        ]){
          const saved = await invoke('studio_set_asset_prompt',{projectId,expectedUpdatedAt:promptVersion,targetType,targetId,visualPrompt:body})
          assert.equal(saved.promptField,promptField)
          promptVersion = saved.updatedAt
          await checkSavedPrompt(targetType,targetId,promptField,body)
        }
        assert.deepEqual((await checkSavedPrompt('scene',firstScene,'platePrompt','scene anchor')).promptLayout,layout)
        await checkSavedPrompt('shot-image',s2,'request','one-off override',{visualPrompt:'one-off override'})
        await checkSavedPrompt('shot-image',s2,'imagePrompt','saved shot prompt')
        const sceneImage = await makeImage({ targetType:'scene', targetId:firstScene, profileId:textImageProfile })
        const sceneMeta=(await invoke('studio_project_items',{projectId,type:'assets',limit:50})).items.find(item=>item.id===sceneImage)
        assert.equal(sceneMeta.frameRole,'board')
        const stillBlocked=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
          name:'studio_attach_image_asset',input:{projectId,targetType:'shot-first',targetId:s2,assetId:sceneImage,
            expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt}})})
        assert.equal(stillBlocked.status,400)
        assert.equal((await stillBlocked.json()).issues[0].code,'board_in_frame_slot')
        const openaiBase = `http://127.0.0.1:${mock.address().port}/v1`
        const testKey = randomBytes(16).toString('hex')
        const {profileId:openaiProfile} = await invoke('studio_add_model',{kind:'image',name:'Images API fixture',
          baseUrl:openaiBase,adapterType:'openai-images',model:'gpt-image-2.6-sunburst',select:false})
        await invoke('studio_set_credential',{kind:'image',profileId:openaiProfile,expectedBaseUrl:openaiBase,apiKey:testKey})
        const openaiInputs = await invoke('studio_describe_image_inputs',{profileId:openaiProfile})
        assert.equal(openaiInputs.ready,true)
        assert.equal(openaiInputs.referenceMode,'ordered-optional')
        assert.equal(openaiInputs.maxReferences,10)
        assert.equal((await invoke('studio_describe_model',{kind:'image',profileId:openaiProfile})).ready,true)
        const refs = ['first','second','third'].map(mark=>Buffer.concat([pngFixture,Buffer.from(mark)]))
        const refIds = []
        for (const bytes of refs) {
          const result = await invoke('studio_import_image_asset',{projectId,requestId:randomUUID(),dataBase64:bytes.toString('base64')})
          refIds.push(result.assetId)
        }
        const actorId = randomUUID()
        const cast = await invoke('studio_upsert_project_assets',{projectId,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,
          items:[{id:actorId,type:'subject',kind:'person',name:'核对演员',promptName:'Review Actor',description:'仅用于参考图一致性检查'}]})
        await invoke('studio_attach_image_asset',{projectId,expectedUpdatedAt:cast.updatedAt,targetType:'subject',targetId:actorId,assetId:refIds[0]})
        const castRequest={projectId,profileId:openaiProfile,targetType:'shot-image',targetId:'#02',
          expectedUpdatedAt:'auto',visualPrompt:'核对演员站在门边',referenceAssetIds:[]}
        async function castPreflight(input) {
          const reply=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
            name:'studio_prepare_image',input:{...castRequest,requestId:randomUUID(),...input}})})
          return {status:reply.status,body:await reply.json()}
        }
        const beforeCastChecks=openaiRequests.length
        const unbound=await castPreflight({visualPrompt:'Review Actor waits by the door.'})
        assert.equal(unbound.status,400)
        assert.equal(unbound.body.issues[0].code,'subject_not_bound')
        await invoke('studio_update_shot_inputs',{projectId,shotId:'#02',expectedUpdatedAt:'auto',subjectBindingMode:'explicit',subjectIds:[actorId]})
        const wrongReference=await castPreflight({visualPrompt:'他站在门边',referenceAssetIds:[refIds[1]]})
        assert.equal(wrongReference.status,400)
        assert.equal(wrongReference.body.issues[0].expectedAssetId,refIds[0])
        const approved=await castPreflight({referenceAssetIds:[refIds[0]]})
        assert.equal(approved.status,200,JSON.stringify(approved.body))
        assert.equal(approved.body.job.targetId,s2)
        assert.deepEqual(approved.body.job.referenceFacts.subjects[0],{
          subjectId:actorId,name:'核对演员',promptName:'Review Actor',selectedAssetId:refIds[0],submittedAssetId:refIds[0]})
        assert.equal(approved.body.job.workload.modelRequests,1)
        assert.equal(approved.body.job.workload.imageInputs,1)
        assert.equal(approved.body.job.workload.moneyStatus,'not_estimated')
        assert.equal(await invoke('studio_discard_image_plan',{projectId,requestId:approved.body.job.requestId}).then(reply=>reply.job.status),'discarded')
        const override=await castPreflight({bindingReview:'画外声音提到该演员'})
        assert.equal(override.status,200,JSON.stringify(override.body))
        assert.ok(override.body.job.warnings.some(warning=>warning.includes('核对演员')))
        await invoke('studio_discard_image_plan',{projectId,requestId:override.body.job.requestId})
        await invoke('studio_update_shot_inputs',{projectId,shotId:s2,expectedUpdatedAt:'auto',subjectBindingMode:'explicit',subjectIds:[]})
        const referenceDiagnosis=await invoke('studio_describe_model',{kind:'image',profileId:openaiProfile})
        assert.equal(referenceDiagnosis.referenceMode,'ordered-optional')
        assert.equal(referenceDiagnosis.maxReferences,10)
        assert.equal(openaiRequests.length,beforeCastChecks)
        async function openaiImage(prompt,ids,status,afterPrepare,targetType=status==='unknown'?'shot-last':'shot-image') {
          const key = {projectId,requestId:randomUUID()}
          const before = openaiRequests.length
          const {job:plan} = await invoke('studio_prepare_image',{...key,profileId:openaiProfile,targetType,
            targetId:s2,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,visualPrompt:prompt,referenceAssetIds:ids})
          assert.equal(plan.status,'prepared')
          assert.deepEqual(plan.referenceAssetIds,ids)
          assert.equal(plan.aspect.size,'1536x864')
          assert.equal(openaiRequests.length,before)
          assert.ok(!JSON.stringify(plan).includes(testKey))
          if (afterPrepare) await afterPrepare(plan)
          await invoke('studio_confirm_image',{...key,confirmed:true})
          for (let attempt = 0; attempt < 100; attempt++) {
            const {job} = await invoke('studio_image_status',key)
            if (job.status !== 'starting' && job.status !== 'submitting') {
              assert.equal(job.status,status,JSON.stringify(job))
              assert.equal((await invoke('studio_confirm_image',{...key,confirmed:true})).reused,true)
              assert.equal(openaiRequests.length,before+1)
              return job
            }
            await new Promise(resolve=>setTimeout(resolve,50))
          }
          throw new Error('OpenAI Images fixture did not finish')
        }
        const generated = await openaiImage('Mock text image',[],'succeeded',async plan=>{
          const changed=await invoke('studio_update_script',{projectId,expectedUpdatedAt:plan.updatedAt,keywords:['changed after preflight']})
          assert.ok(changed.updatedAt>plan.updatedAt)
        })
        const edited = await openaiImage('Mock reference image',refIds,'succeeded')
        assert.equal(generated.selected,true)
        assert.equal(edited.selected,true)
        const fixtureAsset = (await invoke('studio_project_items',{projectId,type:'assets',limit:100})).items
          .find(item=>item.id===edited.assetId)
        assert.ok(fixtureAsset)
        assert.deepEqual(await fs.readFile(path.join(active.folderPath,fixtureAsset.fileRelPath)),pngFixture)
        assert.deepEqual(JSON.parse(openaiRequests[0].body),{model:'gpt-image-2.6-sunburst',
          prompt:'Mock text image',size:'1536x864',n:1})
        assert.equal(openaiRequests[0].route,'/v1/images/generations')
        const editRequest = openaiRequests[1]
        assert.equal(editRequest.route,'/v1/images/edits')
        assert.equal(editRequest.method,'POST')
        assert.equal(editRequest.headers.authorization,`Bearer ${testKey}`)
        assert.match(editRequest.headers['content-type'],/^multipart\/form-data; boundary=/)
        assert.equal(editRequest.body.toString('latin1').match(/name="image\[\]"/g)?.length,3)
        const offsets = refs.map(bytes=>editRequest.body.indexOf(bytes))
        assert.ok(offsets[0]>=0 && offsets[0]<offsets[1] && offsets[1]<offsets[2])
        for(const [prompt,kind,param] of [['rejected-moderation','moderation','prompt'],['rejected-size','invalid_request','size']]){
          const failed = await openaiImage(prompt,[],'failed')
          assert.equal(failed.providerError.category,kind)
          assert.equal(failed.providerError.param,param)
          assert.equal(failed.providerError.requestId,'req_test_fixture')
          assert.equal(failed.nextAction.code,'new_request_id')
          assert.ok(!JSON.stringify(failed).includes(testKey))
          const afterFailure = openaiRequests.length
          const replay = await invoke('studio_prepare_image',{projectId,requestId:failed.requestId,profileId:openaiProfile,
            expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,targetType:'shot-image',targetId:s2,
            visualPrompt:'Different prompt on old requestId'})
          assert.equal(replay.reused,true)
          assert.equal(replay.job.nextAction.code,'new_request_id')
          assert.deepEqual(replay.job.providerError,failed.providerError)
          assert.equal(openaiRequests.length,afterFailure)
        }
        await openaiImage('Corrected prompt using a new requestId',[],'succeeded')
        const changedKey={projectId,requestId:randomUUID()}
        const {job:beforeChanged}=await invoke('studio_prepare_image',{...changedKey,targetType:'shot-image',targetId:s2,
          profileId:openaiProfile,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,visualPrompt:'Mock changed aspect'})
        const changedAspect=await invoke('studio_update_project_defaults',{projectId,expectedUpdatedAt:beforeChanged.updatedAt,aspectRatio:'9:16'})
        const response=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
          name:'studio_confirm_image',input:{...changedKey,confirmed:true}})})
        assert.equal(response.status,409)
        const conflict=await response.json()
        assert.equal(conflict.currentUpdatedAt,changedAspect.updatedAt)
        assert.match(conflict.error,/画幅/)
        await invoke('studio_discard_image_plan',changedKey)
        await invoke('studio_update_project_defaults',{projectId,expectedUpdatedAt:changedAspect.updatedAt,aspectRatio:'16:9'})
        let heldKey
        const sent = new Promise(resolve=>{ onHeldOpenAIResponse = resolve })
        const lateResult = openaiImage('hold-for-approval-review',[],'succeeded',async plan=>{heldKey=plan})
        const release = await Promise.race([sent,new Promise((_,reject)=>setTimeout(()=>reject(Error('Mock did not reach submitted')),6000).unref())])
        const previousSelected = (await invoke('studio_shots',{projectId,shotId:s2,fields:['imageAssetId']})).items[0].imageAssetId
        await invoke('studio_update_project_defaults',{projectId,expectedUpdatedAt:heldKey.updatedAt,aspectRatio:'9:16'})
        release()
        const late = await lateResult
        assert.equal(late.selected,false)
        assert.ok(late.assetId)
        assert.equal((await invoke('studio_shots',{projectId,shotId:s2,fields:['imageAssetId']})).items[0].imageAssetId,previousSelected)
        await invoke('studio_update_project_defaults',{projectId,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,aspectRatio:'16:9'})
        const unknown = await openaiImage('uncertain-provider-result',[],'unknown')
        const old = {projectId,requestId:unknown.requestId}
        const review = confirmed => fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,body:JSON.stringify({
          name:'studio_resolve_unknown_image',input:{...old,confirmed,reason:'模拟供应商确认报错，无可恢复结果'}})})
        assert.equal((await review(false)).status,400)
        const resolution = await review(true)
        assert.equal(resolution.status,200)
        const {job:closed}=await resolution.json()
        assert.equal(closed.status,'discarded')
        assert.equal(closed.unknownResolution.previousStatus,'unknown')
        assert.equal(closed.error,unknown.error)
        assert.equal((await (await review(true)).json()).reused,true)
        assert.equal((await invoke('studio_image_status',old)).job.unknownResolution.reason,'模拟供应商确认报错，无可恢复结果')
        const replacement = await invoke('studio_prepare_image',{projectId,requestId:randomUUID(),profileId:openaiProfile,
          targetType:'shot-last',targetId:s2,expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,
          visualPrompt:'New confirmed attempt'})
        assert.equal(replacement.job.status,'prepared')
        await invoke('studio_discard_image_plan',{projectId,requestId:replacement.job.requestId})
        const slots = async () => (await invoke('studio_shots',{
          projectId,shotId:s2,fields:['imageAssetId','firstFrameAssetId','imageConsistency','imageHistoryIds']
        })).items[0]
        const independent = (await slots()).imageAssetId
        const first = await openaiImage('First frame with separate main image',[],'succeeded',undefined,'shot-first')
        assert.equal(first.mainImageSynced,false)
        assert.equal((await slots()).imageConsistency,'diverged')
        assert.equal((await slots()).imageAssetId,independent)
        const bind = await invoke('studio_bind_shot_asset',{projectId,shotId:s2,slot:'image',assetId:first.assetId,expectedUpdatedAt:'auto'})
        assert.equal(bind.bind.affectsVideo,false)
        assert.equal(bind.imageConsistency,'same')
        const nextFirst = await openaiImage('Refresh linked first frame',[],'succeeded',undefined,'shot-first')
        assert.equal(nextFirst.mainImageSynced,true)
        assert.equal((await slots()).imageAssetId,nextFirst.assetId)
        assert.equal((await slots()).firstFrameAssetId,nextFirst.assetId)
        assert.ok((await slots()).imageHistoryIds.includes(first.assetId))
        const noMain=JSON.parse(await fs.readFile(boardProjectPath,'utf8'))
        delete noMain.shots.find(shot=>shot.id===s2).imageAssetId
        const oldStamp=noMain.updatedAt
        noMain.updatedAt+=1
        await ok('writeText',{path:boardProjectPath,expectedUpdatedAt:oldStamp,value:JSON.stringify(noMain)})
        assert.equal((await slots()).imageConsistency,'first_only')
        const filledMain = await openaiImage('First frame fills empty main image',[],'succeeded',undefined,'shot-first')
        assert.equal(filledMain.mainImageSynced,true)
        assert.equal((await slots()).imageAssetId,filledMain.assetId)
        assert.equal((await slots()).imageConsistency,'same')
        console.log('PASS OpenAI Images MCP preflight, text-to-image, three ordered image edits, automatic archive and uncertain submission deduplication')
        const referenceProfile = await imageProfile(true)
        const inputs = await invoke('studio_describe_image_inputs', { profileId: referenceProfile })
        assert.equal(inputs.referenceCount, 1)
        assert.equal(inputs.referenceSlots[0].nodeId, '2')
        assert.equal(inputs.referenceSlots[0].field, 'image')
        assert.equal(inputs.referenceSlots[0].source, 'inferred')
        const invalidProfile = await imageProfile(true)
        await invoke('studio_model_update', { kind:'image', profileId:invalidProfile, inputMap:{bindings:[
          { role:'prompt', nodeId:'1', field:'text' }, { role:'image', nodeId:'missing', field:'image' }
        ]} })
        const invalidSlots = await fetch(origin + '/api/dramlo/mcp', { method:'POST', headers:mcpHeaders,
          body:JSON.stringify({name:'studio_describe_image_inputs',input:{profileId:invalidProfile}}) })
        assert.equal(invalidSlots.status, 200)
        const diagnosis = await invalidSlots.json()
        assert.equal(diagnosis.ready, false)
        assert.match(diagnosis.issue, /映射/)
        assert.equal(diagnosis.promptCandidates[0].nodeId, '1')

        const partialProfile = await imageProfile(true)
        const multiReference = {
          '1':{class_type:'CLIPTextEncode',inputs:{text:'prompt'}},
          '2':{class_type:'LoadImage',inputs:{image:'bound.png'}},
          '3':{class_type:'LoadImage',inputs:{image:'fixed-mask.png'}},
          '4':{class_type:'LoadImage',inputs:{image:'disconnected.png'}},
          '5':{class_type:'ReferenceConsumer',inputs:{image:['2',0],mask:['3',1]}},
          '6':{class_type:'SaveImage',inputs:{images:['5',0]}}
        }
        const bindings = [{role:'prompt',nodeId:'1',field:'text'}, {role:'image',nodeId:'2',field:'image'}]
        await invoke('studio_model_update', {kind:'image',profileId:partialProfile,
          workflowTemplate:JSON.stringify(multiReference),inputMap:{bindings}})
        const partial = await invoke('studio_describe_image_inputs',{profileId:partialProfile})
        assert.equal(partial.ready,false)
        assert.deepEqual(partial.templateImages,[{nodeId:'3',field:'image',filename:'fixed-mask.png'}])
        const partialRemoval = await invoke('studio_remove_model',{kind:'image',profileId:partialProfile})
        assert.equal(partialRemoval.ready,false)
        assert.deepEqual(partialRemoval.missingMapping,['image'])
        const partialModel = await invoke('studio_describe_model',{kind:'image',profileId:partialProfile})
        assert.deepEqual(partialModel.missingMapping,partialRemoval.missingMapping)
        assert.deepEqual(partialModel.workflow.missingMapping,partialModel.missingMapping)
        assert.equal(partialRemoval.mappingChecked,true)
        const beforeBlockedImage = imageGraphs.length
        const blockedImage = await fetch(origin + '/api/dramlo/mcp', {method:'POST',headers:mcpHeaders,
          body:JSON.stringify({name:'studio_prepare_image',input:{projectId, requestId:randomUUID(),
            expectedUpdatedAt:(await invoke('studio_open_project',{projectId})).updatedAt,
            targetType:'shot-image',targetId:s2,profileId:partialProfile,prompt:'masked fixture',referenceAssetIds:[sceneImage]}})})
        assert.equal(blockedImage.status,400)
        assert.match((await blockedImage.json()).error,/fixed-mask\.png/)
        assert.equal(imageGraphs.length,beforeBlockedImage)
        await invoke('studio_model_update',{kind:'image',profileId:partialProfile,
          inputMap:{bindings:[...bindings,{role:'image',nodeId:'3',field:'image'}]}})
        const fullyBound = await invoke('studio_describe_image_inputs',{profileId:partialProfile})
        assert.equal(fullyBound.ready,true)
        assert.deepEqual(fullyBound.templateImages,[])

        const frame = await makeImage({ targetType:'shot-image', targetId:s2, profileId:referenceProfile, prompt:'keyframe', referenceAssetIds:[sceneImage] })
        assert.equal(imageGraphs[0]['23:24'].inputs.value, 'scene anchor')
        assert.ok(!JSON.stringify(imageGraphs[0]).includes('仅用于项目定位'))
        assert.equal(imageGraphs[1]['2'].inputs.image, 'scene-ref.png')
        assert.ok(refUploads[0].includes(pngOutput))
        assert.equal((await invoke('studio_project_items', { projectId, type:'scenes' })).items.find(scene => scene.id === firstScene).plateAssetId, sceneImage)
        assert.equal((await invoke('studio_shots', { projectId })).items.find(shot => shot.id === s2).imageAssetId, frame)
        const preflight = await invoke('studio_check_video_readiness', { projectId, shots:[
          { shotId:s2, profileId:profiles[1], mode:'first-frame', firstFrameAssetId:frame }
        ] })
        assert.equal(preflight.ready, true, JSON.stringify(preflight))
        assert.equal(preflight.items[0].frameSources.firstFrameAssetId, frame)
        assert.equal((await invoke('studio_current_image_job', { projectId, targetType:'shot-image', targetId:s2 })).job.assetId, frame)
        const videoKey = { projectId, requestId:randomUUID() }
        await invoke('studio_prepare_video', { ...videoKey, expectedUpdatedAt:preflight.updatedAt, shots:[
          { shotId:s2, profileId:profiles[1], mode:'first-frame', firstFrameAssetId:frame }
        ] })
        await invoke('studio_confirm_video', { ...videoKey, confirmed:true })
        let chained
        for (let attempt = 0; attempt < 100; attempt++) {
          chained = (await invoke('studio_video_status', videoKey)).batch
          if (chained.status !== 'running') break
          await new Promise(resolve => setTimeout(resolve, 100))
        }
        assert.equal(chained.status, 'succeeded', JSON.stringify(chained))
        assert.equal((await invoke('studio_project_items', { projectId, type:'assets', limit:50 })).items.find(item => item.id === chained.steps[0].assetId).kind, 'video')

        const imported = await invoke('studio_import_image_asset', { projectId, requestId:randomUUID(), dataBase64:pngOutput.toString('base64') })
        const revision = (await invoke('studio_open_project', { projectId })).updatedAt
        const attached = await invoke('studio_attach_image_asset', {
          projectId, expectedUpdatedAt:'auto', targetType:'prop', targetId:propId, assetId:imported.assetId
        })
        assert.ok(attached.updatedAt > revision)
        assert.equal((await invoke('studio_project_items', { projectId, type:'props' })).items.find(prop => prop.id === propId).designSheetAssetId, imported.assetId)
        const planId = randomUUID()
        const beforePlan = imageGraphs.length
        const steps = [
          {id:'draw',kind:'image',input:{targetType:'scene',targetId:firstScene,profileId:textImageProfile,visualPrompt:'plan scene'}},
          {id:'frame',kind:'image',input:{targetType:'shot-image',targetId:s2,profileId:referenceProfile,visualPrompt:'plan frame'},
            sources:[{stepId:'draw',slot:'reference:0'}]},
          {id:'movie',kind:'video',input:{shotId:s2,profileId:profiles[1],mode:'first-frame'},
            sources:[{stepId:'frame',slot:'firstFrame'}]},
          {id:'tail',kind:'tail',nodeId:extensionId,input:{},sources:[{stepId:'movie',slot:'video'}]},
          {id:'final',kind:'export',input:{clips:[{id:'plan-clip',shotId:s2,startTime:0,duration:1,sourceIn:0,
            speed:1,volume:1,enabled:true,transitionIn:'none',transitionDuration:0}]},after:['movie','tail']}
        ]
        let saved = await invoke('studio_save_plan',{projectId,planId,name:'持久化执行验收',
          expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,steps:[...steps].reverse()})
        const graph = await invoke('studio_canvas_graph',{projectId,planId})
        assert.deepEqual(graph.execution.map(item=>[item.id,item.state]),
          [['draw','ready'],['frame','waiting'],['movie','waiting'],['tail','waiting'],['final','waiting']])
        assert.equal(graph.execution[2].requires[0].source,'frame')
        assert.deepEqual(graph.execution[4].reason.stepIds,['movie','tail'])
        assert.equal(graph.execution[4].reason.code,'waiting_dependency')
        assert.ok(graph.execution[4].requires.some(item=>item.slot==='clipMedia' && item.source==='movie' && item.clipId==='plan-clip'))
        const partialExportId = randomUUID()
        const exportWithMissingClip = structuredClone(steps[4])
        const missingShot = randomUUID()
        exportWithMissingClip.input.clips.push({...exportWithMissingClip.input.clips[0],id:'missing-clip',shotId:missingShot,startTime:1})
        await invoke('studio_save_plan',{projectId,planId:partialExportId,name:'另有片段缺素材',
          expectedUpdatedAt:graph.updatedAt,steps:[...steps.slice(0,4),exportWithMissingClip]})
        const partialExport = (await invoke('studio_canvas_graph',{projectId,planId:partialExportId})).execution[4]
        assert.equal(partialExport.state,'blocked')
        assert.deepEqual(partialExport.reason.issues,[{kind:'missing-clip-media',clipId:'missing-clip',shotId:missingShot,clipIndex:2}])
        assert.ok(partialExport.requires.some(item=>item.slot==='clipMedia' && item.source==='movie' && item.state==='waiting'))
        const focused = await invoke('studio_canvas_graph',{projectId,planId,nodeId:'shotVideo:'+s2})
        assert.deepEqual(focused.focus.executionPaths[0].upstream,['frame','draw'])
        assert.equal(imageGraphs.length,beforePlan)
        assert.equal((await invoke('studio_current_plan',{projectId})).plans.find(item=>item.id===planId).steps[0].input.visualPrompt,'plan scene')
        const run = async expectedUpdatedAt => {
          const { executionRevision } = await invoke('studio_canvas_graph',{projectId,planId})
          return invoke('studio_run_plan',{projectId,planId,expectedPlanVersion:saved.plan.version,
            expectedExecutionRevision:executionRevision,expectedUpdatedAt,confirmed:true})
        }
        const status = async (state) => {
          for (let attempt=0;attempt<160;attempt++) {
            const next = await invoke('studio_current_plan',{projectId,includeCompleted:true})
            const plan = next.plans.find(item=>item.id===planId)
            if (plan.state === state) return next
            assert.ok(['running', 'paused'].includes(plan.state),JSON.stringify(plan))
            await new Promise(resolve=>setTimeout(resolve,100))
          }
          throw new Error('plan did not reach '+state)
        }
        const launch = await fetch(origin + '/api/dramlo/execution-plan',{
          method:'POST',headers:{'X-Dramlo-UI':'1','Content-Type':'application/json'},
          body:JSON.stringify({name:'studio_run_plan',input:{projectId,planId,expectedPlanVersion:saved.plan.version,
            expectedExecutionRevision:graph.executionRevision,expectedUpdatedAt:graph.updatedAt,confirmed:true}})
        })
        assert.equal(launch.status,200,await launch.clone().text())
        for (let attempt=0;attempt<50 && imageGraphs.length===beforePlan;attempt++) {
          await new Promise(resolve=>setTimeout(resolve,20))
        }
        assert.equal(imageGraphs.length,beforePlan+1)
        await invoke('studio_pause_plan',{projectId,planId})
        const paused = await status('paused')
        const started = paused.plans.find(item=>item.id===planId)
        assert.equal(started.progress.draw.state,'succeeded')
        assert.equal(started.progress.frame,undefined)
        saved = await invoke('studio_save_plan',{projectId,planId,name:'持久化执行验收',
          expectedUpdatedAt:paused.updatedAt,expectedPlanVersion:saved.plan.version,steps:started.steps})
        assert.equal(saved.plan.version,2)
        await stop()
        origin = await start()
        ;({ ok } = await commands(origin))
        await invoke('studio_open_project',{projectId})
        assert.equal((await invoke('studio_current_plan',{projectId})).plans.find(item=>item.id===planId).state,'paused')
        await run(paused.updatedAt)
        const complete = await status('succeeded')
        assert.equal(imageGraphs.length,beforePlan+2)
        assert.equal((await run(complete.updatedAt)).reused,true)
        const progress = complete.plans.find(item=>item.id===planId).progress
        assert.equal(Object.keys(progress).length,5)
        const timeline = await invoke('studio_timeline',{projectId})
        assert.equal(timeline.items[0].assetId,progress.movie.assetId)
        assert.equal(timeline.finalVideoAssetId,progress.final.assetId)
        const rejectedId = randomUUID()
        const singleStep = {...steps[0],input:{...steps[0].input,visualPrompt:'plan retry'}}
        const failedDraft = await invoke('studio_save_plan',{projectId,planId:rejectedId,name:'失败停止验收',
          expectedUpdatedAt:complete.updatedAt,steps:[singleStep]})
        const runFailure = async (retryFailed) => {
          const check = await invoke('studio_canvas_graph',{projectId,planId:rejectedId})
          await invoke('studio_run_plan',{projectId,planId:rejectedId,expectedPlanVersion:failedDraft.plan.version,
            expectedUpdatedAt:check.updatedAt,expectedExecutionRevision:check.executionRevision,confirmed:true,...(retryFailed?{retryFailed:true}:{})})
          for (let attempt=0;attempt<60;attempt++) {
            const plans = await invoke('studio_current_plan',{projectId,includeCompleted:true})
            const job = plans.plans.find(item=>item.id===rejectedId)
            if (job.state !== 'running') return job.state
            await new Promise(resolve=>setTimeout(resolve,100))
          }
          throw new Error('plan failure did not stop')
        }
        const beforeFailure = imageGraphs.length
        rejectPlanPrompt = true
        assert.equal(await runFailure(false),'blocked')
        rejectPlanPrompt = false
        assert.equal(await runFailure(false),'blocked')
        assert.equal(imageGraphs.length,beforeFailure+1)
        assert.equal(await runFailure(true),'succeeded')
        assert.equal(imageGraphs.length,beforeFailure+2)
        const recoverId = randomUUID()
        const recoverDraft = await invoke('studio_save_plan',{projectId,planId:recoverId,name:'服务重启后接手',
          expectedUpdatedAt:(await invoke('studio_project',{projectId})).project.updatedAt,steps:[singleStep]})
        const runRecover = async () => {
          const check = await invoke('studio_canvas_graph',{projectId,planId:recoverId})
          return invoke('studio_run_plan',{projectId,planId:recoverId,expectedPlanVersion:recoverDraft.plan.version,
            expectedUpdatedAt:check.updatedAt,expectedExecutionRevision:check.executionRevision,confirmed:true})
        }
        const beforeRecovery = imageGraphs.length
        holdPlanHistory = true
        await runRecover()
        for (let attempt=0;attempt<100 && !planHistorySeen;attempt++) await new Promise(resolve=>setTimeout(resolve,20))
        assert.ok(planHistorySeen)
        await stop()
        origin = await start()
        ;({ ok } = await commands(origin))
        assert.equal((await invoke('studio_current_plan',{projectId})).plans.find(item=>item.id===recoverId).state,'interrupted')
        holdPlanHistory = false
        await runRecover()
        let recovered
        for (let attempt=0;attempt<60;attempt++) {
          recovered = (await invoke('studio_current_plan',{projectId,includeCompleted:true})).plans.find(item=>item.id===recoverId)
          if (recovered.state !== 'running') break
          await new Promise(resolve=>setTimeout(resolve,100))
        }
        assert.equal(recovered.state,'succeeded',JSON.stringify(recovered))
        assert.equal(imageGraphs.length,beforeRecovery+1)
        await invoke('studio_open_project',{projectId})
        console.log('PASS persisted execution graph, zero-output wiring, pause, resume and duplicate protection')
        console.log('PASS server image workflow: scene, keyframe, video archive, image import and prop attachment')
      } finally { await new Promise(resolve => imageMock.close(resolve)) }
      const editing = await invoke('studio_open_project', { projectId })
      const reorder = { projectId, expectedUpdatedAt: editing.updatedAt, shotIds: [s3, s1, s2] }
      const previewOrder = await invoke('studio_reorder_shots', reorder)
      assert.equal(previewOrder.applied, false)
      assert.equal(previewOrder.updatedAt, editing.updatedAt)
      const applied = await invoke('studio_reorder_shots', { ...reorder, confirmed: true })
      const timing = (await invoke('studio_shots', { projectId, sceneId: firstScene })).items
      assert.deepEqual(timing.map(item => [item.id, item.plannedStartSec, item.plannedEndSec]), [[s1, 5, 10], [s2, 10, 15]])
      const deletion = { projectId, expectedUpdatedAt: applied.updatedAt, shotId: s1 }
      const previewDeletion = await invoke('studio_remove_shot', deletion)
      assert.ok(previewDeletion.removedNodeIds.includes(source.id))
      assert.ok(previewDeletion.removedEdgeIds.includes(edge.edge.id))
      assert.ok(previewDeletion.continuityReview.some(item => item.shotId === s2))
      const removed = await invoke('studio_remove_shot', { ...deletion, confirmed: true })
      assert.ok(!(await invoke('studio_project_items', { projectId, type: 'nodes' })).items.some(item => item.id === source.id))
      assert.ok((await invoke('studio_project_items', { projectId, type: 'assets', limit:50 })).items.some(item => item.id === status.steps[0].assetId))
      const insertedId = randomUUID()
      const insertion = await invoke('studio_replace_shots',{projectId,expectedUpdatedAt:removed.updatedAt,
        mode:'insert',afterShotId:s3,shots:[{...draft(insertedId),sceneId:firstScene}]})
      assert.deepEqual(insertion.shotIds,[s3,insertedId,s2])
      const insertedShots = (await invoke('studio_shots',{projectId})).items
      const inserted = insertedShots.find(shot=>shot.id===insertedId)
      assert.equal(inserted.voiceover,'')
      assert.equal(inserted.audioDescription,'')
      const beforeCut = insertedShots.find(shot=>shot.id===s2)
      const cut = {projectId,expectedUpdatedAt:insertion.updatedAt,mode:'replace',
        shotIds:[s3,insertedId,s2], shots:[{...draft(s2),sceneId:firstScene,duration:7},{...draft(insertedId),sceneId:nextScene}]}
      const previewCut = await invoke('studio_replace_shots',cut)
      assert.equal(previewCut.applied,false)
      assert.deepEqual(previewCut.removedShotIds,[s3])
      const recut = await invoke('studio_replace_shots',{...cut,confirmed:true})
      assert.deepEqual(recut.shotIds,[s2,insertedId])
      const recutShots = (await invoke('studio_shots',{projectId})).items
      assert.equal(recutShots[0].videoAssetId,beforeCut.videoAssetId)
      assert.equal(recutShots[0].plannedEndSec,7)
      assert.equal(recutShots[1].sceneId,nextScene)
      assert.ok((await invoke('studio_project_items',{projectId,type:'assets',limit:50})).items.some(item=>item.id===status.steps[2].assetId))
      const clip = {id:'export-clip',shotId:s2,assetId:status.steps[0].assetId,startTime:0,duration:0.8,sourceIn:0.1,
        transitionIn:'none',transitionDuration:0,speed:1,volume:1,enabled:true}
      const captionSrt = '1\n00:00:00,000 --> 00:00:00,800\n字幕保留\n'
      const editedTimeline = await invoke('studio_edit_timeline',{projectId,expectedUpdatedAt:recut.updatedAt,clips:[clip],captionSrt})
      const timeline = await invoke('studio_timeline',{projectId})
      assert.deepEqual(timeline.items,[clip])
      assert.equal(timeline.finalVideoStale,true)
      const exportKey = {projectId,requestId:randomUUID()}
      const exportInput = {...exportKey,expectedUpdatedAt:editedTimeline.updatedAt}
      const preparedFilm = (await invoke('studio_export_final',exportInput)).job
      assert.equal(preparedFilm.status,'prepared')
      assert.equal(preparedFilm.burnSubtitles,false)
      const idlessPreview = (await invoke('studio_export_final',{projectId,expectedUpdatedAt:editedTimeline.updatedAt})).job
      assert.equal(idlessPreview.status,'prepared')
      assert.equal(idlessPreview.requestId,null)
      const burnBlocked = await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
        body:JSON.stringify({name:'studio_export_final',input:{...exportInput,burnSubtitles:true}})})
      assert.equal(burnBlocked.status,400)
      assert.equal((await invoke('studio_export_final',{...exportInput,confirmed:true})).job.status,'running')
      let film
      for(let i=0;i<100;i++){
        film=(await invoke('studio_export_status',exportKey)).job
        if(film.status!=='running') break
        await new Promise(resolve=>setTimeout(resolve,100))
      }
      assert.equal(film.status,'succeeded',JSON.stringify(film))
      assert.equal(film.matchesCurrentTimeline,true)
      assert.equal(film.output.selected,true)
      assert.equal((await invoke('studio_export_status',{projectId})).job.requestId,exportKey.requestId)
      const finishedTimeline = await invoke('studio_timeline',{projectId})
      assert.equal(finishedTimeline.finalVideoAssetId,film.output.assetId)
      assert.equal(finishedTimeline.finalVideoStale,false)
      assert.equal(finishedTimeline.captionSrt,captionSrt)
      assert.ok((await fs.stat(film.output.path)).size>1024)
      assert.equal((await invoke('studio_export_final',{...exportInput,confirmed:true})).reused,true)
      const renamedFilm = await invoke('studio_rename_project',{projectId,expectedUpdatedAt:finishedTimeline.updatedAt,name:'原片不受改名影响'})
      assert.equal((await invoke('studio_timeline',{projectId})).finalVideoStale,false)
      await invoke('studio_edit_timeline',{projectId,expectedUpdatedAt:renamedFilm.updatedAt,clips:[{...clip,duration:0.5}]})
      const revisedTimeline = await invoke('studio_timeline',{projectId})
      assert.equal(revisedTimeline.finalVideoStale,true)
      assert.equal(revisedTimeline.finalVideoAssetId,film.output.assetId)
      const reusedFilm = await invoke('studio_export_final',{...exportKey,expectedUpdatedAt:revisedTimeline.updatedAt,confirmed:true})
      assert.equal(reusedFilm.reused,true)
      assert.equal(reusedFilm.job.matchesCurrentTimeline,false)
      assert.equal(reusedFilm.job.duration,0.8)
      assert.equal(reusedFilm.job.output.assetId,film.output.assetId)
      assert.match(reusedFilm.job.reuseNotice,/新的请求 ID/)
      assert.equal((await invoke('studio_export_status',exportKey)).job.matchesCurrentTimeline,false)
      const newPlan = await invoke('studio_export_final',{projectId,requestId:randomUUID(),expectedUpdatedAt:revisedTimeline.updatedAt})
      assert.equal(newPlan.job.matchesCurrentTimeline,true)
      assert.equal(newPlan.job.duration,0.5)
      const brokenTimeline = await invoke('studio_edit_timeline',{projectId,expectedUpdatedAt:revisedTimeline.updatedAt,clips:[
        {...clip,id:'cut-a',duration:0.5},
        {...clip,id:'cut-b',startTime:0.8,duration:0.6},
        {...clip,id:'cut-c',startTime:1.2,duration:0.5},
        {...clip,id:'disabled',startTime:0.6,enabled:false}
      ]})
      assert.equal(brokenTimeline.warnings.length,2)
      const brokenRead = await invoke('studio_timeline',{projectId,limit:1})
      assert.equal(brokenRead.finalVideoStale,true)
      assert.deepEqual(brokenRead.timelineIssues,brokenTimeline.timelineIssues)
      assert.deepEqual(brokenRead.timelineIssues.map(({afterClipId,clipId,kind,seconds})=>[afterClipId,clipId,kind,seconds]),
        [['cut-a','cut-b','gap',0.3],['cut-b','cut-c','overlap',0.2]])
      const exportProbe = {projectId,requestId:randomUUID(),expectedUpdatedAt:brokenTimeline.updatedAt}
      const rejectedExport = await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
        body:JSON.stringify({name:'studio_export_final',input:exportProbe})})
      assert.equal(rejectedExport.status,400)
      assert.deepEqual((await rejectedExport.json()).issues,brokenRead.timelineIssues)
      const timelineClient = new Client({name:'timeline-diagnostic',version:'1'})
      try {
        await timelineClient.connect(new StdioClientTransport({command:binary,args:['mcp','studio'],
          env:{...env,DRAMLO_WEB_MCP_URL:origin}}))
        const brokenExport = await timelineClient.callTool({name:'studio_export_final',arguments:exportProbe})
        assert.equal(brokenExport.isError,true)
        assert.deepEqual(brokenExport.structuredContent.issues,brokenRead.timelineIssues)
        assert.match(brokenExport.content[0].text,/片段 1.*0\.300/)
      } finally { await timelineClient.close() }
      const revisedPrompt = '[00:03–00:08] 角色回头'
      const timedPrompt = await invoke('studio_set_video_prompt',{projectId,shotId:s2,
        expectedUpdatedAt:'auto',prompt:revisedPrompt})
      assert.ok(timedPrompt.updatedAt > brokenTimeline.updatedAt)
      const revisedShots = await invoke('studio_shots',{ projectId, fields:['id','compiledVideoPrompt'] })
      assert.equal(revisedShots.items.find(shot => shot.id === s2)?.compiledVideoPrompt,revisedPrompt)
      assert.match(timedPrompt.warnings[0],/7 秒/)
      console.log('PASS auto revisions save asset bindings and video prompts, with readback')
      const timingFixed = await invoke('studio_update_shot_inputs',{projectId,shotId:s2,
        expectedUpdatedAt:timedPrompt.updatedAt,duration:8})
      assert.deepEqual(timingFixed.warnings,[])
      console.log('PASS native timeline editing and local FFmpeg export with archived output and idempotent confirmation')
      console.log('PASS cross-scene insert and recut preserve selected and removed-shot media')
      console.log('PASS readiness without side effects, planned timing, reorder and shot removal with retained assets')
      // Exercise the WB capture tools themselves, not just ordinary shot queues.
      await invoke('studio_select_model',{kind:'video',profileId:profiles[1]})
      const imageProfiles = await invoke('studio_list_models',{kind:'image'})
      const imageFixture = imageProfiles.profiles.find(profile=>profile.name==='Images API fixture')
      assert.ok(imageFixture)
      await invoke('studio_select_model',{kind:'image',profileId:imageFixture.id})
      const captureProject=(await invoke('studio_create_project',{name:'连续拍隔离夹具'})).projectId
      const segment=(name)=>({title:name,duration:6,prompt:name,startState:'起幅',endState:'落幅',sound:'环境声'})
      const makeScene=(id,segments)=>({id,name:id,sourceText:'场次正文',openingPrompt:'单幅空镜',assetIds:[],segments})
      const plan={title:'连续拍',aesthetic:'测试画风',assets:[],
        scenes:[makeScene('场次甲',[segment('第一镜'),segment('第二镜')]),makeScene('场次乙',[segment('第三镜')])]}
      const captureBase={projectId:captureProject,expectedUpdatedAt:'auto'}
      const captureError=async (name,input,pattern)=>{
        const response=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
          body:JSON.stringify({name,input}),signal:AbortSignal.timeout(6000)})
        assert.equal(response.status,400)
        assert.match((await response.json()).error,pattern)
      }
      await captureError('studio_capture_set_plan',{...captureBase,plan:{...plan,scenes:[plan.scenes[0],plan.scenes[0]]}},/场景 ID 重复/)
      const capture=await invoke('studio_capture_set_plan',{...captureBase,plan})
      assert.equal(capture.approvalMode,'batch')
      const captureKey={...captureBase,captureId:capture.captureId}
      await captureError('studio_capture_approve',{...captureKey,phase:'assets'},/sceneIndex/)
      await captureError('studio_capture_approve',{...captureKey,phase:'assets',sceneIndex:0},/尚未归档/)
      const editable=(await invoke('studio_capture_state',{projectId:captureProject})).capture.scenes[0].segments[0].nodeId
      await invoke('studio_canvas_node_update',{...captureBase,nodeId:editable,prompt:'人工修改后的第一镜'})
      const videoCount=submits
      const imageCount=openaiRequests.length
      const uploadCount=uploads.length
      const captureDraft=await invoke('studio_capture_prepare',captureKey)
      assert.equal(captureDraft.plan.steps.length,5)
      assert.equal((await invoke('studio_capture_prepare',captureKey)).reused,true)
      assert.equal(submits,videoCount)
      assert.equal(openaiRequests.length,imageCount)
      const captureRun={projectId:captureProject,planId:captureDraft.plan.id}
      const graph=await invoke('studio_canvas_graph',captureRun)
      assert.ok(!graph.execution.some(step=>step.state==='blocked'),JSON.stringify(graph.execution))
      const approval={...captureRun,expectedUpdatedAt:graph.updatedAt,expectedPlanVersion:graph.planVersion,
        expectedExecutionRevision:graph.executionRevision,confirmed:true}
      assert.equal((await invoke('studio_run_plan',approval)).plan.state,'running')
      assert.equal((await invoke('studio_run_plan',approval)).reused,true)
      let completedCapture
      for(let i=0;i<150;i++){
        completedCapture=(await invoke('studio_current_plan',{projectId:captureProject,includeCompleted:true})).plans.find(item=>item.id===captureRun.planId)
        if(completedCapture?.state!=='running') break
        await new Promise(resolve=>setTimeout(resolve,100))
      }
      assert.equal(completedCapture?.state,'succeeded',JSON.stringify(completedCapture))
      assert.equal(submits-videoCount,3)
      assert.equal(graphs[videoCount]['1'].inputs.prompt,'人工修改后的第一镜')
      assert.equal(openaiRequests.length-imageCount,2)
      assert.equal(uploads.length-uploadCount,3)
      assert.ok(uploads[uploadCount].includes(pngFixture))
      assert.ok(!uploads[uploadCount+1].includes(pngFixture),'same-scene second shot must use the prior video tail')
      assert.ok(uploads[uploadCount+2].includes(pngFixture),'a new scene must start from its own still')
      const captureRead=(await invoke('studio_capture_state',{projectId:captureProject})).capture
      assert.equal(captureRead.phase,'complete')
      assert.equal((await invoke('studio_shots',{projectId:captureProject})).total,0)
      const firstAssetApproval=await invoke('studio_capture_approve',{...captureKey,phase:'assets',sceneIndex:0})
      const repeated=await invoke('studio_capture_approve',{...captureKey,phase:'assets',sceneIndex:0,
        expectedUpdatedAt:firstAssetApproval.updatedAt-1})
      assert.equal(repeated.reused,true)
      assert.equal(repeated.sceneIndex,1)
      const tailNodeId=captureRead.scenes[0].segments[0].nodeId
      const extractedReply=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
        body:JSON.stringify({name:'studio_extract_tail_frame',input:{...captureBase,nodeId:tailNodeId}}),
        signal:AbortSignal.timeout(6000)})
      assert.equal(extractedReply.status,200,await extractedReply.clone().text())
      const extracted=await extractedReply.json()
      const located=await invoke('studio_locate_asset',{projectId:captureProject,assetId:extracted.assetId})
      assert.ok((await fs.stat(located.path)).isFile())
      const subsequent=await fetch(origin+'/api/dramlo/mcp',{method:'POST',headers:mcpHeaders,
        body:JSON.stringify({name:'studio_project',input:{projectId:captureProject}}),signal:AbortSignal.timeout(3000)})
      assert.equal(subsequent.status,200)
      const sceneProject=(await invoke('studio_create_project',{name:'逐场确认夹具'})).projectId
      const sceneMode={projectId:sceneProject,expectedUpdatedAt:'auto'}
      const sceneCapture=await invoke('studio_capture_set_plan',{...sceneMode,plan,approvalMode:'scene'})
      const sceneKey={...sceneMode,captureId:sceneCapture.captureId}
      await captureError('studio_capture_prepare',{...sceneKey,sceneIndex:1},/先完成前面/)
      const onlyFirst=await invoke('studio_capture_prepare',{...sceneKey,sceneIndex:0})
      assert.equal(onlyFirst.plan.steps.length,3)
      await invoke('studio_open_project',{projectId})
      for(const [targetType,promptSource] of [['shot-first','firstFramePrompt'],['shot-last','lastFramePrompt']]){
        const requestId=randomUUID()
        await invoke('studio_set_asset_prompt',{projectId,expectedUpdatedAt:'auto',targetType,targetId:s2,
          visualPrompt:'人物站在窗边'})
        const preview=await invoke('studio_prepare_image',{projectId,requestId,expectedUpdatedAt:'auto',
          targetType,targetId:s2,profileId:imageFixture.id,referenceAssetIds:[]})
        assert.equal(preview.job.promptSource,promptSource)
        await invoke('studio_discard_image_plan',{projectId,requestId})
      }
      console.log('PASS continuous capture: duplicate protection, one approval, serial real tail, output archive and next-scene guard')
      await invoke('studio_open_project', { projectId: 'fixture' })
      console.log('PASS headless Rust two-scene sequence and FFmpeg tail extraction')
    }

  } finally {
    await client.close()
  }
  await stop()
  origin = await start()
  ;({ ok } = await commands(origin))
  assert.deepEqual(JSON.parse(await ok('readText', { path: projectPath })), saved)
  assert.equal(await ok('kvGet', { name: 'test-key' }), 'persisted')
  if (process.env.DRAMLO_TEST_BROWSER) {
    const { stdout } = await promisify(execFile)(process.env.DRAMLO_TEST_BROWSER, [
      '--headless', '--no-first-run', '--no-default-browser-check', '--disable-background-networking',
      '--disable-gpu', '--user-data-dir=' + path.join(scratch, 'browser'), '--virtual-time-budget=5000', '--dump-dom',
      origin + '/#/settings/external'
    ], { windowsHide: true, timeout: 28000, maxBuffer: 1024 * 1024 })
    assert.match(stdout, /下载接入说明/)
    assert.match(stdout, /同一个 MCP 连接覆盖/)
    assert.doesNotMatch(stdout, /下载 Skill/)
    console.log('PASS browser rendering of the external Agent setup')
    const extensions = await promisify(execFile)(process.env.DRAMLO_TEST_BROWSER, [
      '--headless', '--no-first-run', '--disable-background-networking', '--disable-gpu',
      '--user-data-dir=' + path.join(scratch, 'extension-browser'), '--virtual-time-budget=5000', '--dump-dom',
      origin + '/#/extensions'
    ], { windowsHide: true, timeout: 28000, maxBuffer: 1024 * 1024 })
    assert.match(extensions.stdout, /下载 ComfyUI 清单模板/)
    assert.match(extensions.stdout, /提取视频尾帧/)
    console.log('PASS browser rendering of the extension library and built-in catalog')

  }
  const install = path.join(scratch, 'ComfyUI sample')
  const workflows = path.join(install, 'user', 'default', 'workflows')
  await fs.mkdir(workflows, { recursive: true })
  await fs.mkdir(path.join(install, 'comfy'))
  await fs.writeFile(path.join(install, 'main.py'), '# fixture')
  await fs.writeFile(path.join(workflows, '样例视频-api.json'), JSON.stringify({
    '1': { class_type: 'CR Prompt Text', inputs: { prompt: 'original' } },
    '2': { class_type: 'PrimitiveFloat', inputs: { value: 4 }, _meta: { title: 'duration' } },
    '3': { class_type: 'VHS_VideoCombine', inputs: {} }
  }))
  const managedClient = new Client({ name: 'studio-comfy-instances', version: '1' })
  try {
    await managedClient.connect(new StdioClientTransport({ command: binary, args: ['mcp', 'studio'],
      env: { ...env, DRAMLO_WEB_MCP_URL: origin } }))
    async function managed(name, args = {}) {
      const reply = await managedClient.callTool({ name: 'studio_comfy_' + name, arguments: args })
      assert.ok(!reply.isError, JSON.stringify(reply.content))
      return reply.structuredContent
    }
    const reservation = reservePort()
    reservation.listen(0, '127.0.0.1')
    await once(reservation, 'listening')
    const port = reservation.address().port
    await new Promise(resolve => reservation.close(resolve))
    const found = await managed('discover', { roots: [install], ports:[mock.address().port] })
    assert.equal(found.installations.length, 1)
    assert.deepEqual(found.registered,[])
    const onlineTarget = found.running[0].target
    assert.equal(onlineTarget.port,mock.address().port)
    const onlineFiles = await managed('workflows',{target:onlineTarget})
    const online = onlineFiles.items.find(item => item.relativePath === 'online.json')
    const onlineDetails = await managed('workflow_read',{...online.readInput,includeNodeInputs:true})
    assert.equal(onlineDetails.weights[0].filename,'fixture.safetensors')
    const nodeCatalog=onlineDetails.nodeCatalog
    assert.deepEqual(nodeCatalog.schemaByClass.ResolutionSelector.inputs.aspect_ratio.options,
      ['16:9 (Widescreen)','9:16 (Portrait Widescreen)'])
    assert.equal(nodeCatalog.schemaByClass.ResolutionSelector.inputs.megapixels.max,16)
    const onlineImport = await managed('import',online.readInput)
    assert.equal(onlineImport.kind,'image')
    assert.deepEqual((await managed('instances')).instances,[])
    const visibleTarget = {...found.installations[0].target,port:mock.address().port}
    await fs.writeFile(path.join(workflows,'online.json'),JSON.stringify(onlineWorkflow))
    await fs.writeFile(path.join(workflows,'same-name.json'),JSON.stringify({
      '1':{class_type:'SaveImage',inputs:{filename_prefix:'different disk graph'}}
    }))
    extraServerWorkflow = true
    const merged = await managed('workflows',{target:visibleTarget})
    const duplicate = merged.items.filter(item => item.relativePath === 'online.json')
    assert.equal(duplicate.length,1)
    assert.equal(duplicate[0].source,'server')
    assert.equal(duplicate[0].alternateLocation.source,'disk')
    assert.equal(duplicate[0].configurationStatus,'configured')
    assert.equal(duplicate[0].profiles[0].profileId,onlineImport.profileId)
    assert.deepEqual(merged.items.filter(item => item.relativePath === 'same-name.json').map(item => item.source),['disk','server'])
    assert.equal(merged.items.find(item => item.relativePath === 'server-only.json').source,'server')
    assert.equal(merged.items.find(item => item.relativePath === '样例视频-api.json').source,'disk')
    assert.equal(merged.items.find(item => item.relativePath === 'server-only.json').configurationStatus,'available')
    const reusedOnline = await managed('import',{target:visibleTarget,source:'server',relativePath:'online.json'})
    assert.equal(reusedOnline.profileId,onlineImport.profileId)
    const imageModels = await managedClient.callTool({name:'studio_list_models',arguments:{kind:'image'}})
    assert.equal(imageModels.structuredContent.workflowDiscovery.listTool,'studio_comfy_workflows')
    assert.equal(imageModels.structuredContent.profiles.filter(p => p.id === onlineImport.profileId).length,1)
    assert.equal(await fs.unlink(path.join(workflows,'online.json')),undefined)
    assert.equal(await fs.unlink(path.join(workflows,'same-name.json')),undefined)
    extraServerWorkflow = false
    console.log('PASS combined workflow catalogs retain separate same-name graphs and reuse configured aliases')

    const diskTarget = {...found.installations[0].target,port}
    const diskFiles = await managed('workflows',{target:diskTarget})
    const disk = diskFiles.items[0]
    const diskLocator = {target:diskTarget,source:disk.source,folder:disk.folder,relativePath:disk.relativePath}
    const browserDetails = await fetch(origin + '/api/dramlo/comfy-instances', {
      method:'POST',headers,body:JSON.stringify({name:'studio_comfy_workflow_read',input:diskLocator})
    })
    assert.equal(browserDetails.status,200)
    assert.ok((await browserDetails.json()).candidateSlots.some(slot => slot.role === 'prompt'))
    const transient = await managed('import',diskLocator)
    assert.equal(transient.kind,'video')
    assert.deepEqual((await managed('instances')).instances,[])
    assert.ok((await managedClient.callTool({name:'studio_comfy_start',arguments:{instanceId:'temporary',confirmed:true}})).isError)
    await testDirectBrowsePage(origin, mock.address().port)
    assert.deepEqual((await managed('instances')).instances,[])
    const registration = { root: install, port, confirmed: true }
    const { instance } = await managed('register', registration)
    assert.equal((await managed('register', registration)).reused, true)
    const { items } = await managed('workflows', { instanceId: instance.id })
    assert.equal(items.length, 1)
    const locator = { instanceId: instance.id, source: items[0].source, folder: items[0].folder, relativePath: items[0].relativePath }
    const read = await managed('workflow_read', { ...locator, includeApiJson: true })
    assert.equal(JSON.parse(read.apiJson)['3'].class_type, 'VHS_VideoCombine')
    const settingsBeforePreview=await ok('kvGet',{name:'settings'})
    const importPreview=await managed('import',{...locator,dryRun:true,select:true})
    assert.equal(importPreview.applied,false)
    assert.ok(importPreview.workflowFingerprint)
    assert.ok(importPreview.counts.prompt>0)
    assert.equal(await ok('kvGet',{name:'settings'}),settingsBeforePreview)
    const imported = await managed('import', {...locator,workflowFingerprint:importPreview.workflowFingerprint})
    assert.equal(imported.kind, 'video')
    assert.equal(imported.profileId,transient.profileId)
    assert.deepEqual(imported.missingMapping, [])
    const unchanged = await managed('import', { ...locator, select: true })
    assert.equal(unchanged.profileId, imported.profileId)
    assert.equal(unchanged.selected, true)
    assert.equal(unchanged.updated, false)
    const imageName = 'h3-placeholder-api.json'
    await fs.writeFile(path.join(workflows, imageName), JSON.stringify({
      '23:24':{class_type:'PrimitiveStringMultiline',inputs:{value:'sample'}},
      '23:19':{class_type:'StringReplace',inputs:{find:'{input text}',replace:['23:24',0]}},
      '27':{class_type:'SaveImage',inputs:{filename_prefix:'fixture'}}
    }))
    const foundImage = (await managed('workflows', { instanceId:instance.id })).items.find(item => item.relativePath === imageName)
    const bottledImage = await managed('import', { instanceId:instance.id, source:foundImage.source, folder:foundImage.folder, relativePath:foundImage.relativePath })
    assert.equal(bottledImage.kind,'image')
    assert.deepEqual(bottledImage.missingMapping,[])
    const importedDescription = (await managedClient.callTool({name:'studio_describe_image_inputs',arguments:{profileId:bottledImage.profileId}})).structuredContent
    assert.equal(importedDescription.ready,true)
    assert.equal(importedDescription.promptCandidates[0].nodeId,'23:24')

    const graphPath = path.join(workflows, '样例视频-api.json')
    const editedGraph = JSON.parse(await fs.readFile(graphPath, 'utf8'))
    editedGraph['1'].inputs.prompt = 'edited source'
    await fs.writeFile(graphPath, JSON.stringify(editedGraph))
    const updated = await managed('import', locator)
    assert.equal(updated.profileId, imported.profileId)
    assert.equal(updated.updated, true)
    const settingsAfterImport = JSON.parse(await ok('kvGet', { name: 'settings' }))
    assert.equal(JSON.parse(settingsAfterImport.video.adapter.workflowTemplate)['1'].inputs.prompt, 'edited source')
    let manageProcess = managed
    if (process.env.DRAMLO_TEST_PYTHON) {
      await promisify(execFile)(process.env.DRAMLO_TEST_PYTHON, ['-m', 'venv', '--without-pip', path.join(install, '.venv')], { windowsHide: true })
      await fs.writeFile(path.join(install, 'main.py'), [
        'import json, sys',
        'from pathlib import Path',
        'from http.server import BaseHTTPRequestHandler, HTTPServer',
        'class Handler(BaseHTTPRequestHandler):',
        '    def do_GET(self):',
        '        if self.path == "/system_stats": payload = {"system": {}, "devices": []}',
        '        elif self.path == "/queue": payload = {"queue_running": [1] if Path("busy").exists() else [], "queue_pending": []}',
        '        else: return self.send_error(404)',
        '        self.send_response(200)',
        '        self.end_headers()',
        '        self.wfile.write(json.dumps(payload).encode())',
        'HTTPServer(("127.0.0.1", int(sys.argv[sys.argv.index("--port")+1])), Handler).serve_forever()'
      ].join('\n'))
      const ref = { instanceId: instance.id, confirmed: true }
      await managed('start', ref)
      try {
        let state
        for (let i = 0; i < 50; i++) {
          state = (await managed('instances')).instances.find(item => item.id === instance.id)
          if (state.online) break
          await new Promise(resolve => setTimeout(resolve, 100))
        }
        assert.equal(state.online, true)
        assert.equal(state.managed, true)
        assert.equal((await managed('start', ref)).reused, true)
        const runningMarker = path.join(install, 'busy')
        await fs.writeFile(runningMarker, '')
        assert.equal((await managedClient.callTool({ name: 'studio_comfy_stop', arguments: ref })).isError, true)
        await fs.unlink(runningMarker)
        if (process.platform === 'win32') {
          await stop()
          origin = await start()
          ;({ ok } = await commands(origin))
          manageProcess = async (name, input = {}) => {
            const reply = await fetch(origin + '/api/dramlo/mcp', {
              method: 'POST', headers: mcpHeaders, body: JSON.stringify({ name: 'studio_comfy_' + name, input })
            })
            const body = await reply.json()
            assert.ok(reply.ok, JSON.stringify(body))
            return body
          }
          const recovered = (await manageProcess('instances')).instances.find(item => item.id === instance.id)
          assert.equal(recovered.online, true)
          assert.equal(recovered.managed, true)
          assert.equal((await manageProcess('start', ref)).reused, true)
        }
      } finally {
        await manageProcess('stop', ref)
      }
      for (let attempt = 0; attempt < 20; attempt++) {
        if (!(await manageProcess('instances')).instances.find(item => item.id === instance.id).online) break
        await new Promise(resolve => setTimeout(resolve, 50))
      }
      assert.equal((await manageProcess('instances')).instances.find(item => item.id === instance.id).online, false)
      console.log('PASS managed ComfyUI startup, reuse, active-queue protection and shutdown (local fixture)')
    }
    await manageProcess('remove', { instanceId: instance.id, confirmed: true })
    const settings = JSON.parse(await ok('kvGet', { name: 'settings' }))
    assert.ok(settings.videos.some(profile => profile.id === imported.profileId && profile.baseUrl === 'http://127.0.0.1:' + port))
    console.log('PASS ComfyUI discovery, per-instance catalog, MCP import, deduplication and safe unregister')
  } finally {
    await managedClient.close()
  }
  await stop()
  coldOrigin = origin
  const cold = new Client({ name: 'dramlo-web-cold-start', version: '1' })
  const coldTransport = new StdioClientTransport({ command: binary, args: ['mcp', 'studio'],
    env: { ...env, DRAMLO_WEB_MCP_URL: origin, DRAMLO_WEB_WORKSPACE: root, DRAMLO_WEB_FRONTEND: path.join(cwd, "dist") } })
  try {
    await cold.connect(coldTransport)
    async function call(name, args = {}) {
      const reply = await cold.callTool({ name, arguments: args })
      assert.ok(!reply.isError, JSON.stringify(reply.content))
      return reply.structuredContent
    }
    assert.equal((await call('studio_web_status')).ready, false)
    assert.equal((await call('studio_start_web')).reused, false)
    assert.equal((await call('studio_start_web')).reused, true)
    const active = await call('studio_open_project', { projectId: 'fixture' })
    assert.equal((await call('studio_save_exit', { projectId: 'fixture', expectedUpdatedAt: active.updatedAt, confirmed: true })).exitRequested, true)
    for (let attempt = 0; attempt < 30; attempt++) {
      if (!(await call('studio_web_status')).ready) break
      await new Promise((resolve) => setTimeout(resolve, 100))
    }
    assert.equal((await call('studio_web_status')).ready, false)
    coldOrigin = undefined
  } finally {
    await cold.close()
  }
  const offlineEnv = { ...env, DRAMLO_WEB_MCP_URL: origin }
  delete offlineEnv.DRAMLO_MCP_BRIDGE_TOKEN
  const offline = new Client({ name: 'studio-offline-no-token', version: '1' })
  try {
    await offline.connect(new StdioClientTransport({ command: binary, args: ['mcp', 'studio'], env: offlineEnv }))
    assert.ok((await offline.listTools()).tools.some(({ name }) => name === 'studio_start_web'))
    assert.equal((await offline.callTool({ name: 'studio_web_status' })).structuredContent.ready, false)
  } finally {
    await offline.close()
  }
  if (process.platform === 'win32') {
    const desktopPackage = path.join(scratch, 'dpapi')
    const desktopBinary = path.join(desktopPackage, 'dramlo-server.exe')
    await fs.mkdir(desktopPackage)
    await fs.copyFile(binary, desktopBinary)
    const dpapi = new Client({ name: 'studio-dpapi', version: '1' })
    const dpapiEnv = { ...offlineEnv, DRAMLO_WEB_WORKSPACE: root, DRAMLO_WEB_FRONTEND: frontend }
    const run = promisify(execFile)
    const shell = path.join(process.env.SystemRoot, 'System32', 'WindowsPowerShell', 'v1.0', 'powershell.exe')
    const dpapiFile = path.join(desktopPackage, 'launcher-token.dpapi')
    const shellEnv = Object.fromEntries(Object.entries(process.env).filter(([key]) => key.toLowerCase() !== 'psmodulepath'))
    async function call(name, args = {}) {
      const reply = await dpapi.callTool({ name, arguments: args })
      assert.ok(!reply.isError, JSON.stringify(reply.content))
      return reply.structuredContent
    }
    try {
      await dpapi.connect(new StdioClientTransport({ command: desktopBinary,
        args: ['mcp', 'studio', '--token-from-launcher'], env: dpapiEnv }))
      assert.equal((await call('studio_web_status')).ready, false)
      await assert.rejects(fs.access(dpapiFile), { code: 'ENOENT' })
      assert.equal((await call('studio_start_web')).ready, true)
      const shellResult = await run(shell, ['-NoProfile', '-NonInteractive', '-Command',
        "$ErrorActionPreference = 'Stop'; $secure = ConvertTo-SecureString -String (Get-Content -LiteralPath $env:DRAMLO_TEST_DPAPI_FILE -Raw); $secure.Length"],
        { env: { ...shellEnv, DRAMLO_TEST_DPAPI_FILE: dpapiFile }, windowsHide: true })
      assert.equal(shellResult.stdout.trim(), '64')
      const created = await call('studio_create_project', { name: '改名前' })
      const material = '原始中文剧本。'
      const imported = await call('studio_import_script', { projectId: created.projectId, expectedUpdatedAt: created.updatedAt,
        title: '旧剧名', content: material })
      const layoutBefore = await call('studio_project_settings', { projectId: created.projectId })
      const projectFile = path.join(layoutBefore.folderPath, 'project.json')
      const projectRecord = JSON.parse(await fs.readFile(projectFile, 'utf8'))
      projectRecord.playApproved = true
      await fs.writeFile(projectFile, JSON.stringify(projectRecord))
      const patch = { projectId: created.projectId, expectedUpdatedAt: imported.updatedAt, topic: '新剧名', keywords: ['悬疑'] }
      assert.equal((await call('studio_update_script', patch)).playApproved, true)
      assert.equal((await dpapi.callTool({ name: 'studio_update_script', arguments: patch })).isError, true)
      const script = await call('studio_read_script', { projectId: created.projectId })
      assert.equal(script.topic, '新剧名')
      assert.equal(script.content, material)
      assert.deepEqual(script.keywords, ['悬疑'])
      assert.equal(script.style, 'cinematic')
      assert.equal(script.aspectRatio, '16:9')
      const renamed = await call('studio_rename_project', { projectId: created.projectId, expectedUpdatedAt: script.updatedAt, name: '改名后' })
      assert.equal((await call('studio_project')).project.name, '改名后')
      assert.equal((await call('studio_project_settings', { projectId: created.projectId })).folderPath, layoutBefore.folderPath)
      const replaced = await call('studio_update_script', { projectId: created.projectId, expectedUpdatedAt: renamed.updatedAt, content: '正文第二版' })
      assert.equal(replaced.playApproved, false)
      console.log('PASS tokenless handshake, Windows DPAPI cold start, metadata readback and safe project rename')
    } finally {
      try {
        const current = await call('studio_project')
        const active = current.project
        await call('studio_save_exit', active ? { projectId: active.id, expectedUpdatedAt: active.updatedAt, confirmed: true } : { confirmed: true })
        for (let attempt = 0; attempt < 30 && (await call('studio_web_status')).ready; attempt++) {
          await new Promise(resolve => setTimeout(resolve, 100))
        }
      } finally {
        await dpapi.close()
      }
    }
  }
  console.log('PASS Rust Web hosting, shared MCP and UI project writes, revision conflicts, range media, cold start and exit')
  }
} finally {
  if (coldOrigin) {
    const invoke = (name, input) => fetch(coldOrigin + '/api/dramlo/mcp', {
      method: 'POST', headers: { Authorization: 'Bearer ' + token, 'Content-Type': 'application/json' },
      body: JSON.stringify({ name, input }), signal: AbortSignal.timeout(1500)
    })
    try {
      const response = await invoke('studio_project', {})
      const { project } = response.ok ? await response.json() : {}
      if (project) await invoke('studio_save_exit', { projectId: project.id, expectedUpdatedAt: project.updatedAt, confirmed: true })
    } catch {
      // This test service may already have exited.
    }
  }
  await stop()
  mock.closeAllConnections()
  await new Promise((resolve) => mock.close(resolve))
  if (path.dirname(scratch) === os.tmpdir() && path.basename(scratch).startsWith('dramlo-web-test-')) {
    await fs.rm(scratch, { recursive: true, force: true })
  }
}
