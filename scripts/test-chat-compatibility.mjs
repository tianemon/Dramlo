import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import ts from 'typescript'
const source = await readFile(new URL('../src/services/ai/chatCompatibility.ts', import.meta.url), 'utf8')
const { outputText } = ts.transpileModule(source, { compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext } })
const { prepareChatPayload, compatibleChatRequest } = await import('data:text/javascript;base64,' + Buffer.from(outputText).toString('base64'))
for (const model of ['gpt-6', 'gpt-6.1', 'gpt-6.2', 'openai/gpt-6.2', 'gpt-5', 'gpt-5.2', 'gpt-5-mini', 'o1', 'o3-mini', 'o4-mini', 'openai/gpt-5']) {
  const input = { model, max_tokens: 4000, temperature: 0.7 }
  const out = prepareChatPayload(input)
  assert.equal(out.max_completion_tokens, 4000)
  assert.equal(out.max_tokens, undefined)
  assert.equal(out.temperature, undefined)
  assert.equal(input.max_tokens, 4000)
}
for (const model of ['gpt-4o', 'gpt-4.1', 'gpt-3.5-turbo', 'deepseek-chat', 'qwen3', 'local-model']) {
  const input = { model, max_tokens: 8192, temperature: 0.7 }
  assert.deepEqual(prepareChatPayload(input), input)
}
assert.equal(prepareChatPayload({ model: 'gpt-5' }).max_completion_tokens, undefined)
assert.equal(prepareChatPayload({ model: 'gpt-5', max_tokens: 10, max_completion_tokens: 20 }).max_completion_tokens, 20)
const success = { ok: true, status: 200, errText: '' }
let sent = []
await compatibleChatRequest({ model: 'alias', max_tokens: 6000, temperature: 0 }, async body => {
  sent.push(body)
  if(sent.length === 1) return { ok: false, status: 400, errText: "Unsupported parameter: 'max_tokens'. Use 'max_completion_tokens' instead." }
  if(sent.length === 2) return { ok: false, status: 400, errText: "Unsupported value: 'temperature' does not support 0" }
  return success
})
assert.equal(sent.length, 3)
assert.equal(sent[1].max_completion_tokens, 6000)
assert.equal(sent[1].max_tokens, undefined)
assert.equal(sent[2].temperature, undefined)
sent = []
await compatibleChatRequest({ model: 'gpt-5', max_tokens: 12000 }, async body => {
  sent.push(body)
  return sent.length === 1 ? { ok:false, status:400, errText:'Unsupported max_completion_tokens; use max_tokens instead' } : success
})
assert.equal(sent[1].max_tokens, 12000)
assert.equal(sent[1].max_completion_tokens, undefined)
for(const status of [401, 403, 429, 500]) {
  let count=0
  await compatibleChatRequest({max_tokens:4},async()=>{count++;return {ok:false,status,errText:'Unsupported max_tokens; use max_completion_tokens'}})
  assert.equal(count,1)
}
let count=0
await compatibleChatRequest({max_tokens:4},async()=>{count++;return {ok:false,status:400,errText:'Unsupported max_tokens and max_completion_tokens'}})
assert.equal(count,2)
count=0
await compatibleChatRequest({max_tokens:4},async()=>{count++;return {ok:false,status:400,errText:'max_tokens must be positive'}})
assert.equal(count,1)
count=0
await compatibleChatRequest({max_tokens:4},async()=>{count++;return success})
assert.equal(count,1)
// All three request implementations must retain the shared compatibility layer.
for(const path of ['../src/services/ai/client.ts','../src/services/ai/adapters.ts','../src/services/canvas/aiSdkAgent.ts']) {
  assert.match(await readFile(new URL(path,import.meta.url),'utf8'),/await compatibleChatRequest\(/)
}
console.log('PASS chat compatibility: new/legacy models, aliases, limits, bounded retries, errors, all request paths (offline)')

// Execute the real callLLM pipeline using mock transports, including JSON repair.
const { build } = await import('esbuild')
const { resolve } = await import('node:path')
const bundled = await build({
  entryPoints: ['src/services/ai/client.ts'], bundle:true, write:false, format:'esm', platform:'node',
  alias:{'@':resolve('src')}, plugins:[{name:'offline-client',setup(b){
    b.onResolve({filter:/.*/},args=>{
      if(args.path==='@/services/desktop')return {path:'desktop',namespace:'mock'}
      if(args.path==='./runtime')return {path:'runtime',namespace:'mock'}
      if(args.path==='./mock')return {path:'mock',namespace:'mock'}
      if(args.path==='@/utils/jsonRepair')return {path:'json',namespace:'mock'}
    })
    b.onLoad({filter:/.*/,namespace:'mock'},args=>({contents: {
      desktop:'export const isDesktop=()=>globalThis.chatTest.desktop; export const desktop=()=>({llmFetch:globalThis.chatTest.send});',
      runtime:'export const getAIMode=()=>"real"; export const getLlmConfig=()=>({baseUrl:"https://offline.invalid/v1",apiKey:"mock-test-credential",model:globalThis.chatTest.model}); export const getVisionConfig=getLlmConfig;',
      mock:'export const mockLLM=()=>{throw Error("unexpected mock mode")};',
      json:'export const parseJsonRobust=JSON.parse;'
    }[args.path],loader:'js'}))
  }}]
})
const {callLLM}=await import('data:text/javascript;base64,'+Buffer.from(bundled.outputFiles[0].text).toString('base64'))
const originalFetch=globalThis.fetch
try {
 for(const desktop of [true,false]) {
  for(const model of ['gpt-6.2','gpt-5','deepseek-chat']) {
   const requests=[]
   globalThis.chatTest={desktop,model,send:async req=>{
    requests.push(JSON.parse(req.body))
    return {ok:true,status:200,text:JSON.stringify({choices:[{message:{content:requests.length===1?'not JSON':'{"ok":true}'}}]})}
   }}
   globalThis.fetch=async (_url,init)=>{const r=await globalThis.chatTest.send(init);return new Response(r.text,{status:r.status})}
   const response=await callLLM({messages:[{role:'user',content:'offline'}],jsonMode:true,maxTokens:4000})
   assert.equal(response.content,'{"ok":true}')
   assert.equal(requests.length,2)
   const key=model==='deepseek-chat'?'max_tokens':'max_completion_tokens'
   assert.equal(requests[0][key],4000)
   assert.equal(requests[1][key],6000)
   assert.equal(requests[1][key==='max_tokens'?'max_completion_tokens':'max_tokens'],undefined)
   requests.length=0
   await callLLM({messages:[{role:'user',content:[{type:'image_url',image_url:{url:'data:image/png;base64,AA=='}}]}],provider:'vision',useProviderOutputLimit:true})
   assert.equal(requests[0].max_tokens,undefined)
   assert.equal(requests[0].max_completion_tokens,undefined)
  }
 }
} finally {globalThis.fetch=originalFetch;delete globalThis.chatTest}
console.log('PASS real LLM browser/desktop transport, JSON repair budgets, vision/provider output limits (offline)')
