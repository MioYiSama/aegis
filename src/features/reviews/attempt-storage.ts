import type {
  AttendanceAttempt,
  AttemptFactors,
  ReasonCode,
} from "@/api/generated/models"

const storagePrefix = "aegis:reviewable:"
const storageEvent = "aegis:reviewable-change"
export const reviewableStorageEvent = storageEvent

export type ReviewableReceipt = AttendanceAttempt & { lesson_id: string }
type StoredAttempt = Pick<
  ReviewableReceipt,
  | "id"
  | "lesson_id"
  | "stage_id"
  | "submitted_at"
  | "is_late"
  | "factors"
  | "reason_codes"
>

function isStoredAttempt(value: unknown): value is StoredAttempt {
  if (!value || typeof value !== "object") return false
  const item = value as Record<string, unknown>
  const factors = item.factors
  if (!factors || typeof factors !== "object") return false
  const factorValues = factors as Record<string, unknown>
  const validQr =
    factorValues.qr === undefined ||
    factorValues.qr === null ||
    typeof factorValues.qr === "boolean"
  return (
    typeof item.id === "string" &&
    typeof item.lesson_id === "string" &&
    typeof item.stage_id === "string" &&
    typeof item.submitted_at === "string" &&
    typeof item.is_late === "boolean" &&
    Array.isArray(item.reason_codes) &&
    item.reason_codes.every((code) => typeof code === "string") &&
    typeof factorValues.face === "boolean" &&
    typeof factorValues.location === "boolean" &&
    validQr
  )
}

export function saveReviewable(
  userId: string,
  attempt: AttendanceAttempt,
  lessonId: string,
): boolean {
  if (attempt.outcome !== "reviewable" || attempt.review_id) return false
  try {
    const previous = readReviewable(userId).filter(
      (item) => item.id !== attempt.id,
    )
    const stored: StoredAttempt = {
      id: attempt.id,
      lesson_id: lessonId,
      stage_id: attempt.stage_id,
      submitted_at: attempt.submitted_at,
      is_late: attempt.is_late,
      factors: {
        face: attempt.factors.face,
        location: attempt.factors.location,
        qr: attempt.factors.qr,
      },
      reason_codes: [...attempt.reason_codes],
    }
    sessionStorage.setItem(
      `${storagePrefix}${userId}`,
      JSON.stringify([...previous, stored]),
    )
    if (typeof window !== "undefined")
      window.dispatchEvent(new Event(storageEvent))
    return true
  } catch {
    return false
  }
}

export function readReviewable(userId: string): ReviewableReceipt[] {
  try {
    const data = sessionStorage.getItem(`${storagePrefix}${userId}`)
    if (!data) return []
    const parsed: unknown = JSON.parse(data)
    if (!Array.isArray(parsed)) return []
    return parsed.filter(isStoredAttempt).map((item) => ({
      id: item.id,
      lesson_id: item.lesson_id,
      stage_id: item.stage_id,
      submitted_at: item.submitted_at,
      is_late: item.is_late,
      factors: {
        face: item.factors.face,
        location: item.factors.location,
        qr: item.factors.qr,
      },
      reason_codes: [...item.reason_codes],
      outcome: "reviewable",
    }))
  } catch {
    return []
  }
}

export function removeReviewable(userId: string, attemptId: string): void {
  try {
    const remaining = readReviewable(userId).filter(
      (item) => item.id !== attemptId,
    )
    if (remaining.length) {
      const stored: StoredAttempt[] = remaining.map((item) => ({
        id: item.id,
        lesson_id: item.lesson_id,
        stage_id: item.stage_id,
        submitted_at: item.submitted_at,
        is_late: item.is_late,
        factors: {
          face: item.factors.face,
          location: item.factors.location,
          qr: item.factors.qr,
        },
        reason_codes: [...item.reason_codes],
      }))
      sessionStorage.setItem(
        `${storagePrefix}${userId}`,
        JSON.stringify(stored),
      )
    } else sessionStorage.removeItem(`${storagePrefix}${userId}`)
    if (typeof window !== "undefined")
      window.dispatchEvent(new Event(storageEvent))
  } catch {}
}
