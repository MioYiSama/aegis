import { mkdir, writeFile } from "node:fs/promises"
const url =
  process.env.AEGIS_OPENAPI_URL ?? "http://127.0.0.1:3000/api/openapi.json"
const response = await fetch(url)
if (!response.ok) throw new Error("OpenAPI HTTP " + response.status)
const document = await response.json()
await mkdir("src/api", { recursive: true })
await writeFile(
  "src/api/openapi.json",
  JSON.stringify(document, null, 2) + "\n",
)
