import { useEffect, useState } from "react"
import { useParams } from "react-router"
import { ArrowsOut } from "@phosphor-icons/react"
import {
  getStagesGetStageQrUrl,
  useCoursesGetCourse,
  useCoursesGetLesson,
} from "@/api/generated/client"
import { ApiRequestError, requestBinary } from "@/api/http"
import { liveQuery, registerSessionCleanup } from "@/app/providers"
import {
  Button,
  ErrorState,
  Link,
  LoadingRows,
  PageHeader,
} from "@/components/common"
import { stageActive } from "@/features/lessons/window"

export function ProjectionPage() {
  const { lessonId = "", stageId = "" } = useParams()
  const detail = useCoursesGetLesson(lessonId, { query: liveQuery })
  const course = useCoursesGetCourse(detail.data?.course_id ?? "", {
    query: { enabled: Boolean(detail.data) },
  })
  const [now, setNow] = useState(Date.now())
  const [image, setImage] = useState<string | null>(null)
  const [error, setError] = useState<unknown>(null)
  const [revision, setRevision] = useState(0)
  const stage = detail.data?.stages.find((item) => item.id === stageId)
  const active = Boolean(
    detail.data &&
    stage &&
    stage.kind === "check_in" &&
    stageActive(detail.data, stage, now),
  )

  useEffect(() => {
    const clock = window.setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(clock)
  }, [])

  useEffect(() => {
    if (!active) {
      setImage(null)
      return
    }
    let disposed = false
    let url: string | null = null
    let timer: number | undefined
    let controller: AbortController | undefined
    let inFlight = false
    const clearImage = () => {
      if (url) URL.revokeObjectURL(url)
      url = null
      setImage(null)
      clearTimeout(timer)
    }
    const fetchQr = async () => {
      if (disposed || document.hidden || inFlight) return
      inFlight = true
      controller = new AbortController()
      try {
        const requestStartedAt = performance.now()
        const { blob, headers } = await requestBinary(
          getStagesGetStageQrUrl(stageId),
          controller.signal,
        )
        if (disposed || document.hidden || controller.signal.aborted) return
        const rawExpiry = headers.get("X-QR-Expires-At")
        const expiresAt =
          rawExpiry &&
          /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?(?:Z|[+-]\d{2}:\d{2})$/.test(
            rawExpiry,
          )
            ? Date.parse(rawExpiry)
            : NaN
        if (!Number.isFinite(expiresAt) || blob.type !== "image/png")
          throw new Error("二维码协议错误：缺少合法有效期或 PNG 内容类型")
        const rawDate = headers.get("Date")
        const serverDate =
          rawDate &&
          /^[A-Z][a-z]{2}, \d{2} [A-Z][a-z]{2} \d{4} \d{2}:\d{2}:\d{2} GMT$/.test(
            rawDate,
          )
            ? Date.parse(rawDate)
            : NaN
        // Bound server time using Date precision plus all request/body delivery time.
        const estimatedServerNow = Number.isFinite(serverDate)
          ? serverDate + 1000 + Math.max(0, performance.now() - requestStartedAt)
          : Date.now()
        const ttl = Math.max(0, Math.min(5000, expiresAt - estimatedServerNow))
        clearImage()
        setError(null)
        if (ttl > 0) {
          url = URL.createObjectURL(blob)
          setImage(url)
        }
        // Remove the expired image before starting the next network request.
        timer = window.setTimeout(() => {
          clearImage()
          void fetchQr()
        }, ttl || 250)
      } catch (failure) {
        if (disposed || controller?.signal.aborted) return
        clearImage()
        setError(failure)
        if (failure instanceof ApiRequestError && failure.status === 409)
          void detail.refetch()
      } finally {
        inFlight = false
        if (!disposed && !document.hidden && controller?.signal.aborted)
          void fetchQr()
      }
    }
    const visibility = () => {
      clearImage()
      if (document.hidden) controller?.abort()
      else if (!inFlight) void fetchQr()
    }
    const stop = () => {
      disposed = true
      controller?.abort()
      clearImage()
    }
    const unregister = registerSessionCleanup(stop)
    document.addEventListener("visibilitychange", visibility)
    setError(null)
    void fetchQr()
    return () => {
      stop()
      unregister()
      document.removeEventListener("visibilitychange", visibility)
    }
  }, [active, stageId, revision])

  if (detail.isPending) return <LoadingRows />
  if (detail.error)
    return (
      <ErrorState error={detail.error} retry={() => void detail.refetch()} />
    )
  if (!stage || stage.kind !== "check_in")
    return (
      <ErrorState
        error={new ApiRequestError(404, "not_found", "签到阶段不可见")}
      />
    )
  return (
    <div className="mx-auto max-w-[1000px]">
      <Link
        className="mb-5 inline-flex min-h-11 items-center"
        to={"/teacher/lessons/" + lessonId}
      >
        ← 返回课次
      </Link>
      <PageHeader
        title={course.data?.title ?? detail.data!.title}
        description={detail.data!.title + " · 签到投屏"}
        action={
          <Button
            variant="outline"
            onClick={() => {
              void document.documentElement.requestFullscreen().catch(setError)
            }}
          >
            <ArrowsOut size={20} />
            全屏
          </Button>
        }
      />
      <div className="mb-4 flex flex-wrap justify-between gap-3 text-muted-foreground">
        <p>完整拍摄二维码及外框 · 每张最多有效 5 秒</p>
        <p role="timer">
          {active
            ? "阶段剩余 " +
              Math.max(
                0,
                Math.ceil((Date.parse(stage.closes_at) - now) / 1000),
              ) +
              " 秒"
            : "签到已结束"}
        </p>
      </div>
      {active && Boolean(error) && (
        <ErrorState
          error={error}
          retry={() => setRevision((value) => value + 1)}
        />
      )}
      <div className="flex min-h-64 items-center justify-center border-y py-6">
        {active && image ? (
          <img
            src={image}
            alt="当前有效签到色度二维码"
            className="block h-auto max-h-[70dvh] w-auto max-w-full object-contain"
          />
        ) : (
          <p role="status" className="text-muted-foreground">
            {!active
              ? "签到已结束"
              : error
                ? "二维码已移除，请手动恢复"
                : "正在获取新二维码…"}
          </p>
        )}
      </div>
    </div>
  )
}
