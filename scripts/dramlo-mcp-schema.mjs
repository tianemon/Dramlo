import assert from 'node:assert/strict'
import fs from 'node:fs'
import { z } from 'zod'
import { studioTools } from '../src/services/studioMcpTools.ts'

const file = new URL('../dramlo-server/mcp-tools.json', import.meta.url)
const tools = Object.entries(studioTools).map(([name, { description, schema }]) => ({
  name, description, inputSchema: z.toJSONSchema(schema, { io: 'input' })
}))
function checkFields(schema, path) {
  for (const [name, field] of Object.entries(schema.properties ?? {})) {
    assert.ok(field.description?.trim(), 'Missing MCP field description: ' + path + '.' + name)
    checkFields(field, path + '.' + name)
  }
  if (schema.items) checkFields(schema.items, path + '[]')
  for (const choice of [...(schema.anyOf ?? []), ...(schema.oneOf ?? []), ...(schema.allOf ?? [])]) {
    checkFields(choice, path)
  }
}
for (const tool of tools) checkFields(tool.inputSchema, tool.name)

const output = JSON.stringify(tools, null, 2) + '\n'
if (process.argv.includes('--check')) {
  assert.equal(fs.readFileSync(file, 'utf8').replaceAll('\r\n', '\n'), output, 'Run npm run mcp:schema to update the Rust MCP schemas')
} else {
  fs.writeFileSync(file, output)
}
