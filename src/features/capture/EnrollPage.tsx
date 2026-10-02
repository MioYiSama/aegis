import { useRef, useState } from "react"
import { useNavigate, useSearchParams } from "react-router"
import {
  attendanceEnrollFace,
  attendanceIssueFaceChallenge,
} from "@/api/generated/client"
import { ApiRequestError } from "@/api/http"
import {
  Button,
  ErrorState,
  Link,
  PageHeader,
  errorMessage,
} from "@/components/common"
import { captureFaceFrames, validateUploadSize } from "./media"
import { useCaptureSession } from "./useCaptureSession"

const ATTENDANCE_RETURN_PATH =
  /^\/student\/lessons\/[^/?#]+\/attend\/[^/?#]+(?:\?.*)?$/

type EnrollmentStep = "idle" | "camera" | "frames" | "submitting" | "enrolled"

export function EnrollPage() {
  const media = useCaptureSession()
  const navigate = useNavigate()
  const [searchParams] = useSearchParams()
  const returnParam = searchParams.get("returnTo")
  const returnPath =
    returnParam && ATTENDANCE_RETURN_PATH.test(returnParam) ? returnParam : null
  const [step, setStep] = useState<EnrollmentStep>("idle")
  const [frameCount, setFrameCount] = useState(0)
  const [error, setError] = useState<Error | null>(null)
  const operationRef = useRef(false)
  const busy = step === "camera" || step === "frames" || step === "submitting"

  async function startEnrollment() {
    if (busy || operationRef.current || step === "enrolled") return
    operationRef.current = true
    setError(null)
    setFrameCount(0)
    const controller = media.beginSession()
    try {
      setStep("camera")
      await media.startCamera("user", controller.signal)
      const challenge = await attendanceIssueFaceChallenge(
        { purpose: "enroll" },
        { signal: controller.signal },
      )
      setStep("frames")
      const frames = await captureFaceFrames(
        media.videoRef.current!,
        controller.signal,
        setFrameCount,
      )
      validateUploadSize(frames)
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
        setStep("enrolled")
      } else {
        setStep("idle")
        setError(new Error("服务器尚未确认登记成功，请重新开始采集"))
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
        setError(new Error("该账号已完成人脸登记，首次登记后不能自行替换。"))
      } else {
        setError(
          cause instanceof Error ? cause : new Error(errorMessage(cause)),
        )
      }
    } finally {
      operationRef.current = false
    }
  }

  function cancelEnrollment() {
    if (step !== "camera" && step !== "frames") return
    media.stopSession()
    setError(new Error("采集已取消。下一次开始会获取新的登记挑战。"))
    setFrameCount(0)
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
              busy
                ? "aspect-video w-full rounded-lg bg-muted object-cover"
                : "hidden"
            }
            autoPlay
            muted
            playsInline
            aria-label="前置相机预览"
          />
          {media.facingMode && (
            <p role="status" className="text-sm text-muted-foreground">
              前置相机已就绪，照片只保留在本页内存中。
            </p>
          )}
          {step === "frames" && (
            <p role="status" className="text-sm text-muted-foreground">
              正在采集第 {frameCount + 1} / 3 张照片。
            </p>
          )}
          {step === "submitting" && (
            <p role="status" className="text-sm text-muted-foreground">
              正在提交登记，请勿关闭页面。
            </p>
          )}
          {error && <ErrorState error={error} />}
        </div>

        <div className="flex flex-wrap gap-3">
          {step !== "enrolled" && (
            <Button
              className="min-h-11 flex-1"
              disabled={busy || operationRef.current}
              onClick={startEnrollment}
            >
              {busy ? "正在采集…" : error ? "重新开始" : "开始登记"}
            </Button>
          )}
          {(step === "camera" || step === "frames") && (
            <Button
              variant="outline"
              className="min-h-11"
              onClick={cancelEnrollment}
            >
              取消
            </Button>
          )}
        </div>

        {step === "enrolled" && (
          <div className="space-y-3">
            <p role="status" className="border-y py-4 font-medium">
              服务器已确认人脸登记成功。
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
          </div>
        )}
      </section>
    </div>
  )
}
