/** Compatibility for OpenAI Chat Completions and legacy compatible providers. */
export function prepareChatPayload(input: Record<string, unknown>): Record<string, unknown> {
  const body = { ...input }
  const model = String(body.model || '').split('/').pop() || ''
  if (/^(?:o[134](?:-|$)|gpt-[56](?:[.-]|$))/i.test(model)) {
    if (body.max_tokens !== undefined) {
      body.max_completion_tokens ??= body.max_tokens
      delete body.max_tokens
    }
    // These families may reject non-default sampling settings.
    delete body.temperature
  }
  return body
}

export async function compatibleChatRequest<T extends { ok: boolean; status: number; errText: string }>(
  input: Record<string, unknown>, send: (body: Record<string, unknown>) => Promise<T>
): Promise<T> {
  const body = prepareChatPayload(input)
  // At most one token-name correction and one sampling correction. Never retry
  // successful generation or authentication/rate-limit/network errors here.
  const changed = new Set<string>()
  for (;;) {
    const result = await send({ ...body })
    if (result.ok || result.status !== 400) return result
    const error = result.errText
    const unsupported = /unsupported|not supported|not allowed|unknown|unrecognized|unexpected/i.test(error)
    if (!unsupported) return result
    const from = body.max_tokens !== undefined ? 'max_tokens' : 'max_completion_tokens'
    const to = from === 'max_tokens' ? 'max_completion_tokens' : 'max_tokens'
    if (!changed.has('tokens') && body[from] !== undefined && error.includes(from) && error.includes(to)) {
      body[to] = body[from]
      delete body[from]
      changed.add('tokens')
    } else if (!changed.has('temperature') && body.temperature !== undefined && /temperature/i.test(error)) {
      delete body.temperature
      changed.add('temperature')
    } else return result
  }
}
