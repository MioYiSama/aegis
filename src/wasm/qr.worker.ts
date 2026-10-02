import init, { decode_chroma_rgba } from "./generated/aegis_wasm"
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
    const readable =
      decode_chroma_rgba(new Uint8Array(buffer), width, height) !== undefined
    self.postMessage(
      { buffer, width, height, readable },
      { transfer: [buffer] },
    )
  } catch (error) {
    self.postMessage({
      error: error instanceof Error ? error.message : "二维码解码失败",
    })
  }
}
