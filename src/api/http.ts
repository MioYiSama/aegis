export class ApiRequestError extends Error {
  constructor(
    public status: number,
    public code: string,
    message: string,
  ) {
    super(message)
    this.name = "ApiRequestError"
  }
}
let unauthorized: (() => Promise<void>) | undefined
export function onUnauthorized(handler: () => Promise<void>) {
  unauthorized = handler
  return () => {
    unauthorized = undefined
  }
}
async function checkedResponse(url: string, options: RequestInit = {}) {
  const headers = new Headers(options.headers)
  if (options.body instanceof FormData) headers.delete("Content-Type")
  const response = await fetch(url, {
    ...options,
    headers,
    credentials: "same-origin",
  })
  if (!response.ok) {
    const body = await response.json().catch(() => null)
    if (
      response.status === 401 &&
      !["/api/auth/login", "/api/auth/register", "/api/auth/logout"].includes(
        url,
      )
    )
      await unauthorized?.()
    throw new ApiRequestError(
      response.status,
      body?.code ?? "http_error",
      body?.message ?? "请求失败",
    )
  }
  return response
}
export async function apiRequest<T>(
  url: string,
  options?: RequestInit,
): Promise<T> {
  const response = await checkedResponse(url, options)
  if (response.status === 204) return undefined as T
  return (
    response.headers.get("Content-Type")?.includes("json")
      ? response.json()
      : response.blob()
  ) as Promise<T>
}
export async function requestBinary(
  url: string,
  signal?: AbortSignal,
): Promise<{ blob: Blob; headers: Headers }> {
  const response = await checkedResponse(url, { signal, cache: "no-store" })
  return { blob: await response.blob(), headers: response.headers }
}
