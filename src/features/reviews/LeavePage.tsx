import { useRef, useState, type ChangeEvent, type FormEvent } from "react"
import { useQueryClient, useQueries } from "@tanstack/react-query"
import { Link, useParams } from "react-router"
import {
  getReviewsListLessonReviewsQueryKey,
  getReviewsListLessonReviewsQueryOptions,
  useReviewsCreateLeaveRequest,
  useReviewsListLessonReviews,
} from "@/api/generated/client"
import type {
  PageReview,
  PageReviewItemsItem,
  Review,
} from "@/api/generated/models"
import { ApiRequestError } from "@/api/http"
import { liveQuery } from "@/app/providers"
import { useMe } from "@/features/auth/session"
import {
  Button,
  EmptyState,
  ErrorState,
  LoadingRows,
  PageHeader,
  StatusBadge,
  Textarea,
  formatTime,
} from "@/components/common"
import { EvidenceDownload } from "./EvidenceDownload"

const leaveReviewPageSize = 100
const allowedEvidenceTypes: Record<string, true> = {
  "image/jpeg": true,
  "image/png": true,
  "application/pdf": true,
}
const maxEvidenceBytes = 5 * 1024 * 1024

function evidenceValidation(file: File): string | null {
  if (file.size > maxEvidenceBytes) return "凭据文件不能超过 5 MiB。"
  if (!allowedEvidenceTypes[file.type]) return "请选择 JPEG、PNG 或 PDF 文件。"
  return null
}

