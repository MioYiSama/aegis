import { useEffect, useRef, useState } from "react"
import { useQueries } from "@tanstack/react-query"
import {
  getReviewsListLessonReviewsQueryOptions,
  useCoursesGetLesson,
  useReviewsListLessonReviews,
  useSummaryGetLessonAttendance,
} from "@/api/generated/client"
import type { PageReview } from "@/api/generated/models"
import { liveQuery } from "@/app/providers"
import { useMe } from "@/features/auth/session"
import { ErrorState, LoadingRows } from "@/components/common"
import { AttemptResult } from "./AttemptResult"
import {
  readReviewable,
  removeReviewable,
  reviewableStorageEvent,
} from "./attempt-storage"

const recoveryReviewPageSize = 100

export function RecoveredAttempts({
  lessonId,
  stageId,
}: {
  lessonId: string
  stageId?: string
}) {
  const user = useMe()
  const isStudent = user.role === "student"
  const [reviewableState, setReviewableState] = useState(() => ({
    userId: user.id,
    attempts: readReviewable(user.id),
  }))
  const retainedAttemptIds = useRef(new Set<string>())
  const attempts =
    reviewableState.userId === user.id ? reviewableState.attempts : []
  const lessonAttempts = attempts.filter(
    (attempt) =>
      attempt.lesson_id === lessonId &&
      (!stageId || attempt.stage_id === stageId),
  )
  const hasAttempts = isStudent && lessonAttempts.length > 0
  const lesson = useCoursesGetLesson(lessonId, {
    query: { enabled: hasAttempts, refetchOnMount: "always" },
  })
  const attendance = useSummaryGetLessonAttendance(lessonId, {
    query: { ...liveQuery, enabled: hasAttempts, refetchOnMount: "always" },
  })
  const firstReviews = useReviewsListLessonReviews(
    lessonId,
    { limit: recoveryReviewPageSize, offset: 0 },
    { query: { ...liveQuery, enabled: hasAttempts, refetchOnMount: "always" } },
  )
  const reviewPageCount = firstReviews.data
    ? Math.ceil(firstReviews.data.total / recoveryReviewPageSize)
    : 0
  const otherReviews = useQueries({
    queries: Array.from(
      { length: Math.max(0, reviewPageCount - 1) },
      (_, index) => {
        const params = {
          limit: recoveryReviewPageSize,
          offset: (index + 1) * recoveryReviewPageSize,
        }
        return getReviewsListLessonReviewsQueryOptions(lessonId, params, {
          query: {
            ...liveQuery,
            enabled: hasAttempts,
            refetchOnMount: "always",
          },
        })
      },
    ),
  })
  useEffect(() => {
    retainedAttemptIds.current.clear()
    const refresh = () => {
      const saved = readReviewable(user.id)
      setReviewableState((current) => {
        const retained =
          current.userId === user.id
            ? current.attempts.filter((attempt) =>
                retainedAttemptIds.current.has(attempt.id),
              )
            : []
        const savedIds = new Set(saved.map((attempt) => attempt.id))
        return {
          userId: user.id,
          attempts: [
            ...saved,
            ...retained.filter((attempt) => !savedIds.has(attempt.id)),
          ],
        }
      })
    }
    refresh()
    window.addEventListener(reviewableStorageEvent, refresh)
    return () => window.removeEventListener(reviewableStorageEvent, refresh)
  }, [user.id])

  const reviewPages = [
    firstReviews.data,
    ...otherReviews.map((query) => query.data),
  ].filter((page): page is PageReview => page !== undefined)
  const allReviews = reviewPages.flatMap((page) => page.items)
  const reviewsReady =
    Boolean(firstReviews.data) && otherReviews.every((query) => query.isSuccess)
  const ownStages =
    attendance.data?.students.find((student) => student.user_id === user.id)
      ?.stages ?? []
  const passedStageIds = new Set(
    ownStages
      .filter((stage) => stage.status === "passed")
      .map((stage) => stage.stage_id),
  )
  useEffect(() => {
    if (!lesson.data) return
    const stageIds = new Set(lesson.data.stages.map((stage) => stage.id))
    for (const attempt of lessonAttempts) {
      if (!stageIds.has(attempt.stage_id)) continue
      const submitted =
        reviewsReady &&
        allReviews.some(
          (review) =>
            review.user_id === user.id && review.attempt_id === attempt.id,
        )
      if (retainedAttemptIds.current.has(attempt.id)) continue
      if (passedStageIds.has(attempt.stage_id) || submitted) {
        removeReviewable(user.id, attempt.id)
      }
    }
  }, [
    allReviews,
    lesson.data,
    lessonAttempts,
    passedStageIds,
    reviewsReady,
    user.id,
  ])

  if (!isStudent || lessonAttempts.length === 0) return null
  if (lesson.isLoading)
    return (
      <section aria-label="恢复待申请考勤">
        <LoadingRows />
      </section>
    )
  if (lesson.isError || !lesson.data)
    return (
      <section aria-label="恢复待申请考勤">
        <ErrorState error={lesson.error} retry={() => lesson.refetch()} />
      </section>
    )

  const stageIds = new Set(lesson.data.stages.map((stage) => stage.id))
  const relevant = lessonAttempts.filter((attempt) =>
    stageIds.has(attempt.stage_id),
  )
  if (relevant.length === 0) return null
  const retry = () => {
    void Promise.all([
      firstReviews.refetch(),
      attendance.refetch(),
      ...otherReviews.map((query) => query.refetch()),
    ])
  }
  const reviewError =
    firstReviews.error ??
    otherReviews.find((query) => query.error)?.error ??
    attendance.error ??
    new Error("无法刷新待申请回执状态。")

  return (
    <section
      className="my-6 space-y-4"
      aria-labelledby="recovered-attempts-title"
    >
      <h2 id="recovered-attempts-title" className="text-lg font-semibold">
        考勤回执
      </h2>
      {relevant.map((attempt) => (
        <div key={attempt.id} className="border-y py-4">
          <p className="mb-3 text-sm text-muted-foreground">
            阶段 ID：
            <code className="select-all break-all font-mono">
              {attempt.stage_id}
            </code>
          </p>
          <AttemptResult
            attempt={attempt}
            onReviewSubmitted={(attemptId) => {
              retainedAttemptIds.current.add(attemptId)
            }}
          />
        </div>
      ))}
      {(firstReviews.isError ||
        otherReviews.some((query) => query.isError) ||
        attendance.isError) && <ErrorState error={reviewError} retry={retry} />}
      {!reviewsReady && !firstReviews.isError && !attendance.isError && (
        <p role="status" className="text-sm text-muted-foreground">
          正在确认该回执是否已申请审核。
        </p>
      )}
    </section>
  )
}
