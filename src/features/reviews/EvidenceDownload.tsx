import { useState } from "react"
import { getReviewsGetEvidenceUrl } from "@/api/generated/client"
import { requestBinary } from "@/api/http"
import { Button, ErrorState } from "@/components/common"

export function EvidenceDownload({ reviewId }: { reviewId: string }) {
  const [pending, setPending] = useState(false)
  const [error, setError] = useState<unknown>(null)
  const download = async () => {
    setPending(true)
    setError(null)
    try {
      const { blob, headers } = await requestBinary(
        getReviewsGetEvidenceUrl(reviewId),
      )
      const mime = (headers.get("Content-Type") ?? "")
        .split(";", 1)[0]
        .trim()
        .toLowerCase()
      const extension =
        mime === "image/jpeg"
          ? ".jpg"
          : mime === "image/png"
            ? ".png"
            : mime === "application/pdf"
              ? ".pdf"
              : null
      if (!extension) throw new Error("附件类型不受支持，无法安全下载。")
      const url = URL.createObjectURL(blob)
      let anchor: HTMLAnchorElement | null = null
      try {
        anchor = document.createElement("a")
        anchor.href = url
        anchor.download = `请假凭据-${reviewId}${extension}`
        anchor.rel = "noreferrer"
        document.body.append(anchor)
        anchor.click()
      } finally {
        anchor?.remove()
        window.setTimeout(() => URL.revokeObjectURL(url), 0)
      }
    } catch (caught) {
      setError(caught)
    } finally {
      setPending(false)
    }
  }
  return (
    <div className="grid justify-items-start gap-1">
      <Button
        className="min-h-11"
        variant="outline"
        disabled={pending}
        onClick={download}
      >
        {pending ? "正在下载…" : "下载凭据"}
      </Button>
      {Boolean(error) && <ErrorState error={error} />}
    </div>
  )
}
