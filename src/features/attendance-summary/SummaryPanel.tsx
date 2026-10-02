import { useSummaryGetLessonAttendance } from "@/api/generated/client"
import { useMe } from "@/features/auth/session"
import {
  Button,
  EmptyState,
  ErrorState,
  LoadingRows,
  StatusBadge,
  names,
} from "@/components/common"
import { liveQuery } from "@/app/providers"
import { useRosterNames } from "./roster-names"

export function SummaryPanel({
  lessonId,
  courseId,
}: {
  lessonId: string
  courseId: string
}) {
  const user = useMe()
  const isTeacher = user.role === "teacher"
  const attendance = useSummaryGetLessonAttendance(lessonId, {
    query: liveQuery,
  })
  const roster = useRosterNames(courseId, isTeacher)

  if (attendance.isLoading)
    return (
      <section aria-labelledby="attendance-summary-title">
        <h2
          id="attendance-summary-title"
          className="mb-4 text-xl font-semibold"
        >
          考勤汇总
        </h2>
        <LoadingRows />
      </section>
    )
  if (attendance.isError || !attendance.data)
    return (
      <section aria-labelledby="attendance-summary-title">
        <h2
          id="attendance-summary-title"
          className="mb-4 text-xl font-semibold"
        >
          考勤汇总
        </h2>
        <ErrorState
          error={attendance.error}
          retry={() => attendance.refetch()}
        />
      </section>
    )

  const rows = isTeacher
    ? attendance.data.students
    : attendance.data.students.filter((student) => student.user_id === user.id)
  if (rows.length === 0)
    return (
      <section aria-labelledby="attendance-summary-title">
        <h2
          id="attendance-summary-title"
          className="mb-4 text-xl font-semibold"
        >
          考勤汇总
        </h2>
        <EmptyState>当前没有可显示的考勤记录。</EmptyState>
      </section>
    )

  return (
    <section aria-labelledby="attendance-summary-title">
      <h2 id="attendance-summary-title" className="mb-4 text-xl font-semibold">
        考勤汇总
      </h2>
      <div className="divide-y border-y">
        {rows.map((student) => {
          const member = isTeacher
            ? roster.data?.get(student.user_id)
            : undefined
          const identity = isTeacher ? (
            member ? (
              <>
                <span>{member.display_name}</span>
                <span className="ml-2 text-sm text-muted-foreground">
                  {member.student_no}
                </span>
              </>
            ) : roster.isError ? (
              <>
                <span>名单映射不可用</span>
                <span className="ml-2 break-all font-mono text-xs text-muted-foreground">
                  {student.user_id}
                </span>
              </>
            ) : roster.data ? (
              <>
                <span>历史名单成员</span>
                <span className="ml-2 break-all font-mono text-xs text-muted-foreground">
                  {student.user_id}
                </span>
              </>
            ) : (
              <span className="text-muted-foreground">正在读取名单…</span>
            )
          ) : null
          const attendanceResult =
            student.attendance_success === true
              ? "已确认"
              : student.attendance_success === false
                ? "未通过"
                : "未定"
          return (
            <article
              key={student.user_id}
              className="grid gap-3 py-5 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-start"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-x-3 gap-y-1 font-medium">
                  {isTeacher ? identity : <span>我的考勤</span>}
                  {student.is_late && <StatusBadge value="获批迟到" />}
                </div>
                <ul
                  className="mt-3 flex flex-wrap gap-2"
                  aria-label="阶段考勤结果"
                >
                  {student.stages.map((stage) => (
                    <li
                      key={stage.stage_id}
                      className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
                    >
                      <span>{names[stage.kind] ?? stage.kind}</span>
                      <StatusBadge value={stage.status} />
                    </li>
                  ))}
                </ul>
                {student.stages.length === 0 && (
                  <p className="mt-3 text-sm text-muted-foreground">
                    本课次尚无已发起阶段。
                  </p>
                )}
                <p className="mt-3 text-sm text-muted-foreground">
                  快照结果：{attendanceResult}
                </p>
              </div>
              <StatusBadge value={student.status} />
            </article>
          )
        })}
      </div>
      {isTeacher && roster.isError && (
        <div
          role="status"
          className="mt-3 flex flex-wrap items-center gap-3 text-sm text-muted-foreground"
        >
          <span>名单映射暂不可用；历史汇总行仍完整保留。</span>
          <Button
            variant="outline"
            className="min-h-11"
            onClick={() => void roster.refetch()}
          >
            重新读取名单
          </Button>
        </div>
      )}
      {isTeacher && roster.isLoading && (
        <p role="status" className="mt-3 text-sm text-muted-foreground">
          正在读取当前名单姓名；不会影响考勤汇总。
        </p>
      )}
      <p className="mt-3 text-xs text-muted-foreground">
        最终考勤结果及阶段记录来自课次冻结快照。
      </p>
    </section>
  )
}
