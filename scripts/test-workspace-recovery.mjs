import assert from 'node:assert/strict'
import fs from 'node:fs/promises'
import path from 'node:path'
import os from 'node:os'
import { spawn } from 'node:child_process'
import { once } from 'node:events'
import { randomBytes, randomUUID } from 'node:crypto'
import { createServer } from 'node:http'

const binary = process.env.DRAMLO_SERVER_BINARY ??
  path.resolve('dramlo-server/target/debug', process.platform === 'win32' ? 'dramlo-server.exe' : 'dramlo-server')
const frontend = path.resolve('dist')
const temp = await fs.mkdtemp(path.join(os.tmpdir(), 'studio-workspace-test-'))
const token = randomBytes(24).toString('hex')
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a00cAAAAASUVORK5CYII=', 'base64')
const mp4 = Buffer.from('0000ftypisom0000')
const ui = { 'X-Dramlo-UI':'1', 'Content-Type':'application/json' }
const processes = []
let mock
async function start(dir) {
  await fs.mkdir(dir)
  for (let i=0;i<5;i++) {
    const child = spawn(binary,[dir,frontend,'0'],{windowsHide:true,env:{...process.env,DRAMLO_MCP_BRIDGE_TOKEN:token}})
    processes.push(child)
    const [stdout] = await once(child.stdout,'data')
    const origin = stdout.toString().trim()
    try { await (await fetch(origin)).body.cancel(); return origin }
    catch (error) { if (error.cause?.message !== 'bad port') throw error }
  }
  throw new Error('无法分配测试端口')
}
const req = (origin, route, body, headers=ui) =>
  fetch(origin+'/api/dramlo/'+route,{method:'POST',headers,body:JSON.stringify(body)})
