import { useEffect, useState, type FormEvent } from "react"
import { useQueryClient } from "@tanstack/react-query"
import {
  getReviewsListLessonReviewsQueryKey,
  getSummaryGetLessonAttendanceQueryKey,
  useReviewsDecideReview,
  useReviewsListLessonReviews,
} from "@/api/generated/client"
import type {
  DecisionRequest,
  PageReviewItemsItem,
  PageStudentRosterItemItemsItem,
  ReviewDecision,
} from "@/api/generated/models"
import { ApiRequestError } from "@/api/http"
import { liveQuery } from "@/app/providers"
import { useMe } from "@/features/auth/session"
import { useRosterNames } from "@/features/attendance-summary/roster-names"
import {
  Dialog,
  DialogDescription,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import {
  Button,
  EmptyState,
  ErrorState,
  LoadingRows,
  Pagination,
  StatusBadge,
  Textarea,
  formatTime,
} from "@/components/common"
import { EvidenceDownload } from "./EvidenceDownload"

export function ReviewsPanel({
  lessonId,
  courseId,
}: {
  lessonId: string
  courseId: string
}) {
  const user = useMe()
  const isTeacher = user.role === "teacher"
  const queryClient = useQueryClient()
  const [page, setPage] = useState(0)
  useEffect(() => setPage(0), [lessonId])
  const params = { limit: 20, offset: page * 20 }
  const reviews = useReviewsListLessonReviews(lessonId, params, {
    query: liveQuery,
  })
  const roster = useRosterNames(courseId, isTeacher)
  const visibleReviews = (reviews.data?.items ?? []).filter(
    (review) => isTeacher || review.user_id === user.id,
  )
  const refreshLesson = async (): Promise<void> => {
    await Promise.all([
      queryClient.invalidateQueries({
        queryKey: getReviewsListLessonReviewsQueryKey(lessonId),
      }),
      queryClient.invalidateQueries({
        queryKey: getSummaryGetLessonAttendanceQueryKey(lessonId),
      }),
    ])
  }

  if (reviews.isLoading)
    return (
      <section aria-labelledby="reviews-title">
        <h2 id="reviews-title" className="mb-4 text-xl font-semibold">
          审核记录
        </h2>
        <LoadingRows />
      </section>
    )
  if (reviews.isError || !reviews.data)
    return (
      <section aria-labelledby="reviews-title">
        <h2 id="reviews-title" className="mb-4 text-xl font-semibold">
          审核记录
        </h2>
        <ErrorState error={reviews.error} retry={() => reviews.refetch()} />
      </section>
    )

  return (
    <section aria-labelledby="reviews-title">
      <h2 id="reviews-title" className="mb-2 text-xl font-semibold">
        审核记录
      </h2>
      <p className="mb-4 text-sm text-muted-foreground">
        仅显示本课次已有的审核申请；最终结果以服务端处理为准。
      </p>
      {visibleReviews.length === 0 ? (
        <EmptyState>当前页没有审核记录。</EmptyState>
      ) : (
        <div className="divide-y border-y">
          {visibleReviews.map((review) => (
            <ReviewRow
              key={review.id}
              review={review}
              isTeacher={isTeacher}
              member={roster.data?.get(review.user_id)}
              rosterFailed={roster.isError}
              rosterLoaded={Boolean(roster.data)}
              refreshLesson={refreshLesson}
            />
          ))}
        </div>
      )}
      {isTeacher && roster.isError && (
        <div
          role="status"
          className="mt-3 flex flex-wrap items-center gap-3 text-sm text-muted-foreground"
        >
          <span>名单映射暂不可用；审核记录仍完整保留。</span>
          <Button
            variant="outline"
            className="min-h-11"
            onClick={() => void roster.refetch()}
          >
            重新读取名单
          </Button>
        </div>
      )}
      {reviews.data.total > 20 && (
        <Pagination page={page} total={reviews.data.total} onChange={setPage} />
      )}
    </section>
  )
}

function ReviewRow({
  review,
  isTeacher,
  member,
  rosterFailed,
  rosterLoaded,
  refreshLesson,
}: {
  review: PageReviewItemsItem
  isTeacher: boolean
  member?: Pick<PageStudentRosterItemItemsItem, "display_name" | "student_no">
  rosterFailed: boolean
  rosterLoaded: boolean
  refreshLesson: () => Promise<void>
}) {
  const superseded = review.decision_note === "superseded_by_success"
  const status = superseded ? "已被成功考勤替代" : review.status
  const memberLabel = member
    ? `${member.display_name} · ${member.student_no}`
    : rosterLoaded
      ? "历史名单成员"
      : rosterFailed
        ? "名单映射不可用"
        : "正在读取名单…"

  return (
    <article className="grid gap-4 py-5 lg:grid-cols-[minmax(0,1fr)_auto] lg:items-start">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <StatusBadge value={review.kind} />
          <StatusBadge value={status} />
          {isTeacher && <span className="font-medium">{memberLabel}</span>}
        </div>
        {isTeacher && !member && (rosterFailed || rosterLoaded) && (
          <p className="mt-1 break-all text-xs text-muted-foreground">
            用户 ID：
            <code className="select-all font-mono">{review.user_id}</code>
          </p>
        )}
        <p className="mt-3 whitespace-pre-wrap text-sm">{review.reason}</p>
        <dl className="mt-3 grid gap-x-4 gap-y-1 text-xs text-muted-foreground sm:grid-cols-2">
          <div>
            <dt className="inline">提交时间：</dt>
            <dd className="inline">{formatTime(review.created_at)}</dd>
          </div>
          {review.reviewed_at && (
            <div>
              <dt className="inline">审核时间：</dt>
              <dd className="inline">{formatTime(review.reviewed_at)}</dd>
            </div>
          )}
        </dl>
        {review.decision_note && !superseded && (
          <p className="mt-3 text-sm text-muted-foreground">
            审核备注：{review.decision_note}
          </p>
        )}
        {superseded && (
          <p className="mt-3 text-sm text-muted-foreground">
            此申请已被该阶段成功考勤替代，不代表教师拒绝。
          </p>
        )}
      </div>
      <div className="flex flex-wrap gap-2">
        {review.has_evidence && <EvidenceDownload reviewId={review.id} />}
        {isTeacher && review.status === "pending" && (
          <>
            <DecisionAction
              reviewId={review.id}
              decision="approve"
              refreshLesson={refreshLesson}
            />
            <DecisionAction
              reviewId={review.id}
              decision="reject"
              refreshLesson={refreshLesson}
            />
          </>
        )}
      </div>
    </article>
  )
}

function DecisionAction({
  reviewId,
  decision,
  refreshLesson,
}: {
  reviewId: string
  decision: ReviewDecision
  refreshLesson: () => Promise<void>
}) {
  const [open, setOpen] = useState(false)
  const [note, setNote] = useState("")
  const mutation = useReviewsDecideReview()
  const approve = decision === "approve"
  const title = approve ? "确认批准申请" : "确认拒绝申请"

  const handleOpenChange = (nextOpen: boolean) => {
    if (mutation.isPending) return
    setOpen(nextOpen)
    if (nextOpen) {
      mutation.reset()
      setNote("")
    }
  }
  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const data: DecisionRequest = { decision, ...(note.length ? { note } : {}) }
    try {
      await mutation.mutateAsync({ reviewId, data })
      await refreshLesson()
      setOpen(false)
    } catch (error) {
      if (error instanceof ApiRequestError && error.status === 409)
        await refreshLesson()
    }
  }

  return (
    <>
      <Button
        className="min-h-11"
        variant="outline"
        disabled={mutation.isPending}
        onClick={() => handleOpenChange(true)}
      >
        {approve ? "批准" : "拒绝"}
      </Button>
      <Dialog open={open} onOpenChange={handleOpenChange}>
        <DialogContent showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>{title}</DialogTitle>
            <DialogDescription>
              审核决定提交后由服务端保存，请核对申请原因。
            </DialogDescription>
          </DialogHeader>
          <form className="grid gap-4" onSubmit={submit}>
            <label
              className="grid gap-2 text-sm"
              htmlFor={`review-note-${reviewId}-${decision}`}
            >
              <span>备注（可选，最多 1000 字）</span>
              <Textarea
                id={`review-note-${reviewId}-${decision}`}
                maxLength={1000}
                value={note}
                onChange={(event) => setNote(event.target.value)}
                rows={3}
              />
            </label>
            {mutation.error && <ErrorState error={mutation.error} />}
            <DialogFooter className="sm:flex-row">
              <Button
                className="min-h-11"
                type="button"
                variant="outline"
                disabled={mutation.isPending}
                onClick={() => handleOpenChange(false)}
              >
                取消
              </Button>
              <Button
                className="min-h-11"
                type="submit"
                disabled={mutation.isPending}
              >
                {mutation.isPending
                  ? "正在提交…"
                  : approve
                    ? "确认批准"
                    : "确认拒绝"}
              </Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </>
  )
}
