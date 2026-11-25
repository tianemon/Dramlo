/** Exercise the real upload -> input injection -> submit path with an offline ComfyUI transport. */
import assert from 'node:assert/strict'
import { build } from 'esbuild'
import { resolve, basename } from 'node:path'

const built = await build({
  stdin: { contents: `export { comfyPromptGenerate } from './src/services/ai/adapters';
    export { fitComfyReferenceImages, guessInputMap } from './src/services/ai/comfyInputMap';`, resolveDir: process.cwd() },
  bundle: true, write: false, platform: 'node', format: 'esm', logLevel: 'silent',
  plugins: [{ name: 'offline-comfy', setup(b) {
    b.onResolve({ filter: /.*/ }, args => {
      const name=basename(args.path).replace(/\.ts$/, '')
      if (['desktop','zealman','client'].includes(name)) return { path:name, namespace:'mock' }
      return { path:resolve(args.path.startsWith('@/')?'src':args.resolveDir,args.path.replace(/^@\//,'')+'.ts') }
    })
    b.onLoad({filter:/.*/,namespace:'mock'},args=>({contents:{
      desktop:`export const isDesktop=()=>true; export const desktop=()=>({autodlFetch:req=>globalThis.__comfyTest.request(req)});`,
      zealman:`export const zealmanWorkflowById=()=>undefined;`,
      client:`export const callLLM=()=>{throw Error('Unexpected LLM call in offline test')};`
    }[args.path],loader:'js'}))
  } }]
})
const {comfyPromptGenerate,fitComfyReferenceImages,guessInputMap}=await import('data:text/javascript;base64,'+Buffer.from(built.outputFiles[0].text).toString('base64'))
const image=i=>'data:image/png;base64,'+Buffer.from('test-image-'+i).toString('base64')
let tests=0
function fixture(count=9, h3=false) {
  const prompt={text:{class_type:'CR Prompt Text',inputs:{prompt:'default prompt'}}}
  const ids=Array.from({length:count},(_,i)=>String(100-i)) // not object-key order
  for(const [i,id] of ids.entries()) prompt[id]={class_type:'LoadImage',inputs:{image:`black-${i+1}.png`}}
  prompt.condition={class_type:h3?'MiniMaxH3AudioConditioningT8':'CustomVideoConditioning',inputs:{task_type:'ref2va'}}
  ids.forEach((id,i)=>{prompt.condition.inputs[`ref_images.ref_image_${i}`]=[id,0]})
  const bindings=[{role:'prompt',nodeId:'text',field:'prompt'},...ids.map(id=>({role:'image',nodeId:id,field:'image',mediaMode:'upload'}))]
  return {payload:{prompt},map:{version:1,kind:'image-to-video',summary:'test',source:'heuristic',bindings},ids}
}
async function generate(f, count, extra={}) {
  const uploads=[], submissions=[], logs=[]
  globalThis.__comfyTest={request:async req=>{
    assert.equal(new URL(req.url).origin,'http://127.0.0.1:8188')
    const pathname=new URL(req.url).pathname
    const response=(data,status=200)=>({ok:status===200,status,text:JSON.stringify(data)})
    if(pathname==='/upload/image') {
      uploads.push(Buffer.from(req.bodyBase64,'base64').toString())
      if(extra.failUpload) return response({error:'upload failed'},500)
      return response({name:`uploaded-${uploads.length}.png`,subfolder:'inputs'})
    }
    if(pathname==='/prompt') { submissions.push(JSON.parse(req.body));return response({prompt_id:'test-job'}) }
    if(pathname==='/history/test-job') return response({'test-job':{outputs:{video:{videos:[{filename:'test.mp4',type:'output'}]}}}})
    if(pathname==='/view') return {ok:true,status:200,text:'',contentType:'video/mp4',dataBase64:Buffer.from('test-video').toString('base64')}
    throw Error('Unexpected request: '+req.url)
  }}
  const template=JSON.stringify(f.payload)
  const opts={baseUrl:'http://127.0.0.1:8188',kind:'video',prompt:'本次提示词',
    imageDataUrl:count?image(1):undefined, referenceImageDataUrls:Array.from({length:Math.max(0,count-1)},(_,i)=>image(i+2)),
    adapter:{workflowTemplate:template,inputMap:f.map},onProgress:m=>logs.push(m),...extra}
  delete opts.failUpload
  try { await comfyPromptGenerate(opts) }
  catch(error) { return {error,uploads,submissions,logs} }
  assert.equal(JSON.stringify(f.payload),template,'stored workflow must stay unchanged')
  return {uploads,submissions,logs}
}
async function test(name,fn) { await fn(); tests++;console.log('PASS',name) }

await test('Web 画布与队列一致，在上传前拦截未绑定的固定图',async()=>{
  const blocked=await generate(fixture(2),1)
  assert.match(String(blocked.error),/black-2\.png/)
  assert.equal(blocked.uploads.length,0)
  assert.equal(blocked.submissions.length,0)
  const complete=await generate(fixture(2),2)
  assert.ifError(complete.error)
  assert.equal(complete.submissions.length,1)
})

for (const count of [1, 6, 9]) await test('映射 ' + count + ' 张参考图时按槽位顺序上传', async () => {
  const f = fixture(count)
  const result = await generate(f, count)
  assert.ifError(result.error)
  assert.equal(result.uploads.length, count)
  assert.equal(result.submissions.length, 1)
  const nodes = result.submissions[0].prompt
  f.ids.forEach((id, index) => assert.equal(nodes[id].inputs.image, 'inputs/uploaded-' + (index + 1) + '.png'))
  assert.deepEqual(nodes.condition, f.payload.prompt.condition)
  assert.equal(nodes.text.inputs.prompt, '本次提示词')
  result.uploads.forEach((body, index) => assert.ok(body.includes('test-image-' + (index + 1))))
})
await test('H3 缺少映射时，按实际连线补齐并保留节点', async () => {
  const f = fixture(9, true)
  f.map.bindings = f.map.bindings.slice(0, 2)
  const result = await generate(f, 9)
  assert.ifError(result.error)
  const nodes = result.submissions[0].prompt
  assert.equal(Object.keys(nodes).length, Object.keys(f.payload.prompt).length)
  assert.deepEqual(nodes.condition, f.payload.prompt.condition)
  f.ids.forEach((id, index) => assert.equal(nodes[id].inputs.image, 'inputs/uploaded-' + (index + 1) + '.png'))
})
await test('参考图槽位经缩放节点回溯到实际输入图，保留原有连线',async()=>{
  const f=fixture(1)
  const graph=f.payload.prompt
  graph.resize={class_type:'ResizeImage',inputs:{image:[f.ids[0],0]}}
  graph.condition.inputs['ref_images.ref_image_0']=['resize',0]
  f.map=guessInputMap(f.payload)
  const r=await generate(f,1)
  assert.ifError(r.error)
  assert.deepEqual(r.submissions[0].prompt.condition.inputs['ref_images.ref_image_0'],['resize',0])
  assert.equal(r.submissions[0].prompt[f.ids[0]].inputs.image,'inputs/uploaded-1.png')
})
await test('未连接到条件节点的固定图片保持原样', async () => {
  const f = fixture(1)
  f.payload.prompt.fixed = { class_type: 'LoadImage', inputs: { image: 'fixed-black.png' } }
  const result = await generate(f, 1)
  assert.ifError(result.error)
  assert.equal(result.uploads.length, 1)
  assert.equal(result.submissions[0].prompt.fixed.inputs.image, 'fixed-black.png')
})
await test('尾帧/蒙版单独替换，不挤占参考图槽位',async()=>{
  const f=fixture(1)
  for(const role of ['imageEnd','mask']) {
    f.payload.prompt[role]={class_type:'LoadImage',inputs:{image:'old.png'}}
    f.map.bindings.splice(1,0,{role,nodeId:role,field:'image',mediaMode:'upload'})
  }
  const r=await generate(f,1,{imageEndDataUrl:image(99),maskDataUrl:image(98)})
  assert.ifError(r.error);assert.equal(r.uploads.length,3)
  assert.equal(r.submissions[0].prompt[f.ids[0]].inputs.image,'inputs/uploaded-3.png')
  assert.equal(r.submissions[0].prompt.imageEnd.inputs.image,'inputs/uploaded-2.png')
})
await test('支持已知H3结构从4槽扩容到6槽',()=>{
  const f=fixture(4,true), map=fitComfyReferenceImages(f.payload,f.map,6)
  assert.equal(map.bindings.filter(b=>b.role==='image').length,6)
  assert.equal(Object.keys(f.payload.prompt.condition.inputs).filter(k=>k.startsWith('ref_images.')).length,6)
})
await test('超出不支持扩容的工作流容量仍明确提示，不能丢图',async()=>{
  const r=await generate(fixture(3),4)
  assert.match(r.error?.message,/只有 3 个可用参考图槽位/)
  assert.equal(r.uploads.length,0);assert.equal(r.submissions.length,0)
})
await test('实际素材上传失败时不能使用默认图冒充成功',async()=>{
  const r=await generate(fixture(1),1,{failUpload:true})
  assert.match(r.error?.message,/上传素材失败/);assert.equal(r.submissions.length,0)
})
await test('重复映射仍阻止提交，避免前后两张覆盖同一个槽位',async()=>{
  const f=fixture();f.map.bindings.push({...f.map.bindings[1]})
  const r=await generate(f,1)
  assert.match(r.error?.message,/同一个节点字段/);assert.equal(r.submissions.length,0)
})
await test('图像工作流也在上传前拦截未绑定的固定图', async () => {
  const result = await generate(fixture(2), 1, { kind: 'image' })
  assert.match(result.error?.message, /black-2\.png/)
  assert.equal(result.uploads.length, 0)
  assert.equal(result.submissions.length, 0)
})
await test('连续拍保存任务ID；恢复时只查询，不上传或重复提交',async()=>{
  let saved=''
  const first=await generate(fixture(1),1,{onTaskSubmitted:id=>{saved=id}})
  assert.ifError(first.error);assert.equal(saved,'test-job')
  const resumed=await generate(fixture(1),1,{resumeTaskId:saved,onTaskSubmitted:()=>{throw Error('must not resubmit')}})
  assert.ifError(resumed.error);assert.equal(resumed.uploads.length,0);assert.equal(resumed.submissions.length,0)
})
console.log(`${tests} ComfyUI reference-slot regressions passed (offline; no network or generation charges).`)
