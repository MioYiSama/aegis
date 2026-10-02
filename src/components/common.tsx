import { useState, type ReactNode } from "react"
import { useMutation } from "@tanstack/react-query"
import { Link, useSearchParams } from "react-router"
import { Button } from "./ui/button"
import { Input } from "./ui/input"
import { Textarea } from "./ui/textarea"
import { Badge } from "./ui/badge"
import { Skeleton } from "./ui/skeleton"
import { Tabs, TabsList, TabsTrigger } from "./ui/tabs"
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from "./ui/dialog"
import { ApiRequestError } from "@/api/http"
export { Button, Input, Textarea, Link }
export function PageHeader({
  title,
  description,
  action,
}: {
  title: string
  description?: string
  action?: ReactNode
}) {
  return (
    <header className="mb-8 flex flex-wrap items-start justify-between gap-4">
      <div className="min-w-0">
        <h1>{title}</h1>
        {description && (
          <p className="mt-2 text-muted-foreground">{description}</p>
        )}
      </div>
      {action}
    </header>
  )
}
export function EmptyState({ children }: { children: ReactNode }) {
  return (
    <div className="border-y py-12 text-center text-muted-foreground">
      {children}
    </div>
  )
}
export function errorMessage(error: unknown) {
  if (error instanceof ApiRequestError) {
    if (error.status === 403) return "禁止访问：当前身份没有权限"
    if (error.status === 404) return "内容不存在或对你不可见"
    if (error.code === "window_closed")
      return "提交窗口已关闭，请返回课次重新查看"
    if (error.status === 503)
      return "服务暂不可用，请手动重新开始。" + error.message
    if (error.status === 413) return "上传文件过大"
    return error.message
  }
  return typeof error === "string"
    ? error
    : error instanceof Error
      ? error.message
      : "请求失败，请重试"
}
export function ErrorState({
  error,
  retry,
}: {
  error: unknown
  retry?: () => void
}) {
  return (
    <div
      role="alert"
      className="my-4 rounded-lg border border-destructive/30 p-4"
    >
      <p className="text-destructive">{errorMessage(error)}</p>
      {retry && (
        <Button variant="outline" className="mt-3" onClick={retry}>
          重试
        </Button>
      )}
    </div>
  )
}
export function LoadingRows() {
  return (
    <div role="status" aria-label="正在加载" className="space-y-4">
      {[0, 1, 2].map((i) => (
        <Skeleton key={i} className="h-16 w-full" />
      ))}
    </div>
  )
}
export function Pagination({
  page,
  total,
  onChange,
}: {
  page: number
  total: number
  onChange: (page: number) => void
}) {
  return (
    <nav
      aria-label="分页"
      className="mt-6 flex items-center justify-between gap-2"
    >
      <Button
        variant="outline"
        disabled={page === 0}
        onClick={() => onChange(page - 1)}
      >
        上一页
      </Button>
      <span className="text-sm">
        第 {page + 1} 页 · 共 {total} 项
      </span>
      <Button
        variant="outline"
        disabled={(page + 1) * 20 >= total}
        onClick={() => onChange(page + 1)}
      >
        下一页
      </Button>
    </nav>
  )
}
export const names: Record<string, string> = {
  check_in: "签到",
  renew: "续签",
  check_out: "签退",
  partial: "部分因素审核",
  late: "迟到审核",
  leave: "请假",
  pending: "待审核",
  approved: "已批准",
  rejected: "已拒绝",
  in_progress: "进行中",
  excused: "已请假",
  present: "出勤",
  absent: "缺勤",
  early_departure: "早退",
  missing: "未提交",
  failed: "失败",
  passed: "通过",
  reviewable: "可申请审核",
}
export function StatusBadge({ value }: { value: string }) {
  return <Badge variant="outline">{names[value] ?? value}</Badge>
}
export function UrlTabs({
  items,
}: {
  items: { value: string; label: string }[]
}) {
  const [search, setSearch] = useSearchParams()
  const value = search.get("tab") ?? items[0].value
  return (
    <Tabs
      value={value}
      onValueChange={(v) => setSearch({ tab: String(v) })}
      className="mb-6"
    >
      <TabsList>
        {items.map((item) => (
          <TabsTrigger key={item.value} value={item.value}>
            {item.label}
          </TabsTrigger>
        ))}
      </TabsList>
    </Tabs>
  )
}
export function Confirm({
  label,
  title,
  description,
  disabled,
  action,
  children,
}: {
  label: string
  title: string
  description: string
  disabled?: boolean
  action: () => Promise<unknown>
  children?: ReactNode
}) {
  const [open, setOpen] = useState(false)
  const mutation = useMutation({
    mutationFn: action,
    onSuccess: () => setOpen(false),
  })
  return (
    <>
      <Button
        variant="outline"
        disabled={disabled}
        onClick={() => {
          mutation.reset()
          setOpen(true)
        }}
      >
        {label}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>{description}</DialogDescription>
          </DialogHeader>
          {children}
          {mutation.error && <ErrorState error={mutation.error} />}
          <DialogFooter>
            <Button
              variant="outline"
              disabled={mutation.isPending}
              onClick={() => setOpen(false)}
            >
              取消
            </Button>
            <Button
              disabled={mutation.isPending}
              onClick={() => mutation.mutate()}
            >
              {mutation.isPending ? "处理中…" : "确认"}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </>
  )
}
export function formatTime(value: string) {
  return new Date(value).toLocaleString("zh-CN", { hour12: false })
}
