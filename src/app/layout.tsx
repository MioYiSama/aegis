import {
  Outlet,
  NavLink,
  useLocation,
  useNavigate,
  Navigate,
} from "react-router"
import { useState } from "react"
import { List, BookOpen, UserCircle, SignOut } from "@phosphor-icons/react"
import { useAuthMe, authLogout } from "@/api/generated/client"
import { ApiRequestError } from "@/api/http"
import { clearSession } from "./providers"
import { UserContext } from "@/features/auth/session"
import { Button, ErrorState, LoadingRows } from "@/components/common"
import {
  Sheet,
  SheetTrigger,
  SheetContent,
  SheetTitle,
  SheetDescription,
} from "@/components/ui/sheet"
export function ProtectedLayout({ role }: { role: "student" | "teacher" }) {
  const me = useAuthMe()
  const location = useLocation()
  const navigate = useNavigate()
  const [error, setError] = useState<unknown>()
  const [busy, setBusy] = useState(false)
  const [menu, setMenu] = useState(false)
  if (me.isPending)
    return (
      <main className="mx-auto max-w-lg p-6">
        <LoadingRows />
      </main>
    )
  if (me.error) {
    if (me.error instanceof ApiRequestError && me.error.status === 401)
      return <Navigate to="/login" replace />
    return <ErrorState error={me.error} retry={() => me.refetch()} />
  }
  if (!me.data) return null
  if (me.data.role !== role)
    return (
      <main className="p-8">
        <ErrorState error={new ApiRequestError(403, "forbidden", "禁止访问")} />
      </main>
    )
  const capture =
    location.pathname.includes("/attend/") ||
    location.pathname.endsWith("/enroll")
  const projection = location.pathname.includes("/project/")
  const nav = (
    <nav className="grid gap-2">
      <NavLink
        className="flex min-h-11 items-center gap-3 rounded-lg px-3 aria-[current=page]:bg-muted"
        onClick={() => setMenu(false)}
        to={"/" + role + "/courses"}
      >
        <BookOpen size={20} />
        课程
      </NavLink>
      {role === "student" && (
        <NavLink
          className="flex min-h-11 items-center gap-3 rounded-lg px-3 aria-[current=page]:bg-muted"
          to="/student/account"
        >
          <UserCircle size={20} />
          我的
        </NavLink>
      )}
    </nav>
  )
  const logout = async () => {
    setBusy(true)
    try {
      try {
        await authLogout()
      } catch (e) {
        if (!(e instanceof ApiRequestError && e.status === 401)) throw e
      }
      await clearSession()
      navigate("/login", { replace: true })
    } catch (e) {
      setError(e)
    } finally {
      setBusy(false)
    }
  }
  return (
    <UserContext.Provider value={me.data}>
      {role === "teacher" && !projection ? (
        <div className="min-h-dvh md:grid md:grid-cols-[220px_1fr]">
          <aside className="hidden border-r p-6 md:flex md:flex-col">
            <span className="mb-10 text-xl font-semibold">Aegis</span>
            {nav}
            <div className="mt-auto pt-8">
              <p className="mb-3 break-words text-sm text-muted-foreground">
                {me.data.display_name} · 教师
              </p>
              <Button variant="ghost" disabled={busy} onClick={logout}>
                <SignOut size={18} />
                退出登录
              </Button>
              {Boolean(error) && <ErrorState error={error} />}
            </div>
          </aside>
          <div className="min-w-0">
            <div className="flex h-16 items-center justify-between border-b px-5 md:hidden">
              <span className="font-semibold">Aegis</span>
              <Sheet open={menu} onOpenChange={setMenu}>
                <SheetTrigger
                  render={
                    <Button
                      variant="ghost"
                      aria-label="打开导航"
                    />
                  }
                >
                  <List size={24} />
                </SheetTrigger>
                <SheetContent side="left">
                  <SheetTitle>Aegis · 教师</SheetTitle>
                  <SheetDescription>{me.data.display_name}</SheetDescription>
                  {nav}
                  <Button disabled={busy} onClick={logout}>
                    退出登录
                  </Button>
                  {Boolean(error) && <ErrorState error={error} />}
                </SheetContent>
              </Sheet>
            </div>
            <main className="mx-auto max-w-[1280px] p-5 md:p-10">
              <Outlet />
            </main>
          </div>
        </div>
      ) : (
        <div className="min-h-dvh">
          <main
            className={
              projection
                ? "p-6"
                : "mx-auto max-w-[480px] px-5 pt-7 pb-[calc(100px+env(safe-area-inset-bottom))]"
            }
          >
            <Outlet />
          </main>
          {role === "student" && !capture && (
            <nav
              aria-label="主导航"
              className="fixed inset-x-0 bottom-0 border-t bg-white pb-[env(safe-area-inset-bottom)]"
            >
              <div className="mx-auto grid max-w-[480px] grid-cols-2">
                <NavLink
                  to="/student/courses"
                  className="flex min-h-16 flex-col items-center justify-center text-muted-foreground aria-[current=page]:text-foreground"
                >
                  <BookOpen size={22} />
                  课程
                </NavLink>
                <NavLink
                  to="/student/account"
                  className="flex min-h-16 flex-col items-center justify-center text-muted-foreground aria-[current=page]:text-foreground"
                >
                  <UserCircle size={22} />
                  我的
                </NavLink>
              </div>
            </nav>
          )}
        </div>
      )}
    </UserContext.Provider>
  )
}
