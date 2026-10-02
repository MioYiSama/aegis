import { createContext, useContext } from "react"
import type { User } from "@/api/generated/models"
export const UserContext = createContext<User | null>(null)
export function useMe() {
  const user = useContext(UserContext)
  if (!user) throw new Error("缺少会话")
  return user
}
