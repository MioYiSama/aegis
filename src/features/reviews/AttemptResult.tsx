import { useEffect, useState, type FormEvent } from "react"
import { useQueryClient } from "@tanstack/react-query"
import { useLocation, useParams } from "react-router"
import {
  getReviewsListLessonReviewsQueryKey,
  getSummaryGetLessonAttendanceQueryKey,
  useReviewsCreateAttemptReview,
} from "@/api/generated/client"
import type { AttendanceAttempt, ReasonCode } from "@/api/generated/models"
import { ApiRequestError } from "@/api/http"
import { useMe } from "@/features/auth/session"
import {
  readReviewable,
  removeReviewable,
  reviewableStorageEvent,
} from "./attempt-storage"
import {
  Button,
  ErrorState,
  Link,
  StatusBadge,
  Textarea,
  formatTime,
  names,
} from "@/components/common"

const reasonLabels: Record<string, string> = {
  qr_missing: "未采集签到二维码",
  qr_unreadable: "二维码无法识别",
  qr_expired: "签到二维码已过期",
  qr_wrong_stage: "二维码不属于当前阶段",
  location_missing: "未采集定位",
  location_invalid: "定位数据无效",
  location_inaccurate: "定位精度不足",
  location_outside: "不在签到范围内",
  face_missing: "未采集人脸",
  face_unenrolled: "尚未登记人脸",
  face_not_single: "未检测到单个人脸",
  face_spoof: "活体检测未通过",
  face_mismatch: "人脸与登记信息不匹配",
}

