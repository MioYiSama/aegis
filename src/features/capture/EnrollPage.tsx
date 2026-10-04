import { useRef, useState } from "react"
import { useNavigate, useSearchParams } from "react-router"
import { useQueryClient } from "@tanstack/react-query"
import {
  attendanceEnrollFace,
  attendanceIssueFaceChallenge,
  getAttendanceGetFaceEnrollmentQueryKey,
  useAttendanceGetFaceEnrollment,
} from "@/api/generated/client"
import { ApiRequestError } from "@/api/http"
import {
  Button,
  ErrorState,
  Link,
  PageHeader,
  LoadingRows,
  errorMessage,
} from "@/components/common"
import { validateUploadSize } from "./media"
import { useCaptureSession } from "./useCaptureSession"
import { useFaceCapture } from "./useFaceCapture"
import { FaceCaptureStatus } from "./FaceCaptureStatus"

const ATTENDANCE_RETURN_PATH =
  /^\/student\/lessons\/[^/?#]+\/attend\/[^/?#]+(?:\?.*)?$/

type EnrollmentStep = "idle" | "camera" | "aligning" | "frames" | "submitting" | "enrolled"

export function EnrollPage() {
  const media = useCaptureSession()
  const face = useFaceCapture()
  const queryClient = useQueryClient()
  const enrollmentQuery = useAttendanceGetFaceEnrollment({
    query: { staleTime: 0 },
    request: { cache: "no-store" },
  })
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const returnParam = searchParams.get("returnTo")
  const returnPath =
    returnParam && ATTENDANCE_RETURN_PATH.test(returnParam) ? returnParam : null
  const [step, setStep] = useState<EnrollmentStep>("idle")
  const [error, setError] = useState<Error | null>(null)
  const operationRef = useRef(false)
  const busy = step === "camera" || step === "aligning" || step === "frames" || step === "submitting"
  const enrolled = step === "enrolled" || enrollmentQuery.data?.enrolled === true
  const canStart =
    enrollmentQuery.isSuccess &&
    !enrollmentQuery.isFetching &&
    enrollmentQuery.data.enrolled === false &&
    !enrolled

  async function startEnrollment() {
    if (busy || operationRef.current || !canStart) return
    operationRef.current = true
    setError(null)
    face.reset()
    const controller = media.beginSession()
    try {
      setStep("camera")
      await media.startCamera("user", controller.signal)
      setStep("aligning")
      await face.prepare(controller.signal)
      const challenge = await attendanceIssueFaceChallenge(
        { purpose: "enroll" },
        { signal: controller.signal },
      )
      setStep("frames")
      const frames = await face.capture(media.videoRef.current!, controller.signal)
      validateUploadSize(frames)
      media.stopCamera()
      setStep("submitting")
      const receipt = await attendanceEnrollFace(
        {
          challenge_id: challenge.id,
          frame_0: frames[0],
          frame_1: frames[1],
          frame_2: frames[2],
        },
        { signal: controller.signal },
      )
      media.stopSession()
      if (receipt.enrolled) {
        await queryClient.cancelQueries({ queryKey: getAttendanceGetFaceEnrollmentQueryKey() })
        queryClient.setQueryData(getAttendanceGetFaceEnrollmentQueryKey(), receipt)
        setStep("enrolled")
      } else {
        setStep("idle")
        setError(new Error("人脸登记未成功：服务器尚未确认登记，请重新开始采集。"))
      }
    } catch (cause) {
      media.stopSession()
      setStep("idle")
      if (cause instanceof DOMException && cause.name === "AbortError") return
      if (
        cause instanceof ApiRequestError &&
        cause.status === 409 &&
        cause.message.includes("Face is already enrolled")
      ) {
        await queryClient.cancelQueries({ queryKey: getAttendanceGetFaceEnrollmentQueryKey() })
        queryClient.setQueryData(getAttendanceGetFaceEnrollmentQueryKey(), { enrolled: true })
        setStep("enrolled")
      } else {
        setError(
          new Error(`人脸登记未成功：${errorMessage(cause)}。请根据提示处理后重新开始。`),
        )
      }
    } finally {
      operationRef.current = false
    }
  }

  function cancelEnrollment() {
    if (step !== "camera" && step !== "aligning" && step !== "frames") return
    media.stopSession()
    setError(new Error("采集已取消。下一次开始会获取新的登记挑战。"))
    face.reset()
    setStep("idle")
  }

  return (
    <div className="space-y-5">
      <div className="mb-6">
        <Link to={returnPath ?? "/student/account"}>
          {returnPath ? "返回当前考勤" : "返回我的"}
        </Link>
      </div>
      <PageHeader title="人脸登记" description="仅首次登记，不能自助替换。" />
      {enrollmentQuery.isPending ? (
        <LoadingRows />
      ) : enrollmentQuery.error ? (
        <ErrorState
          error={new Error(`无法查询人脸登记状态：${errorMessage(enrollmentQuery.error)}`)}
          retry={() => { void enrollmentQuery.refetch() }}
        />
      ) : !enrolled && !busy ? (
        <p role="status" className="rounded-lg border p-4 text-sm">
          尚未完成人脸登记，请先采集并等待服务器确认。
        </p>
      ) : null}
      <section className="space-y-5">
        <div className="space-y-2 border-y py-4 text-sm leading-6 text-muted-foreground">
          <p>
            系统不核验初次登记者的学籍身份；请先确认你使用的是本人账号。登记后没有自助替换人脸的入口。
          </p>
          <p>
            相机只会在你开始后启动。三张独立时刻照片仅用于本次登记请求，不保存在浏览器；服务器不会向页面返回人脸模板或模型分数。
          </p>
        </div>

        <div className="space-y-3">
          <video
            ref={media.videoRef}
            className={
              step === "camera" || step === "aligning" || step === "frames"
                ? "mx-auto aspect-[9/16] w-full max-w-72 rounded-lg bg-black object-cover object-center"
                : "hidden"
            }
            autoPlay
            muted
            playsInline
            aria-label="前置相机预览"
          />
          {(step === "aligning" || step === "frames") && (
            <FaceCaptureStatus
              aligning={face.aligning}
              countdown={face.countdown}
              frames={face.frames}
              onConfirm={face.confirm}
            />
          )}
          {step === "submitting" && (
            <p role="status" className="rounded-lg border p-4 text-sm font-medium">
              3 张照片已采集完成，正在等待服务器验证。此时尚未登记成功，请勿关闭页面。
            </p>
          )}
          {error && <ErrorState error={error} />}
        </div>

        <div className="flex flex-wrap gap-3">
          {!enrolled && enrollmentQuery.isSuccess && (
            <Button
              className="min-h-11 flex-1"
              disabled={busy || !canStart}
              onClick={startEnrollment}
            >
              {step === "submitting" ? "正在验证…" : busy ? "采集中，请按提示操作" : error ? "重新开始" : "开始登记"}
            </Button>
          )}
          {(step === "camera" || step === "aligning" || step === "frames") && (
            <Button
              variant="outline"
              className="min-h-11"
              onClick={cancelEnrollment}
            >
              取消
            </Button>
          )}
        </div>

        {enrolled && (
          <div className="space-y-3 rounded-lg border p-4">
            <p role="status" className="font-medium">
              {step === "enrolled" ? "人脸登记成功，服务器已确认保存。" : "已完成人脸登记。"}
            </p>
            <p className="text-sm text-muted-foreground">
              登记已完成，无需再次采集。你可以返回继续使用考勤。
            </p>
            {returnPath && (
              <Button
                variant="outline"
                className="min-h-11 w-full"
                onClick={() => navigate(returnPath, { replace: true })}
              >
                返回继续签到
              </Button>
            )}
            {!returnPath && (
              <Button
                variant="outline"
                className="min-h-11 w-full"
                onClick={() => navigate("/student/account", { replace: true })}
              >
                完成，返回我的
              </Button>
            )}
          </div>
        )}
      </section>
    </div>
  )
}
