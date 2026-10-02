import { useState } from "react"
import { useMutation } from "@tanstack/react-query"
import { useNavigate } from "react-router"
import {
  authRegister,
  authLogin,
  authMe,
  getAuthMeQueryKey,
} from "@/api/generated/client"
import { clearSession, queryClient } from "@/app/providers"
import {
  Button,
  Input,
  Link,
  PageHeader,
  ErrorState,
} from "@/components/common"
import {
  Select,
  SelectTrigger,
  SelectValue,
  SelectContent,
  SelectItem,
} from "@/components/ui/select"
export function AuthPage({ register = false }: { register?: boolean }) {
  const navigate = useNavigate()
  const [role, setRole] = useState<"student" | "teacher">("student")
  const [registered, setRegistered] = useState(false)
  const action = useMutation({
    mutationFn: async (form: FormData) => {
      const username = String(form.get("username")).trim()
      const password = String(form.get("password"))
      await clearSession()
      if (register && !registered) {
        await authRegister({
          username,
          password,
          display_name: String(form.get("display_name")).trim(),
          role,
          ...(role === "student"
            ? { student_no: String(form.get("student_no")).trim() }
            : {}),
        })
        setRegistered(true)
      }
      await authLogin({ username, password })
      const user = await authMe()
      queryClient.setQueryData(getAuthMeQueryKey(), user)
      navigate("/" + user.role + "/courses", { replace: true })
    },
  })
  return (
    <main className="mx-auto flex min-h-dvh max-w-[440px] flex-col justify-center px-6 py-12">
      <Link to="/login" className="mb-12 text-xl font-semibold tracking-tight">
        Aegis
        <span className="ml-3 text-sm font-normal text-muted-foreground">
          课程考勤
        </span>
      </Link>
      <PageHeader
        title={register ? "创建账号" : "欢迎回来"}
        description={
          register
            ? "教师负责核对学生名单与登记身份。"
            : "登录后查看你的课程与考勤。"
        }
      />
      <form
        className="grid gap-5"
        onSubmit={(event) => {
          event.preventDefault()
          action.mutate(new FormData(event.currentTarget))
        }}
      >
        <label className="grid gap-2">
          用户名
          <Input
            name="username"
            autoComplete="username"
            required
            minLength={3}
            maxLength={64}
            pattern="[A-Za-z0-9_.-]{3,64}"
            aria-describedby="username-help"
          />
        </label>
        <p id="username-help" className="-mt-3 text-sm text-muted-foreground">
          3–64 位字母、数字、点、下划线或连字符
        </p>
        <label className="grid gap-2">
          密码
          <Input
            name="password"
            type="password"
            autoComplete={register ? "new-password" : "current-password"}
            required
            minLength={12}
            maxLength={128}
          />
        </label>
        {register && (
          <>
            <label className="grid gap-2">
              姓名
              <Input name="display_name" required maxLength={128} />
            </label>
            <div className="grid gap-2">
              <label htmlFor="role-choice">身份</label>
              <Select
                value={role}
                items={[
                  { value: "student", label: "学生" },
                  { value: "teacher", label: "教师" },
                ]}
                onValueChange={(value) => {
                  if (value === "student" || value === "teacher") setRole(value)
                }}
              >
                <SelectTrigger
                  id="role-choice"
                  className="w-full"
                  aria-label="身份"
                >
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  <SelectItem value="student">学生</SelectItem>
                  <SelectItem value="teacher">教师</SelectItem>
                </SelectContent>
              </Select>
            </div>
            {role === "student" && (
              <label className="grid gap-2">
                学号
                <Input name="student_no" required maxLength={64} />
              </label>
            )}
          </>
        )}
        {registered && (
          <p role="status">注册已成功；若登录失败，可再次登录。</p>
        )}
        {action.error && <ErrorState error={action.error} />}
        <Button type="submit" disabled={action.isPending} className="w-full">
          {action.isPending
            ? "处理中…"
            : register && !registered
              ? "注册并登录"
              : "登录"}
        </Button>
      </form>
      <p className="mt-6 text-center text-muted-foreground">
        {register ? "已有账号？" : "还没有账号？"}{" "}
        <Link
          className="font-medium text-foreground underline underline-offset-4"
          to={register ? "/login" : "/register"}
        >
          {register ? "去登录" : "创建账号"}
        </Link>
      </p>
    </main>
  )
}