export function AttemptResult({
  attempt,
  onRestart,
  onReviewSubmitted,
}: {
  attempt: AttendanceAttempt
  onRestart?: () => void
  onReviewSubmitted?: (attemptId: string) => void
}) {
  const user = useMe()
  const location = useLocation()
  const { lessonId } = useParams()
  const queryClient = useQueryClient()
  const [reason, setReason] = useState("")
  const [error, setError] = useState<unknown>(null)
  const [stored, setStored] = useState(() =>
    readReviewable(user.id).some((item) => item.id === attempt.id),
  )
  const {
    data: review,
    isPending,
    mutateAsync,
    reset,
  } = useReviewsCreateAttemptReview()

  useEffect(() => {
    setReason("")
    setError(null)
    setStored(readReviewable(user.id).some((item) => item.id === attempt.id))
    reset()
    const refresh = () =>
      setStored(readReviewable(user.id).some((item) => item.id === attempt.id))
    window.addEventListener(reviewableStorageEvent, refresh)
    return () => window.removeEventListener(reviewableStorageEvent, refresh)
  }, [attempt.id, user.id, reset])
  useEffect(() => {
    if (attempt.outcome === "passed" || attempt.review_id)
      removeReviewable(user.id, attempt.id)
    if (attempt.outcome === "passed" && lessonId) {
      void queryClient.invalidateQueries({
        queryKey: getSummaryGetLessonAttendanceQueryKey(lessonId),
      })
    }
  }, [
    attempt.id,
    attempt.outcome,
    attempt.review_id,
    lessonId,
    queryClient,
    user.id,
  ])

  const submitReview = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const normalizedReason = reason.trim()
    if (normalizedReason.length < 1 || normalizedReason.length > 1000) {
      setError(new Error("申请原因去除首尾空白后须为 1 至 1000 个字符。"))
      return
    }
    setError(null)
    try {
      await mutateAsync({
        attemptId: attempt.id,
        data: { reason: normalizedReason },
      })
      onReviewSubmitted?.(attempt.id)
      removeReviewable(user.id, attempt.id)
      setStored(false)
      if (lessonId)
        await Promise.all([
          queryClient.invalidateQueries({
            queryKey: getReviewsListLessonReviewsQueryKey(lessonId),
          }),
          queryClient.invalidateQueries({
            queryKey: getSummaryGetLessonAttendanceQueryKey(lessonId),
          }),
        ])
    } catch (caught) {
      setError(caught)
      if (
        caught instanceof ApiRequestError &&
        caught.status === 409 &&
        lessonId
      ) {
        await Promise.all([
          queryClient.invalidateQueries({
            queryKey: getReviewsListLessonReviewsQueryKey(lessonId),
          }),
          queryClient.invalidateQueries({
            queryKey: getSummaryGetLessonAttendanceQueryKey(lessonId),
          }),
        ])
      }
    }
  }
  const reviewAlreadyExists = Boolean(review || attempt.review_id)
  const summary = reviewAlreadyExists
    ? "审核申请已送达"
    : attempt.outcome === "passed"
      ? "考勤已通过"
      : attempt.outcome === "reviewable"
        ? "可申请，尚未送审"
        : "考勤未通过"
  const qrResult =
    attempt.factors.qr === null || attempt.factors.qr === undefined
      ? "无需扫码"
      : attempt.factors.qr
        ? "已通过"
        : "未通过"

  return (
    <section className="space-y-5" aria-labelledby="attempt-result-title">
      <div>
        <h2 id="attempt-result-title" className="text-xl font-semibold">
          考勤结果
        </h2>
        <div className="mt-2 flex flex-wrap items-center gap-2">
          <StatusBadge value={attempt.outcome} />
          <span className="text-sm">{summary}</span>
          {attempt.is_late && <StatusBadge value="迟到提交" />}
        </div>
        <p className="mt-2 text-sm text-muted-foreground">
          提交时间：{formatTime(attempt.submitted_at)}
        </p>
      </div>
      <dl className="grid gap-2 sm:grid-cols-3">
        <div className="border-t py-3">
          <dt className="text-sm text-muted-foreground">人脸</dt>
          <dd className="mt-1 font-medium">
            {attempt.factors.face ? "已通过" : "未通过"}
          </dd>
        </div>
        <div className="border-t py-3">
          <dt className="text-sm text-muted-foreground">定位</dt>
          <dd className="mt-1 font-medium">
            {attempt.factors.location ? "已通过" : "未通过"}
          </dd>
        </div>
        <div className="border-t py-3">
          <dt className="text-sm text-muted-foreground">二维码</dt>
          <dd className="mt-1 font-medium">{qrResult}</dd>
        </div>
      </dl>
      {attempt.reason_codes.length > 0 && (
        <div>
          <h3 className="font-medium">服务端返回原因</h3>
          <ul className="mt-2 list-inside list-disc space-y-1 text-sm text-muted-foreground">
            {attempt.reason_codes.map((code: ReasonCode) => (
              <li key={code}>{reasonLabels[code] ?? code}</li>
            ))}
          </ul>
        </div>
      )}
      {attempt.reason_codes.includes("face_unenrolled") && (
        <p className="text-sm">
          尚未登记人脸：
          <Link
            className="underline"
            to={`/student/enroll?returnTo=${encodeURIComponent(location.pathname + location.search)}`}
          >
            前往人脸登记
          </Link>
        </p>
      )}
      {review && (
        <div role="status" className="border-y py-4">
          <p className="font-medium">申请已送达</p>
          <p className="mt-1 text-sm text-muted-foreground">
            类型：{names[review.kind] ?? review.kind} · 状态：
            {names[review.status] ?? review.status} · 提交时间：
            {formatTime(review.created_at)}
          </p>
        </div>
      )}
      {attempt.outcome === "reviewable" && !reviewAlreadyExists && (
        <form className="grid gap-3 border-t pt-4" onSubmit={submitReview}>
          <label
            className="grid gap-2"
            htmlFor={`attempt-review-reason-${attempt.id}`}
          >
            <span className="font-medium">申请原因</span>
            <Textarea
              id={`attempt-review-reason-${attempt.id}`}
              value={reason}
              onChange={(event) => setReason(event.target.value)}
              maxLength={1000}
              rows={4}
              aria-describedby={`attempt-review-hint-${attempt.id}`}
            />
          </label>
          <p
            id={`attempt-review-hint-${attempt.id}`}
            className="text-sm text-muted-foreground"
          >
            去除首尾空白后填写 1 至 1000 个字符。是否批准由教师审核决定。
          </p>
          {Boolean(error) && <ErrorState error={error} />}
          <Button
            className="min-h-11 w-full"
            type="submit"
            disabled={isPending}
          >
            {isPending ? "正在提交…" : "提交审核申请"}
          </Button>
        </form>
      )}
      {reviewAlreadyExists && !review && (
        <p role="status" className="border-y py-4 text-sm">
          该考勤已提交审核申请。
        </p>
      )}
      {attempt.outcome === "reviewable" && !stored && !reviewAlreadyExists && (
        <p role="status" className="text-sm text-destructive">
          当前浏览器无法保存待申请回执；请勿离开此页，否则刷新后可能无法恢复。
        </p>
      )}
      {attempt.outcome === "failed" && onRestart && (
        <Button className="min-h-11" variant="outline" onClick={onRestart}>
          重新开始采集
        </Button>
      )}
    </section>
  )
}