const mcp = async (origin,name,input={}) => {
  const response = await req(origin,'mcp',{name,input},{'Content-Type':'application/json',Authorization:'Bearer '+token})
  const body = await response.json()
  assert.ok(response.ok,JSON.stringify(body))
  return body
}
const local = (origin,command,input={}) => req(origin,'fs',{command,...input})
const exists = p => fs.stat(p).then(()=>true,()=>false)
try {
  const origin = await start(path.join(temp,'source'))
  const uiOrigin = await start(path.join(temp,'ui-creation'))
  const uiProjectId = 'ui_fixture'
  const uiFolder = await (await local(uiOrigin,'ensureProjectDir',{name:'UI fixture',id:uiProjectId})).json()
  const uiProjectPath = path.join(uiFolder,'project.json')
  const projectDefaults = JSON.parse(await fs.readFile(new URL('../src/types/projectDefaults.json', import.meta.url), 'utf8'))
  const uiProject = {
    id:uiProjectId, name:'UI fixture', createdAt:1, updatedAt:1, script:null, shots:[], timeline:[],
    settings:projectDefaults, canvas:{nodes:[],edges:[],groups:[],viewport:{x:0,y:0,zoom:1}}
  }
  assert.equal((await local(uiOrigin,'writeText',{path:uiProjectPath,value:JSON.stringify(uiProject)})).status,200)
  const uiInstructions = path.join(uiFolder,'AGENTS.md')
  const defaultInstructions = await fs.readFile(uiInstructions,'utf8')
  assert.match(defaultInstructions,/记录和复用创作经验/)
  assert.match(defaultInstructions,/docs\/创作经验\.md/)
  const customInstructions = defaultInstructions + '\n本项目自定义约定。\n'
  assert.equal((await local(uiOrigin,'writeText',{path:uiInstructions,value:customInstructions})).status,200)
  assert.equal((await local(uiOrigin,'writeText',{
    path:uiProjectPath,value:JSON.stringify({...uiProject,updatedAt:2}),expectedUpdatedAt:1
  })).status,200)
  assert.equal(await fs.readFile(uiInstructions,'utf8'),customInstructions)
  const incompleteDocument = { ...uiProject, canvas:undefined, updatedAt:3 }
  assert.equal((await local(uiOrigin,'writeText',{
    path:uiProjectPath,value:JSON.stringify(incompleteDocument),expectedUpdatedAt:2
  })).status,400)
  assert.equal(JSON.parse(await fs.readFile(uiProjectPath,'utf8')).updatedAt,2)

  const project = await mcp(origin,'studio_create_project',{name:'Portable fixture'})
  const {projectId} = project
  const {project:{projectInstructionsPath}} = await mcp(origin,'studio_project',{projectId})
  const projectDocument = JSON.parse(await fs.readFile(path.join(path.dirname(projectInstructionsPath),'project.json'),'utf8'))
  assert.deepEqual(projectDocument.settings,projectDefaults)
  assert.equal(path.basename(projectInstructionsPath),'AGENTS.md')
  assert.equal(await fs.readFile(projectInstructionsPath,'utf8'),defaultInstructions)
  assert.equal((await local(origin,'writeText',{path:projectInstructionsPath,value:customInstructions})).status,200)
  await mcp(origin,'studio_import_script',{projectId,expectedUpdatedAt:'auto',title:'Portable',content:'独立测试正文'})
  const asset = await mcp(origin,'studio_import_image_asset',{projectId,requestId:randomUUID(),dataBase64:png.toString('base64')})
  const other = await mcp(origin,'studio_import_image_asset',{projectId,requestId:randomUUID(),dataBase64:png.toString('base64')})
  const sceneId = randomUUID()
  const shotId = randomUUID()
  await mcp(origin,'studio_upsert_project_assets',{projectId,expectedUpdatedAt:'auto',items:[{id:sceneId,type:'scene',name:'一场',description:'测试场景'}]})
  const beforeUnknown = (await mcp(origin,'studio_project')).project.updatedAt
  const badUpsert = await req(origin,'mcp',{name:'studio_upsert_project_assets',input:{
    projectId,expectedUpdatedAt:'auto',items:[{id:sceneId,type:'scene',name:'不应保存',description:'测试场景',deleted:true}]
  }},{'Content-Type':'application/json',Authorization:'Bearer '+token})
  assert.equal(badUpsert.status,400)
  assert.match((await badUpsert.json()).error,/deleted/)
  assert.equal((await mcp(origin,'studio_project')).project.updatedAt,beforeUnknown)
  const personId = randomUUID()
  await mcp(origin,'studio_upsert_project_assets',{projectId,expectedUpdatedAt:'auto',items:[{
    id:personId,type:'subject',name:'废弃角色',kind:'person',description:'回归测试'
  }]})
  const subjectRemoval = {projectId,targetType:'subject',assetId:personId}
  const previewSubject = await mcp(origin,'studio_remove_project_asset',subjectRemoval)
  assert.equal(previewSubject.removed,false)
  assert.equal(previewSubject.blocked,false)
  assert.equal((await mcp(origin,'studio_remove_project_asset',{
    ...subjectRemoval,expectedUpdatedAt:previewSubject.updatedAt,confirmed:true
  })).removed,true)
  assert.equal((await mcp(origin,'studio_project_items',{projectId,type:'subjects'})).items.length,0)
  assert.equal((await mcp(origin,'studio_export_status',{projectId})).job,null)
  await mcp(origin,'studio_append_shots',{projectId,expectedUpdatedAt:'auto',sceneId,shots:[{
    id:shotId,title:'一镜',duration:3,visualPrompt:'白色画面',cameraMovement:'static'
  }]})
  await mcp(origin,'studio_attach_image_asset',{projectId,expectedUpdatedAt:'auto',targetType:'shot-image',targetId:shotId,assetId:asset.assetId})
  await mcp(origin,'studio_edit_timeline',{projectId,expectedUpdatedAt:'auto',buildFromShots:true})
  const located = await mcp(origin,'studio_locate_asset',{projectId,assetId:asset.assetId})
  const otherPath = (await mcp(origin,'studio_locate_asset',{projectId,assetId:other.assetId})).path
  const root = path.dirname(path.dirname(located.path))
  const manifest = path.join(root,'manifest.json')
  assert.equal((await local(origin,'removeIndexed',{name:'projectAsset',id:asset.assetId,path:manifest})).status,400)
  assert.equal(await exists(located.path),true)
  const blockedMedia = await mcp(origin,'studio_remove_project_asset',{projectId,assetId:asset.assetId})
  assert.equal(blockedMedia.blocked,true)
  const media = {projectId,assetId:other.assetId}
  const previewMedia = await mcp(origin,'studio_remove_project_asset',media)
  assert.equal(previewMedia.removed,false)
  assert.equal(await exists(otherPath),true)
  assert.equal((await mcp(origin,'studio_remove_project_asset',{
    ...media,expectedUpdatedAt:previewMedia.updatedAt,confirmed:true
  })).removed,true)
  assert.equal(await exists(otherPath),false)
  assert.deepEqual(JSON.parse(await fs.readFile(manifest,'utf8')).map(x=>x.id),[asset.assetId])

  const secret = 'synthetic-recovery-key-test'
  const provider = await mcp(origin,'studio_add_model',{kind:'llm',name:'fixture provider',baseUrl:'https://example.invalid/v1',model:'fixture',adapterType:'openai-compatible',select:true})
  await mcp(origin,'studio_set_credential',{kind:'llm',profileId:provider.profileId,expectedBaseUrl:'https://example.invalid/v1',apiKey:secret})
  const notes = path.join(path.dirname(root),'docs','notes.md')
  assert.equal((await local(origin,'writeText',{path:notes,value:'# 参考资料'})).status,200)
  const backup = await fetch(origin+'/api/dramlo/workspace-backup',{headers:ui})
  assert.equal(backup.status,200)
  const bytes = Buffer.from(await backup.arrayBuffer())
  assert.ok(bytes.includes(Buffer.from('dramlo-backup.json')))
  assert.ok(!bytes.includes(Buffer.from('.dramlo/')))
  const restored = await start(path.join(temp,'destination'))
  const restore = target => fetch(target+'/api/dramlo/workspace-backup',{method:'POST',headers:ui,body:bytes})
  assert.equal((await restore(origin)).status,400)
  const imported = await restore(restored)
  assert.equal(imported.status,200,await imported.clone().text())
  assert.equal((await imported.json()).imported,1)
  await mcp(restored,'studio_open_project',{projectId})
  const {project:{projectInstructionsPath:restoredInstructions}} = await mcp(restored,'studio_project',{projectId})
  assert.equal(await fs.readFile(restoredInstructions,'utf8'),customInstructions)
  assert.equal((await mcp(restored,'studio_read_script',{projectId})).content,'独立测试正文')
  const restoredAsset = await mcp(restored,'studio_locate_asset',{projectId,assetId:asset.assetId})
  assert.ok(restoredAsset.path.startsWith(path.join(temp,'destination')))
  assert.deepEqual(await fs.readFile(restoredAsset.path),png)
  const restoredProject = await fs.readFile(path.join(path.dirname(path.dirname(path.dirname(restoredAsset.path))),'project.json'),'utf8')
  assert.ok(!restoredProject.includes(secret))
  assert.equal(await fs.readFile(path.join(path.dirname(path.dirname(path.dirname(restoredAsset.path))),'docs','notes.md'),'utf8'),'# 参考资料')
  assert.equal((await mcp(restored,'studio_list_models',{kind:'llm'})).profiles.length,0)

  const incompleteFolder = path.join(path.dirname(uiFolder),'project__incomplete')
  await fs.mkdir(incompleteFolder)
  await fs.writeFile(path.join(incompleteFolder,'project.json'),JSON.stringify({ ...incompleteDocument,id:'incomplete' }))
  const incompleteZip = await fetch(uiOrigin+'/api/dramlo/workspace-backup',{headers:ui})
  assert.equal(incompleteZip.status,200)
  const rejected = await fetch(restored+'/api/dramlo/workspace-backup',{
    method:'POST',headers:ui,body:await incompleteZip.arrayBuffer()
  })
  assert.equal(rejected.status,400)
  assert.match((await rejected.json()).error,/当前格式/)
  assert.equal(await exists(path.join(temp,'destination','project__'+uiProjectId)),false)

  const planAsset = await mcp(origin,'studio_import_image_asset',{projectId,requestId:randomUUID(),dataBase64:png.toString('base64')})
  await mcp(origin,'studio_save_plan',{projectId,expectedUpdatedAt:'auto',planId:'protect-plan-refs',name:'计划引用保护',
    steps:[{id:'scene',kind:'image',input:{targetType:'scene',targetId:sceneId,referenceAssetIds:[planAsset.assetId]}}]
  })
  const plannedMedia = await mcp(origin,'studio_remove_project_asset',{projectId,assetId:planAsset.assetId})
  assert.equal(plannedMedia.blocked,true)
  assert.match(plannedMedia.reason,/未结任务/)

  const renumbered = (await mcp(origin,'studio_project')).project.updatedAt
  const removeShot = await req(origin,'workspace-actions',{name:'studio_remove_shot',input:{
    projectId,shotId,confirmed:true,expectedUpdatedAt:renumbered
  }})
  assert.equal((await removeShot.json()).applied,true)
  assert.equal((await mcp(origin,'studio_timeline',{projectId})).items.length,0)
  assert.equal(await exists(located.path),true)
  const rev = (await mcp(origin,'studio_project')).project.updatedAt
  const duplicate = await local(origin,'ensureProjectDir',{name:'Duplicate fixture',id:projectId})
  const duplicateFile = path.join(await duplicate.json(),'project.json')
  await fs.copyFile(path.join(path.dirname(root),'project.json'),duplicateFile)
  const collision = await req(origin,'workspace-actions',{name:'studio_delete_project',input:{projectId}})
  assert.equal(collision.status,400)
  assert.match((await collision.json()).error,/多个目录/)
  await fs.unlink(duplicateFile)
  const deletion = await req(origin,'workspace-actions',{name:'studio_delete_project',input:{projectId,confirmed:true,expectedUpdatedAt:rev}})
  const removed = await deletion.json()
  assert.equal(removed.removed,true)
  assert.equal(await exists(removed.originalPath),false)
  assert.equal(await exists(path.join(removed.recoveryPath,'project.json')),true)
  const projectFile = path.join(removed.originalPath,'project.json')
  const stale = await fs.readFile(path.join(removed.recoveryPath,'project.json'),'utf8')
  const staleWrite = await local(origin,'writeText',{path:projectFile,value:stale,expectedUpdatedAt:rev})
  assert.equal(staleWrite.status,404)
  const staleMedia = await fetch(origin+'/api/dramlo/file?'+new URLSearchParams({path:located.path}),{
    method:'PUT',headers:ui,body:png
  })
  assert.equal(staleMedia.status,404)
  assert.equal(await exists(removed.originalPath),false)
  assert.ok(!(await (await local(origin,'listProjectDirs')).json()).includes(removed.originalPath))

  mock = createServer((request,response)=>{
    if (request.url==='/api/workflow/result') return response.end('{"status":"success"}')
    response.writeHead(200,{'Content-Type':'application/octet-stream'}).end(mp4)
  })
  mock.listen(0,'127.0.0.1')
  await once(mock,'listening')
  const upstream = 'http://127.0.0.1:'+mock.address().port
  assert.equal((await req(origin,'remote-video',{url:upstream+'/video.mp4'})).status,400)
  await mcp(origin,'studio_add_model',{kind:'video',name:'fixture video service',baseUrl:upstream,model:'fixture',adapterType:'http-custom'})
  const video = await req(origin,'remote-video',{url:upstream+'/video.mp4'})
  assert.equal(video.status,200)
  assert.equal(video.headers.get('content-type'),'video/mp4')
  assert.deepEqual(Buffer.from(await video.arrayBuffer()),mp4)
  const poll = await req(origin,'video-provider',{method:'GET',url:upstream+'/api/workflow/result'})
  assert.equal((await poll.json()).ok,true)
  assert.equal((await req(origin,'video-provider',{method:'GET',url:upstream+'/admin'})).status,400)
  console.log('PASS reversible project and asset removal, shot cleanup, portable project backup without credentials, and scoped video retrieval')
} finally {
  if (mock) await new Promise(resolve=>mock.close(resolve))
  for (const child of processes) {
    if (child.exitCode !== null) continue
    const exited = once(child,'exit')
    child.kill()
    await exited
  }
  await fs.rm(temp,{recursive:true,force:true,maxRetries:4}).catch(()=>{})
}