export function LeavePage() {
  const { lessonId } = useParams()
  const user = useMe()
  const queryClient = useQueryClient()
  const [reason, setReason] = useState("")
  const [evidence, setEvidence] = useState<File | null>(null)
  const [fileError, setFileError] = useState<string | null>(null)
  const [formError, setFormError] = useState<unknown>(null)
  const [refreshing, setRefreshing] = useState(false)
  const fileInput = useRef<HTMLInputElement>(null)
  const firstPage = useReviewsListLessonReviews(
    lessonId ?? "",
    { limit: leaveReviewPageSize, offset: 0 },
    {
      query: {
        ...liveQuery,
        enabled: Boolean(lessonId),
        refetchOnMount: "always",
      },
    },
  )
  const pageCount = firstPage.data
    ? Math.ceil(firstPage.data.total / leaveReviewPageSize)
    : 0
  const remainingPages = useQueries({
    queries: Array.from({ length: Math.max(0, pageCount - 1) }, (_, index) => {
      const params = {
        limit: leaveReviewPageSize,
        offset: (index + 1) * leaveReviewPageSize,
      }
      return getReviewsListLessonReviewsQueryOptions(lessonId ?? "", params, {
        query: {
          ...liveQuery,
          enabled: Boolean(lessonId),
          refetchOnMount: "always",
        },
      })
    }),
  })
  const mutation = useReviewsCreateLeaveRequest()
  const pages = [
    firstPage.data,
    ...remainingPages.map((query) => query.data),
  ].filter((page): page is PageReview => page !== undefined)
  const myReviews = pages
    .flatMap((page) => page.items)
    .filter((review) => review.user_id === user.id)
  const myLeaves = myReviews.filter((review) => review.kind === "leave")
  const activeLeave = myLeaves.find(
    (review) => review.status === "pending" || review.status === "approved",
  )
  const pagesLoading =
    firstPage.isLoading || remainingPages.some((query) => query.isLoading)
  const pagesError =
    firstPage.error ?? remainingPages.find((query) => query.error)?.error
  const refreshReviews = async () =>
    queryClient.invalidateQueries({
      queryKey: getReviewsListLessonReviewsQueryKey(lessonId ?? ""),
    })
  const handleFileChange = (event: ChangeEvent<HTMLInputElement>) => {
    const selected = event.target.files?.[0] ?? null
    setEvidence(selected)
    setFileError(selected ? evidenceValidation(selected) : null)
  }
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    setFormError(null)
    const normalizedReason = reason.trim()
    if (normalizedReason.length < 1 || normalizedReason.length > 1000) {
      setFormError(new Error("请假原因去除首尾空白后须为 1 至 1000 个字符。"))
      return
    }
    if (!evidence) {
      setFileError("请选择 JPEG、PNG 或 PDF 凭据文件。")
      return
    }
    const invalidEvidence = evidenceValidation(evidence)
    if (invalidEvidence) {
      setFileError(invalidEvidence)
      return
    }
    if (!lessonId || activeLeave || pagesLoading || pagesError) return

    setRefreshing(true)
    try {
      await mutation.mutateAsync({
        lessonId,
        data: { reason: normalizedReason, evidence },
      })
      setReason("")
      setEvidence(null)
      setFileError(null)
      if (fileInput.current) fileInput.current.value = ""
      await refreshReviews()
    } catch (error) {
      setFormError(error)
      if (error instanceof ApiRequestError && error.status === 409)
        await refreshReviews()
    } finally {
      setRefreshing(false)
    }
  }
  const retry = () => {
    void Promise.all([
      firstPage.refetch(),
      ...remainingPages.map((query) => query.refetch()),
    ])
  }
  const createdReview: Review | undefined = mutation.data
  const createdListed =
    createdReview && myReviews.some((review) => review.id === createdReview.id)
  const createdPending = Boolean(
    createdReview &&
    (createdListed
      ? myReviews.find((review) => review.id === createdReview.id)?.status ===
        "pending"
      : createdReview.status === "pending"),
  )
  const isBlocked = Boolean(activeLeave || createdPending)
  return (
    <div className="space-y-7 py-6">
      <PageHeader
        title="请假申请"
        description="本次请假仅在服务端审核通过后计为已请假。"
      />
      {activeLeave && (
        <p role="status" className="border-y py-4 text-sm">
          本课次已有{activeLeave.status === "pending" ? "待审核" : "已批准"}
          请假申请，暂不能重复提交。
        </p>
      )}
      {pagesLoading ? (
        <LoadingRows />
      ) : pagesError ? (
        <ErrorState error={pagesError} retry={retry} />
      ) : null}
      <form className="grid gap-5" onSubmit={submit}>
        <label className="grid gap-2" htmlFor="leave-reason">
          <span className="font-medium">请假原因</span>
          <Textarea
            id="leave-reason"
            value={reason}
            onChange={(event) => setReason(event.target.value)}
            maxLength={1000}
            rows={5}
            aria-describedby="leave-reason-hint"
          />
        </label>
        <p
          id="leave-reason-hint"
          className="-mt-3 text-sm text-muted-foreground"
        >
          去除首尾空白后须为 1 至 1000 个字符。
        </p>
        <div className="grid gap-2">
          <label className="font-medium" htmlFor="leave-evidence">
            凭据文件
          </label>
          <input
            ref={fileInput}
            id="leave-evidence"
            type="file"
            accept="image/jpeg,image/png,application/pdf,.jpg,.jpeg,.png,.pdf"
            onChange={handleFileChange}
            aria-describedby="leave-evidence-hint"
            className="min-h-11 w-full rounded-md border px-3 py-2 text-sm"
          />
          <p id="leave-evidence-hint" className="text-sm text-muted-foreground">
            仅 JPEG、PNG、PDF，最大 5 MiB。文件不会转换为 Base64
            或发送至外部服务。
          </p>
          {evidence && (
            <p className="break-all text-sm">
              已选：{evidence.name} · {(evidence.size / 1024).toFixed(1)} KiB
            </p>
          )}
          {fileError && (
            <p role="alert" className="text-sm text-destructive">
              {fileError}
            </p>
          )}
        </div>
        {Boolean(formError) && <ErrorState error={formError} />}
        <Button
          className="min-h-11 w-full"
          type="submit"
          disabled={
            !lessonId ||
            pagesLoading ||
            Boolean(pagesError) ||
            isBlocked ||
            mutation.isPending ||
            refreshing
          }
        >
          {mutation.isPending || refreshing ? "正在提交…" : "提交请假申请"}
        </Button>
      </form>
      <section aria-labelledby="leave-applications-title">
        <h2
          id="leave-applications-title"
          className="mb-3 text-lg font-semibold"
        >
          本课次个人申请
        </h2>
        {pagesLoading ? (
          <LoadingRows />
        ) : pagesError ? null : myReviews.length === 0 ? (
          <EmptyState>暂无个人审核申请。</EmptyState>
        ) : (
          <div className="divide-y border-y">
            {myReviews.map((review) => (
              <LeaveReview key={review.id} review={review} />
            ))}
          </div>
        )}
      </section>
      {lessonId && (
        <Link
          className="inline-flex min-h-11 items-center underline"
          to={`/student/lessons/${lessonId}`}
        >
          返回课次
        </Link>
      )}
    </div>
  )
}

function LeaveReview({ review }: { review: PageReviewItemsItem }) {
  const superseded = review.decision_note === "superseded_by_success"
  const status = superseded ? "已被成功考勤替代" : review.status
  return (
    <article className="grid gap-3 py-4">
      <div className="flex flex-wrap items-center gap-2">
        <StatusBadge value={review.kind} />
        <StatusBadge value={status} />
      </div>
      <p className="whitespace-pre-wrap text-sm">{review.reason}</p>
      <p className="text-xs text-muted-foreground">
        提交于 {formatTime(review.created_at)}
        {review.reviewed_at && ` · 审核于 ${formatTime(review.reviewed_at)}`}
      </p>
      {review.decision_note && !superseded && (
        <p className="text-sm text-muted-foreground">
          审核备注：{review.decision_note}
        </p>
      )}
      {superseded && (
        <p className="text-sm text-muted-foreground">
          已被成功考勤替代，不代表教师拒绝。
        </p>
      )}
      {review.has_evidence && <EvidenceDownload reviewId={review.id} />}
      {review.kind === "leave" && review.status === "rejected" && (
        <p className="text-sm text-muted-foreground">
          此申请已拒绝，可重新提交。
        </p>
      )}
    </article>
  )
}
