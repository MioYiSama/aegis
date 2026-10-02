import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { useEffect, type ReactNode } from "react"
import { onUnauthorized } from "@/api/http"
import { stopScanner } from "@/wasm/client"
export const queryClient = new QueryClient({
  defaultOptions: {
    queries: { retry: false, staleTime: 30_000, refetchOnWindowFocus: true },
    mutations: { retry: false },
  },
})
const cleanups = new Set<() => void>()
export function registerSessionCleanup(cleanup: () => void) {
  cleanups.add(cleanup)
  return () => {
    cleanups.delete(cleanup)
  }
}
export async function clearSession() {
  await queryClient.cancelQueries()
  queryClient.clear()
  stopScanner()
  for (const cleanup of cleanups) cleanup()
  try {
    for (const key of Object.keys(sessionStorage))
      if (key.startsWith("aegis:reviewable:")) sessionStorage.removeItem(key)
  } catch {}
}
export const liveQuery = {
  refetchInterval: 5_000,
  refetchIntervalInBackground: false,
}
export function Providers({ children }: { children: ReactNode }) {
  useEffect(
    () =>
      onUnauthorized(async () => {
        await clearSession()
        if (location.pathname != "/login" && location.pathname != "/register")
          location.replace("/login")
      }),
    [],
  )
  return (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  )
}
