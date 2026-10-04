import { useEffect, useRef, useState } from "react"
import { useQueryClient } from "@tanstack/react-query"
import { useParams } from "react-router"
import {
  attendanceIssueFaceChallenge,
  attendanceSubmitAttempt,
  getCoursesGetLessonQueryKey,
  getSummaryGetLessonAttendanceQueryKey,
  useCoursesGetLesson,
  useSummaryGetLessonAttendance,
} from "@/api/generated/client"
import type {
  AttendanceAttempt,
  AttendanceAttemptMultipart,
  LessonDetail,
  Stage,
} from "@/api/generated/models"
import { ApiRequestError } from "@/api/http"
import {
  Button,
  ErrorState,
  Link,
  LoadingRows,
  PageHeader,
  errorMessage,
  names,
} from "@/components/common"
import { liveQuery } from "@/app/providers"
import { useMe } from "@/features/auth/session"
import { submissionWindow } from "@/features/lessons/window"
import { AttemptResult } from "@/features/reviews/AttemptResult"
import { RecoveredAttempts } from "@/features/reviews/RecoveredAttempts"
import {
  readReviewable,
  reviewableStorageEvent,
  saveReviewable,
} from "@/features/reviews/attempt-storage"
import { scanChromaFrame } from "@/wasm/client"
import {
  delay,
  imageDataToPng,
  readVideoFrame,
  validateUploadSize,
} from "./media"
import { useCaptureSession } from "./useCaptureSession"
import { useFaceCapture } from "./useFaceCapture"
import { FaceCaptureStatus } from "./FaceCaptureStatus"

type CaptureWindow = "regular" | "late"
type CapturePhase =
  | "idle"
  | "preparing"
  | "face"
  | "location"
  | "scanning"
  | "submitting"
  | "result"

