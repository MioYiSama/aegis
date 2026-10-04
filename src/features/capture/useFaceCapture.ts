import { useRef, useState } from "react"
import { captureFaceFrames } from "./media"

export function useFaceCapture() {
  const [aligning, setAligning] = useState(false)
  const [countdown, setCountdown] = useState(0)
  const [frames, setFrames] = useState(0)
  const confirmRef = useRef<(() => void) | null>(null)

  function reset() {
    setAligning(false)
    setCountdown(0)
    setFrames(0)
  }

  function prepare(signal: AbortSignal): Promise<void> {
    reset()
    // Keep the existing ES2022 browser baseline; withResolvers needs newer runtimes.
    return new Promise<void>((resolve, reject) => {
      const cleanup = () => {
        signal.removeEventListener("abort", cancel)
        confirmRef.current = null
        setAligning(false)
      }
      const cancel = () => {
        cleanup()
        reject(new DOMException("采集已取消", "AbortError"))
      }
      confirmRef.current = () => {
        cleanup()
        resolve()
      }
      signal.addEventListener("abort", cancel, { once: true })
      if (signal.aborted) cancel()
      else setAligning(true)
    })
  }

  async function capture(video: HTMLVideoElement, signal: AbortSignal) {
    return await captureFaceFrames(video, signal, setFrames, setCountdown)
  }

  return {
    aligning,
    countdown,
    frames,
    reset,
    prepare,
    capture,
    confirm: () => confirmRef.current?.(),
  }
}
