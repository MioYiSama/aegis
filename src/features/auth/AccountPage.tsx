import { useState } from "react"
import { useNavigate } from "react-router"
import { authLogout } from "@/api/generated/client"
import { ApiRequestError } from "@/api/http"
import { clearSession } from "@/app/providers"
import { useMe } from "./session"
import { Button, PageHeader, Link, ErrorState } from "@/components/common"
export function AccountPage() {
  const me = useMe()
  const navigate = useNavigate()
  const [error, setError] = useState<unknown>()
  const [busy, setBusy] = useState(false)
  return (
    <>
      <PageHeader title="我的" />
      <section className="divide-y border-y">
        <div className="py-5">
          <h2>{me.display_name}</h2>
          <p className="text-muted-foreground">学号 {me.student_no}</p>
          <p className="text-sm text-muted-foreground">{me.username}</p>
        </div>
        <div className="py-5">
          <Link
            to="/student/enroll"
            className="flex min-h-11 items-center justify-between font-medium"
          >
            人脸登记 <span aria-hidden>→</span>
          </Link>
          <p className="text-sm text-muted-foreground">
            首次登记后不能自助替换；登记状态以服务器回执为准。
          </p>
        </div>
      </section>
      {Boolean(error) && <ErrorState error={error} />}
      <Button
        variant="outline"
        className="mt-8 w-full"
        disabled={busy}
        onClick={async () => {
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
        }}
      >
        退出登录
      </Button>
    </>
  )
}
