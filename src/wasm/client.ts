let session:
  | {
      worker: Worker
      signal: AbortSignal
      busy: boolean
      nextAt: number
      cancel: () => void
    }
  | undefined
export function stopScanner() {
  session?.cancel()
}
export async function scanChromaFrame(
  image: ImageData,
  signal: AbortSignal,
): Promise<ImageData | null> {
  if (signal.aborted) throw new DOMException("扫码已取消", "AbortError")
  if (session && session.signal !== signal) stopScanner()
  if (!session) {
    const worker = new Worker(new URL("./qr.worker.ts", import.meta.url), {
      type: "module",
    })
    const cancel = () => {
      worker.dispatchEvent(new ErrorEvent("error", { message: "扫码已取消" }))
      worker.terminate()
      signal.removeEventListener("abort", cancel)
      if (session?.worker === worker) session = undefined
    }
    session = { worker, signal, busy: false, nextAt: 0, cancel }
    signal.addEventListener("abort", cancel, { once: true })
  }
  const current = session
  if (current.busy) throw new Error("已有二维码解码任务")
  if (performance.now() < current.nextAt) return null
  current.busy = true
  try {
    return await new Promise<ImageData | null>((resolve, reject) => {
      const cleanup = () => {
        current.worker.removeEventListener("message", message)
        current.worker.removeEventListener("error", error)
      }
      const message = (event: MessageEvent) => {
        cleanup()
        if (event.data.error) {
          reject(new Error(event.data.error))
          return
        }
        resolve(
          event.data.readable
            ? new ImageData(
                new Uint8ClampedArray(event.data.buffer),
                event.data.width,
                event.data.height,
              )
            : null,
        )
      }
      const error = (event: ErrorEvent) => {
        cleanup()
        reject(
          signal.aborted
            ? new DOMException("扫码已取消", "AbortError")
            : new Error(event.message || "WASM 初始化失败"),
        )
      }
      current.worker.addEventListener("message", message)
      current.worker.addEventListener("error", error)
      const buffer = image.data.buffer as ArrayBuffer
      current.worker.postMessage(
        { buffer, width: image.width, height: image.height },
        [buffer],
      )
    })
  } finally {
    current.busy = false
    current.nextAt = performance.now() + 250
  }
}
