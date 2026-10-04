import init, { scan_chroma_rgba } from "./generated/aegis_wasm"
const ready = init({
  module_or_path: new URL("./generated/aegis_wasm_bg.wasm", import.meta.url),
}).then(
  () => true,
  () => false,
)
self.onmessage = async (
  event: MessageEvent<{ buffer: ArrayBuffer; width: number; height: number }>,
) => {
  const { buffer, width, height } = event.data
  try {
    if (!(await ready)) throw new Error("WASM 初始化失败，请重新开始扫码")
    const rgba = new Uint8Array(buffer)
    const bounds = scan_chroma_rgba(rgba, width, height)
    if (!bounds) {
      self.postMessage(
        { buffer, width, height, readable: false },
        { transfer: [buffer] },
      )
      return
    }
    const [left, top, cropWidth, cropHeight] = bounds
    const cropped = new Uint8Array(cropWidth * cropHeight * 4)
    for (let row = 0; row < cropHeight; row += 1) {
      const start = ((top + row) * width + left) * 4
      cropped.set(rgba.subarray(start, start + cropWidth * 4), row * cropWidth * 4)
    }
    self.postMessage(
      { buffer: cropped.buffer, width: cropWidth, height: cropHeight, readable: true },
      { transfer: [cropped.buffer] },
    )
  } catch (error) {
    self.postMessage({
      error: error instanceof Error ? error.message : "二维码解码失败",
    })
  }
}