const CAPTURE_PHASE_LABELS: Partial<Record<CapturePhase, string>> = {
  preparing: "正在刷新课次并准备相机…",
  location: "正在等待定位…",
  scanning: "正在扫码…",
  submitting: "正在提交…",
}
export function AttendPage() {
  const { lessonId, stageId } = useParams()
  const user = useMe()
  const queryClient = useQueryClient()
  const media = useCaptureSession()
  const face = useFaceCapture()
  const detailQuery = useCoursesGetLesson(lessonId ?? "", {
    query: {
      enabled: Boolean(lessonId) && user.role === "student",
      staleTime: 0,
      ...liveQuery,
    },
  })
  const attendanceQuery = useSummaryGetLessonAttendance(lessonId ?? "", {
    query: {
      enabled: Boolean(lessonId) && user.role === "student",
      staleTime: 0,
      ...liveQuery,
    },
  })
  const [phase, setPhase] = useState<CapturePhase>("idle")
  const [skipFace, setSkipFace] = useState(false)
  const [skipLocation, setSkipLocation] = useState(false)
  const [skipQr, setSkipQr] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  const [storageWarning, setStorageWarning] = useState(false)
  const [attempt, setAttempt] = useState<AttendanceAttempt | null>(null)
  const [hasRecoverableAttempt, setHasRecoverableAttempt] = useState(() =>
    Boolean(
      lessonId &&
      stageId &&
      readReviewable(user.id).some(
        (receipt) =>
          receipt.lesson_id === lessonId && receipt.stage_id === stageId,
      ),
    ),
  )
  const [now, setNow] = useState(() => Date.now())
  const [challengeExpiresAt, setChallengeExpiresAt] = useState<number | null>(
    null,
  )
  const operationRef = useRef(false)
  const cancelRequestedRef = useRef(false)
  const activeWindowRef = useRef<CaptureWindow | null>(null)

  const lesson = detailQuery.data
  const stage = lesson?.stages.find((item) => item.id === stageId)
  const ownAttendance = attendanceQuery.data?.students.find(
    (item) => item.user_id === user.id,
  )
  const stageAttendance = ownAttendance?.stages.find(
    (item) => item.stage_id === stageId,
  )
  const currentWindow =
    lesson && stage ? submissionWindow(lesson, stage, now) : null
  const canOmitFactors = Boolean(
    stage?.kind === "check_in" && currentWindow === "regular",
  )
  const isCapturing =
    phase === "preparing" ||
    phase === "face" ||
    phase === "location" ||
    phase === "scanning"
  const captureDeadline =
    currentWindow === "late" && lesson
      ? Date.parse(lesson.starts_at) + 15 * 60 * 1000
      : currentWindow === "regular" && stage
        ? Date.parse(stage.closes_at)
        : null
  const windowSecondsRemaining =
    captureDeadline !== null && Number.isFinite(captureDeadline)
      ? Math.max(0, Math.ceil((captureDeadline - now) / 1000))
      : null
  const challengeSecondsRemaining =
    challengeExpiresAt === null
      ? null
      : Math.max(0, Math.ceil((challengeExpiresAt - now) / 1000))
  const captureButtonLabel =
    CAPTURE_PHASE_LABELS[phase] ??
    (phase === "face"
      ? face.aligning
        ? "请对准镜头，准备好后开始拍摄"
        : face.countdown > 0
          ? `${face.countdown} 秒后开始拍摄…`
          : `正在采集人脸照片 ${face.frames}/3…`
      : currentWindow
        ? "开始采集"
        : "当前不可提交")

  useEffect(() => {
    const refreshRecoverableState = () => {
      setHasRecoverableAttempt(
        Boolean(
          lessonId &&
          stageId &&
          readReviewable(user.id).some(
            (receipt) =>
              receipt.lesson_id === lessonId && receipt.stage_id === stageId,
          ),
        ),
      )
    }
    refreshRecoverableState()
    window.addEventListener(reviewableStorageEvent, refreshRecoverableState)
    return () =>
      window.removeEventListener(
        reviewableStorageEvent,
        refreshRecoverableState,
      )
  }, [lessonId, stageId, user.id])
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])

  useEffect(() => {
    if (
      !activeWindowRef.current ||
      phase === "submitting" ||
      phase === "result"
    )
      return
    if (currentWindow !== activeWindowRef.current) {
      activeWindowRef.current = null
      setChallengeExpiresAt(null)
      media.stopSession()
      setPhase("idle")
      face.reset()
      setSkipFace(false)
      setSkipLocation(false)
      setSkipQr(false)
      setError(
        new Error(
          "考勤窗口已变化，已停止本次采集。请重新查看当前阶段后手动开始；迟到流程不会沿用旧挑战或二维码。",
        ),
      )
    }
  }, [currentWindow, media.stopSession, phase])

  useEffect(() => {
    if (
      challengeExpiresAt === null ||
      now < challengeExpiresAt ||
      !activeWindowRef.current ||
      phase === "submitting" ||
      phase === "result"
    )
      return
    activeWindowRef.current = null
    setChallengeExpiresAt(null)
    media.stopSession()
    setPhase("idle")
    face.reset()
    setError(
      new Error(
        "本次人脸挑战已过期，已停止采集。请手动重新开始以获取新的挑战。",
      ),
    )
  }, [challengeExpiresAt, media.stopSession, now, phase])

  async function fetchFreshStage(): Promise<{
    stage: Stage
    window: CaptureWindow
  }> {
    if (!lessonId || !stageId) throw new Error("缺少课次或阶段参数")
    const [lessonResult, attendanceResult] = await Promise.all([
      detailQuery.refetch({ throwOnError: true }),
      attendanceQuery.refetch({ throwOnError: true }),
    ])
    const freshLesson = lessonResult.data
    const freshAttendance = attendanceResult.data
    if (!freshLesson || freshLesson.id !== lessonId)
      throw new Error("无法读取最新课次，请重试")
    const freshStage = freshLesson.stages.find((item) => item.id === stageId)
    if (!freshStage) throw new Error("此阶段不属于当前课次或已不可见")
    const studentRecord = freshAttendance?.students.find(
      (item) => item.user_id === user.id,
    )
    if (!studentRecord)
      throw new Error("当前课次名单中没有你的签到记录，无法提交本阶段")
    if (
      studentRecord.stages.find((item) => item.stage_id === stageId)?.status ===
      "passed"
    ) {
      throw new Error("本阶段已经通过，无需重复提交")
    }
    const window = submissionWindow(freshLesson, freshStage, Date.now())
    if (!window)
      throw new ApiRequestError(
        409,
        "window_closed",
        "当前阶段已不在可提交窗口内",
      )
    return { stage: freshStage, window }
  }

  function resetForRetry() {
    media.stopSession()
    activeWindowRef.current = null
    operationRef.current = false
    setChallengeExpiresAt(null)
    setPhase("idle")
    face.reset()
    setAttempt(null)
    setStorageWarning(false)
    setError(null)
  }

  function cancelCapture() {
    if (!isCapturing || cancelRequestedRef.current) return
    cancelRequestedRef.current = true
    activeWindowRef.current = null
    setChallengeExpiresAt(null)
    media.stopSession()
    setError(
      new Error("正在停止采集；下一次开始会重新读取课次信息并获取新的挑战。"),
    )
  }

  async function startCapture() {
    if (operationRef.current || phase !== "idle") return
    operationRef.current = true
    cancelRequestedRef.current = false
    setError(null)
    setAttempt(null)
    setStorageWarning(false)
    face.reset()
    setChallengeExpiresAt(null)
    setPhase("preparing")
    const controller = media.beginSession()
    const signal = controller.signal

    try {
      const fresh = await fetchFreshStage()
      if (signal.aborted) throw new DOMException("采集已取消", "AbortError")
      const omitFactors =
        fresh.window === "regular" && fresh.stage.kind === "check_in"
      if (!omitFactors && (skipFace || skipLocation || skipQr)) {
        setSkipFace(false)
        setSkipLocation(false)
        setSkipQr(false)
        throw new Error(
          "当前窗口不允许省略考勤因素；迟到、续签和签退必须采集实际要求的因素。",
        )
      }

      activeWindowRef.current = fresh.window
      const shouldCaptureFace = !(omitFactors && skipFace)
      const shouldCaptureLocation = !(omitFactors && skipLocation)
      const shouldScanQr =
        fresh.window === "regular" &&
        fresh.stage.kind === "check_in" &&
        !(omitFactors && skipQr)
      if (shouldCaptureLocation) media.startLocation()
      if (shouldCaptureFace) {
        await media.startCamera("user", signal)
        setPhase("face")
        await face.prepare(signal)
      }
      const challenge = await attendanceIssueFaceChallenge(
        { purpose: "attendance", stage_id: fresh.stage.id },
        { signal },
      )
      if (signal.aborted) throw new DOMException("采集已取消", "AbortError")
      const challengeExpiry = Date.parse(challenge.expires_at)
      if (!Number.isFinite(challengeExpiry))
        throw new Error("服务器返回的挑战有效期无效，请手动重新开始")
      setChallengeExpiresAt(challengeExpiry)
      let frames: [Blob, Blob, Blob] | null = null
      if (shouldCaptureFace) {
        setPhase("face")
        frames = await face.capture(media.videoRef.current!, signal)
        media.stopCamera()
      }

      let position: GeolocationPosition | null = null
      if (shouldCaptureLocation) {
        setPhase("location")
        position = await media.waitForLocation(signal)
      }

      async function submit(qrImage?: Blob) {
        const body: AttendanceAttemptMultipart = {
          payload: { face_challenge_id: challenge.id },
        }
        const latestPosition = shouldCaptureLocation
          ? (media.currentPosition() ?? position)
          : null
        if (latestPosition) {
          body.payload.location = {
            latitude: latestPosition.coords.latitude,
            longitude: latestPosition.coords.longitude,
            accuracy_m: latestPosition.coords.accuracy,
          }
        }
        const uploadBlobs: Blob[] = []
        if (frames) {
          body.frame_0 = frames[0]
          body.frame_1 = frames[1]
          body.frame_2 = frames[2]
          uploadBlobs.push(...frames)
        }
        if (qrImage) {
          body.qr_image = qrImage
          uploadBlobs.push(qrImage)
        }
        validateUploadSize(uploadBlobs)
        setPhase("submitting")
        const response = await attendanceSubmitAttempt(fresh.stage.id, body, {
          signal,
        })
        setChallengeExpiresAt(null)
        media.stopSession()
        activeWindowRef.current = null
        setAttempt(response)
        setPhase("result")
        if (response.outcome === "reviewable") {
          try {
            if (!lessonId || !saveReviewable(user.id, response, lessonId))
              setStorageWarning(true)
          } catch {
            setStorageWarning(true)
          }
        }
        if (response.outcome === "passed") {
          await Promise.all([
            queryClient.invalidateQueries({
              queryKey: getCoursesGetLessonQueryKey(lessonId!),
            }),
            queryClient.invalidateQueries({
              queryKey: getSummaryGetLessonAttendanceQueryKey(lessonId!),
            }),
          ])
        }
      }

      if (shouldScanQr) {
        setPhase("scanning")
        await media.startCamera("environment", signal)
        const video = media.videoRef.current
        if (!video) throw new Error("扫码相机预览尚未准备好，请重新开始")
        while (!signal.aborted) {
          const readableFrame = await scanChromaFrame(
            readVideoFrame(video),
            signal,
          )
          if (readableFrame) {
            const qrImage = await imageDataToPng(readableFrame, signal)
            await submit(qrImage)
            break
          }
          await delay(250, signal)
        }
      } else {
        await submit()
      }
    } catch (cause) {
      activeWindowRef.current = null
      setChallengeExpiresAt(null)
      media.stopSession()
      setPhase("idle")
      if (cause instanceof DOMException && cause.name === "AbortError") return
      if (cause instanceof ApiRequestError && cause.code === "window_closed") {
        await Promise.all([detailQuery.refetch(), attendanceQuery.refetch()])
        setError(
          new Error(
            "提交窗口已关闭，课次信息已刷新。请根据当前窗口重新手动开始，系统不会重放本次提交。",
          ),
        )
      } else {
        setError(
          cause instanceof Error ? cause : new Error(errorMessage(cause)),
        )
      }
    } finally {
      operationRef.current = false
    }
  }

  if (user.role !== "student")
    return <ErrorState error="仅学生可以提交本人考勤" />
  if (!lessonId || !stageId) return <ErrorState error="缺少课次或阶段参数" />
  if (detailQuery.isPending || attendanceQuery.isPending) return <LoadingRows />
  if (detailQuery.error || attendanceQuery.error) {
    return (
      <ErrorState
        error={detailQuery.error ?? attendanceQuery.error}
        retry={() => {
          void detailQuery.refetch()
          void attendanceQuery.refetch()
        }}
      />
    )
  }
  if (!lesson || !stage)
    return <ErrorState error="此阶段不属于当前课次或已不可见" />

  return (
    <div className="space-y-5">
      <div className="mb-6">
        <Link
          className="inline-flex min-h-11 items-center"
          to={`/student/lessons/${lessonId}`}
        >
          返回课次
        </Link>
      </div>
      <PageHeader
        title={`${names[stage.kind] ?? stage.kind}采集`}
        description={lesson.title}
      />

      <section className="space-y-5">
        <div className="border-y py-4 text-sm leading-6">
          <p>
            阶段：{names[stage.kind] ?? stage.kind} · {stage.ordinal + 1}
          </p>
          <p className="text-muted-foreground">
            {currentWindow === "regular"
              ? "请按页面提示完成当前窗口要求。"
              : currentWindow === "late"
                ? "这是迟到候选流程；须由教师审核，且仍需人脸与定位。"
                : "当前阶段暂不可提交。"}
          </p>
          <p className="text-muted-foreground">
            相机只在你点击开始后启用；照片和扫码帧仅在本页内存中处理，不写入浏览器存储。
          </p>
          {windowSecondsRemaining !== null && (
            <p role="status" className="text-muted-foreground">
              {currentWindow === "late" ? "迟到候选截止" : "当前阶段窗口"}剩余{" "}
              {windowSecondsRemaining} 秒；本地倒计时仅供参考，服务器判定为准。
            </p>
          )}
          {challengeSecondsRemaining !== null && isCapturing && (
            <p role="status" className="text-muted-foreground">
              本次人脸挑战预计剩余 {challengeSecondsRemaining}{" "}
              秒；过期后会停止采集并要求手动重新开始。
            </p>
          )}
        </div>

        {stageAttendance?.status === "passed" && (
          <p role="status" className="border-y py-3 text-sm">
            本阶段已经通过，无需再次采集。
          </p>
        )}
        {!ownAttendance && (
          <ErrorState error="当前课次名单中没有你的签到记录，无法提交本阶段" />
        )}
        {canOmitFactors &&
          ownAttendance &&
          stageAttendance?.status !== "passed" &&
          phase === "idle" &&
          !attempt && (
            <fieldset className="space-y-2">
              <legend className="mb-2 font-medium">可选的部分因素提交</legend>
              <p className="mb-3 text-sm text-muted-foreground">
                仅普通签到窗口允许明确省略因素；服务器会根据实际收到的因素判定结果。
              </p>
              <label className="flex min-h-11 cursor-pointer items-center gap-3 rounded-md border px-3 py-2 text-sm">
                <input
                  type="checkbox"
                  checked={skipFace}
                  onChange={(event) => setSkipFace(event.target.checked)}
                  className="size-4 accent-primary"
                />
                无法采集人脸，明确提交时省略人脸照片
              </label>
              <label className="flex min-h-11 cursor-pointer items-center gap-3 rounded-md border px-3 py-2 text-sm">
                <input
                  type="checkbox"
                  checked={skipLocation}
                  onChange={(event) => setSkipLocation(event.target.checked)}
                  className="size-4 accent-primary"
                />
                无法定位，明确提交时省略定位数据
              </label>
              <label className="flex min-h-11 cursor-pointer items-center gap-3 rounded-md border px-3 py-2 text-sm">
                <input
                  type="checkbox"
                  checked={skipQr}
                  onChange={(event) => setSkipQr(event.target.checked)}
                  className="size-4 accent-primary"
                />
                无法扫码，明确提交时省略二维码图像
              </label>
            </fieldset>
          )}

        <div
          className={
            media.facingMode || phase === "preparing" || phase === "scanning"
              ? "space-y-3"
              : "hidden"
          }
        >
          <div className="relative mx-auto w-full max-w-72 overflow-hidden rounded-lg bg-black">
            <video
              ref={media.videoRef}
              className="aspect-[9/16] w-full object-cover object-center"
              autoPlay
              muted
              playsInline
              aria-label={
                media.facingMode === "environment"
                  ? "后置相机扫码预览"
                  : "前置相机人脸预览"
              }
            />
          </div>
          {phase === "face" && (
            <FaceCaptureStatus
              aligning={face.aligning}
              countdown={face.countdown}
              frames={face.frames}
              onConfirm={face.confirm}
            />
          )}
          {media.facingMode === "environment" && (
            <p role="status" className="text-sm text-muted-foreground">
              正在扫描二维码；识别到载波后会立即提交当前帧，不会增加确认步骤。
            </p>
          )}
        </div>
        {face.frames > 0 && (
          <p role="status" className="text-sm font-medium">
            {face.frames === 3
              ? "人脸照片已采集完成（3/3）。是否验证通过以服务器考勤结果为准。"
              : `已采集人脸照片 ${face.frames}/3`}
          </p>
        )}
        {skipFace && (
          <p className="text-sm text-muted-foreground">
            人脸因素已明确省略，不会上传照片。
          </p>
        )}

        <div className="border-y py-4 text-sm">
          <p className="font-medium">定位状态</p>
          {skipLocation ? (
            <p className="mt-1 text-muted-foreground">
              定位因素已明确省略，不会上传定位数据。
            </p>
          ) : media.locationStatus === "ready" && media.location ? (
            <p role="status" className="mt-1 text-muted-foreground">
              已采集实际设备定位 · 精度约{" "}
              {Math.round(media.location.coords.accuracy)}{" "}
              米；是否通过由服务器判定。
            </p>
          ) : media.locationError ? (
            <p role="alert" className="mt-1 text-destructive">
              {media.locationError}
            </p>
          ) : media.locationStatus === "waiting" ? (
            <p role="status" className="mt-1 text-muted-foreground">
              正在等待设备定位…
            </p>
          ) : (
            <p className="mt-1 text-muted-foreground">尚未采集定位。</p>
          )}
        </div>

        {error && error.message !== media.locationError && (
          <ErrorState error={error} />
        )}
        {storageWarning && (
          <p role="alert" className="border-y py-3 text-sm text-destructive">
            浏览器未能保存待申请审核的最少回执信息。请不要离开当前页面，否则刷新后无法恢复这条申请入口。
          </p>
        )}

        {attempt && (
          <AttemptResult
            attempt={attempt}
            onRestart={
              attempt.outcome === "failed" && currentWindow
                ? resetForRetry
                : undefined
            }
          />
        )}
        {!attempt && phase === "idle" && (
          <RecoveredAttempts lessonId={lessonId} stageId={stageId} />
        )}

        <div className="flex flex-col gap-3">
          {!attempt && (
            <Button
              className="min-h-11 w-full"
              variant={hasRecoverableAttempt ? "outline" : "default"}
              disabled={
                phase !== "idle" ||
                !currentWindow ||
                !ownAttendance ||
                stageAttendance?.status === "passed"
              }
              onClick={() => {
                void startCapture()
              }}
            >
              {captureButtonLabel}
            </Button>
          )}
          {isCapturing && (
            <Button
              variant="outline"
              className="min-h-11 w-full"
              disabled={cancelRequestedRef.current}
              onClick={cancelCapture}
            >
              {cancelRequestedRef.current ? "正在取消…" : "取消采集"}
            </Button>
          )}
        </div>

        {phase === "submitting" && (
          <p role="status" className="text-sm text-muted-foreground">
            正在提交本次采集；按钮已禁用，网络结果不明时请勿重放旧挑战。
          </p>
        )}
      </section>
    </div>
  )
}
