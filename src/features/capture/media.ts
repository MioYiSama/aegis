const MAX_IMAGE_BYTES = 2 * 1024 * 1024
const MAX_EDGE = 1920

function ensureActive(signal: AbortSignal) {
  if (signal.aborted) throw new DOMException("采集已取消", "AbortError")
}

export function delay(
  milliseconds: number,
  signal: AbortSignal,
): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(new DOMException("采集已取消", "AbortError"))
      return
    }

    const timer = window.setTimeout(() => {
      signal.removeEventListener("abort", cancel)
      resolve()
    }, milliseconds)
    const cancel = () => {
      window.clearTimeout(timer)
      signal.removeEventListener("abort", cancel)
      reject(new DOMException("采集已取消", "AbortError"))
    }
    signal.addEventListener("abort", cancel, { once: true })
  })
}

function captureCanvas(video: HTMLVideoElement) {
  const sourceWidth = video.videoWidth
  const sourceHeight = video.videoHeight
  if (sourceWidth < 1 || sourceHeight < 1)
    throw new Error("相机尚未提供可用画面，请重试")

  // Match the centered 9:16 object-cover preview, including landscape sources.
  const cropWidth = Math.min(sourceWidth, sourceHeight * 9 / 16)
  const cropHeight = cropWidth * 16 / 9
  const scale = Math.min(1, MAX_EDGE / cropWidth, MAX_EDGE / cropHeight)
  const canvas = document.createElement("canvas")
  canvas.width = Math.max(1, Math.round(cropWidth * scale))
  canvas.height = Math.max(1, Math.round(cropHeight * scale))
  const context = canvas.getContext("2d")
  if (!context) throw new Error("无法读取相机画面")
  context.drawImage(
    video,
    (sourceWidth - cropWidth) / 2,
    (sourceHeight - cropHeight) / 2,
    cropWidth,
    cropHeight,
    0,
    0,
    canvas.width,
    canvas.height,
  )
  return canvas
}
export function readVideoFrame(video: HTMLVideoElement): ImageData {
  const canvas = captureCanvas(video)
  const context = canvas.getContext("2d")
  if (!context) throw new Error("无法读取相机画面")
  return context.getImageData(0, 0, canvas.width, canvas.height)
}

function canvasBlob(
  canvas: HTMLCanvasElement,
  type: string,
  quality?: number,
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob) resolve(blob)
        else reject(new Error("无法处理相机画面，请重试"))
      },
      type,
      quality,
    )
  })
}

async function captureJpeg(
  video: HTMLVideoElement,
  signal: AbortSignal,
): Promise<Blob> {
  ensureActive(signal)
  const blob = await canvasBlob(captureCanvas(video), "image/jpeg", 0.9)
  ensureActive(signal)
  if (blob.size > MAX_IMAGE_BYTES) {
    throw new Error("单张人脸照片超过 2 MiB，请调整取景后重试")
  }
  return blob
}

export async function captureFaceFrames(
  video: HTMLVideoElement,
  signal: AbortSignal,
  onFrame: (count: number) => void,
  onCountdown: (seconds: number) => void,
): Promise<[Blob, Blob, Blob]> {
  for (let seconds = 3; seconds > 0; seconds -= 1) {
    ensureActive(signal)
    onCountdown(seconds)
    await delay(1000, signal)
  }
  onCountdown(0)
  const frames: Blob[] = []
  for (let index = 0; index < 3; index += 1) {
    if (index > 0) await delay(1500, signal)
    frames.push(await captureJpeg(video, signal))
    onFrame(frames.length)
  }
  return [frames[0], frames[1], frames[2]]
}

export async function imageDataToPng(
  image: ImageData,
  signal: AbortSignal,
): Promise<Blob> {
  ensureActive(signal)
  const canvas = document.createElement("canvas")
  canvas.width = image.width
  canvas.height = image.height
  const context = canvas.getContext("2d")
  if (!context) throw new Error("无法生成二维码图像")
  context.putImageData(image, 0, 0)
  const blob = await canvasBlob(canvas, "image/png")
  ensureActive(signal)
  if (blob.size > MAX_IMAGE_BYTES) {
    throw new Error("二维码图像超过 2 MiB，请调整取景后重新开始扫码")
  }
  return blob
}

export function validateUploadSize(blobs: Blob[]) {
  const total = blobs.reduce((sum, blob) => sum + blob.size, 0)
  if (total > 12 * 1024 * 1024)
    throw new Error("采集文件总量超过 12 MiB，请重新开始")
}

export function cameraErrorMessage(error: unknown) {
  if (error instanceof DOMException) {
    if (error.name === "NotAllowedError" || error.name === "SecurityError")
      return "相机权限被拒绝，请在浏览器设置中允许相机后重新开始"
    if (error.name === "NotFoundError" || error.name === "OverconstrainedError")
      return "未找到可用相机，请检查设备后重新开始"
    if (error.name === "NotReadableError" || error.name === "AbortError")
      return "相机正在被其他应用使用或无法读取，请关闭其他应用后重试"
  }
  if (error instanceof Error) return error.message
  return "相机启动失败，请重新开始"
}
