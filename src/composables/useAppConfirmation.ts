import { shallowRef } from 'vue'

const pending = shallowRef<{ message: string; resolve: (accepted: boolean) => void }>()

export function confirmAction(message: string): Promise<boolean> {
  if (pending.value) return Promise.resolve(false)
  return new Promise(resolve => {
    pending.value = { message, resolve }
  })
}

export function useAppConfirmation() {
  function finish(accepted: boolean) {
    const request = pending.value
    pending.value = undefined
    request?.resolve(accepted)
  }

  return { pending, finish }
}
