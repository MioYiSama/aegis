import { Button } from "@/components/common"

type Props = {
  aligning: boolean
  countdown: number
  frames: number
  onConfirm: () => void
}

export function FaceCaptureStatus({ aligning, countdown, frames, onConfirm }: Props) {
  return (
    <div className="space-y-3">
      <p role="status" aria-live="polite" className="text-sm font-medium">
        {aligning
          ? "请将完整面部对准画面，光线充足且保持无遮挡。准备好后再开始拍摄。"
          : countdown > 0
            ? `${countdown} 秒后开始拍摄，请正视镜头、保持自然。`
            : `正在拍摄，已完成 ${frames} / 3 张。请保持面部在画面中。`}
      </p>
      {aligning && (
        <Button className="min-h-11 w-full" onClick={onConfirm}>
          我已准备好，开始拍摄
        </Button>
      )}
    </div>
  )
}
