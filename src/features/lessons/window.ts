import type {
  Lesson,
  LessonDetail,
  Stage,
  StageKind,
} from "@/api/generated/models"
export function lessonState(
  lesson: Lesson,
  now = Date.now(),
): "waiting" | "active" | "ended" {
  if (lesson.closed_at || now >= Date.parse(lesson.ends_at)) return "ended"
  return now < Date.parse(lesson.starts_at) ? "waiting" : "active"
}
export function stageActive(lesson: Lesson, stage: Stage, now = Date.now()) {
  return (
    lessonState(lesson, now) === "active" &&
    !stage.closed_at &&
    now >= Date.parse(stage.opens_at) &&
    now < Date.parse(stage.closes_at)
  )
}
export function submissionWindow(
  lesson: Lesson,
  stage: Stage,
  now = Date.now(),
): "regular" | "late" | null {
  if (lessonState(lesson, now) !== "active") return null
  if (stageActive(lesson, stage, now)) return "regular"
  const closes = Date.parse(stage.closes_at)
  if (
    stage.kind === "check_in" &&
    now >= closes &&
    now <= Date.parse(lesson.starts_at) + 15 * 60_000
  )
    return "late"
  return null
}
export function stageDisabledReason(
  lesson: LessonDetail,
  kind: StageKind,
  rosterTotal: number,
  now = Date.now(),
): string | null {
  const state = lessonState(lesson, now)
  if (state === "waiting") return "等待课次开始"
  if (state === "ended") return "课次已结束"
  if (lesson.stages.some((stage) => stageActive(lesson, stage, now)))
    return "请先结束当前考勤窗口"
  if (lesson.stages.some((stage) => stage.kind === "check_out"))
    return "签退后不能开启新阶段"
  const checked = lesson.stages.some((stage) => stage.kind === "check_in")
  if (!checked && kind !== "check_in") return "首个阶段必须是签到"
  if (checked && kind === "check_in") return "签到只能发起一次"
  if (!checked && rosterTotal === 0) return "名单不能为空，请先添加学生"
  return null
}
