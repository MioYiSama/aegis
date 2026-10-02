const allowed = new Set([
  "payload",
  "challenge_id",
  "reason",
  "frame_0",
  "frame_1",
  "frame_2",
  "qr_image",
  "evidence",
])
export function toApiFormData<Body extends object>(body: Body): FormData {
  const form = new FormData()
  for (const [key, value] of Object.entries(body)) {
    if (value == null || !allowed.has(key)) continue
    if (key === "payload") form.append(key, JSON.stringify(value))
    else if (value instanceof Blob) form.append(key, value)
    else if (key === "challenge_id" || key === "reason")
      form.append(key, String(value))
  }
  return form
}
