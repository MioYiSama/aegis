import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type FormEvent,
} from "react"
import { useParams, useSearchParams } from "react-router"
import { useQueryClient, type QueryClient } from "@tanstack/react-query"
import {
  getCoursesGetLessonQueryKey,
  getCoursesListLessonsQueryKey,
  getReviewsListLessonReviewsQueryKey,
  getSummaryGetLessonAttendanceQueryKey,
  useCoursesGetLesson,
  useCoursesListStudents,
  useStagesCloseLesson,
  useStagesCloseStage,
  useStagesCreateLessonStage,
} from "@/api/generated/client"
import { StageKind as StageKindValue } from "@/api/generated/models"
import type {
  CreateStageRequest,
  LessonDetail,
  Stage,
  StageKind,
} from "@/api/generated/models"
import {
  Button,
  Confirm,
  EmptyState,
  ErrorState,
  Input,
  Link,
  LoadingRows,
  PageHeader,
  formatTime,
  names,
  UrlTabs,
} from "@/components/common"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { useMe } from "@/features/auth/session"
import { liveQuery } from "@/app/providers"
import { RecoveredAttempts } from "@/features/reviews/RecoveredAttempts"
import { ReviewsPanel } from "@/features/reviews/ReviewsPanel"
import { SummaryPanel } from "@/features/attendance-summary/SummaryPanel"
import {
  lessonState,
  stageActive,
  stageDisabledReason,
  submissionWindow,
} from "./window"
import { TeacherLocationMap } from "./TeacherLocationMap"

const stageKinds: StageKind[] = [
  StageKindValue.check_in,
  StageKindValue.renew,
  StageKindValue.check_out,
]

async function refreshLessonViews(
  client: QueryClient,
  lessonId: string,
  courseId: string,
) {
  await Promise.all([
    client.invalidateQueries({
      queryKey: getCoursesGetLessonQueryKey(lessonId),
    }),
    client.invalidateQueries({
      queryKey: getSummaryGetLessonAttendanceQueryKey(lessonId),
    }),
    client.invalidateQueries({
      queryKey: getReviewsListLessonReviewsQueryKey(lessonId),
    }),
    client.invalidateQueries({
      queryKey: getCoursesListLessonsQueryKey(courseId),
    }),
  ])
}

function stageReason(
  detail: LessonDetail,
  kind: StageKind,
  rosterTotal: number,
  rosterPending: boolean,
  rosterError: unknown,
): string | null {
  const reason = stageDisabledReason(detail, kind, rosterTotal)
  if (reason === "名单不能为空，请先添加学生" && rosterPending)
    return "正在读取学生名单"
  if (reason === "名单不能为空，请先添加学生" && rosterError)
    return "名单读取失败，请重试"
  return reason
}

function noSubmissionReason(
  lesson: LessonDetail,
  stage: Stage,
  now: number,
): string {
  if (lessonState(lesson, now) === "waiting") return "等待课次开始"
  if (lessonState(lesson, now) === "ended") return "课次已结束"
  if (now < Date.parse(stage.opens_at)) return "阶段尚未开始"
  if (
    stage.kind === "check_in" &&
    now > Date.parse(lesson.starts_at) + 15 * 60_000
  )
    return "迟到签到期限已过"
  if (stage.closed_at) return "阶段已提前结束"
  return "提交窗口已关闭"
}

export function LessonPage() {
  const { lessonId: lessonParam } = useParams()
  const lessonId = lessonParam ?? ""
  const me = useMe()
  const isTeacher = me.role === "teacher"
  const queryClient = useQueryClient()
  const [searchParams] = useSearchParams()
  const teacherTab =
    searchParams.get("tab") === "reviews" ? "reviews" : "attendance"
  const [now, setNow] = useState(Date.now())
  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), 1000)
    return () => window.clearInterval(timer)
  }, [])
  const [kind, setKind] = useState<StageKind>(StageKindValue.check_in)
  const [duration, setDuration] = useState("")
  const [latitude, setLatitude] = useState("")
  const [longitude, setLongitude] = useState("")
  const [radius, setRadius] = useState("")
  const [locationError, setLocationError] = useState<string | null>(null)
  const [validationError, setValidationError] = useState<string | null>(null)
  const [locating, setLocating] = useState(false)
  const lessonQuery = useCoursesGetLesson(lessonId, {
    query: { ...liveQuery, enabled: Boolean(lessonId) },
  })
  const lesson = lessonQuery.data
  const roster = useCoursesListStudents(
    lesson?.course_id ?? "",
    { limit: 20, offset: 0 },
    {
      query: {
        enabled:
          Boolean(lesson?.course_id) &&
          isTeacher &&
          teacherTab === "attendance",
      },
    },
  )
  const createStage = useStagesCreateLessonStage()
  const createStageResetRef = useRef(createStage.reset)
  createStageResetRef.current = createStage.reset
  const handleMapCoordinatesChange = useCallback(
    (nextLatitude: number, nextLongitude: number) => {
      setLatitude(String(nextLatitude))
      setLongitude(String(nextLongitude))
      setLocationError(null)
      setValidationError(null)
      createStageResetRef.current()
    },
    [],
  )
  const closeStage = useStagesCloseStage()
  const closeLesson = useStagesCloseLesson()

  function updateStageField(update: () => void) {
    update()
    setValidationError(null)
    createStage.reset()
  }

  function useCurrentLocation() {
    setLocationError(null)
    if (!navigator.geolocation) {
      setLocationError("此浏览器不支持定位，请手动填写 WGS84 坐标。")
      return
    }
    setLocating(true)
    navigator.geolocation.getCurrentPosition(
      (position) => {
        setLatitude(String(position.coords.latitude))
        setLongitude(String(position.coords.longitude))
        setLocationError(null)
        setValidationError(null)
        createStage.reset()
        setLocating(false)
      },
      (error) => {
        setLocationError(
          error.code === 1
            ? "定位权限被拒绝，请手动填写 WGS84 坐标。"
            : error.code === 2
              ? "当前位置不可用，请手动填写 WGS84 坐标。"
              : "定位超时，请手动填写 WGS84 坐标。",
        )
        setLocating(false)
      },
      { enableHighAccuracy: true, maximumAge: 0, timeout: 10_000 },
    )
  }

  async function submitStage(
    event: FormEvent<HTMLFormElement>,
    detail: LessonDetail,
  ) {
    event.preventDefault()
    const reason = stageReason(
      detail,
      kind,
      roster.data?.total ?? 0,
      roster.isPending,
      roster.error,
    )
    if (reason) {
      setValidationError(reason)
      return
    }
    const durationValue = Number(duration)
    const latitudeValue = Number(latitude)
    const longitudeValue = Number(longitude)
    const radiusValue = Number(radius)
    if (
      !Number.isInteger(durationValue) ||
      durationValue < 1 ||
      durationValue > 900
    ) {
      setValidationError("阶段时长须为 1 至 900 秒的整数。")
      return
    }
    if (
      !Number.isFinite(latitudeValue) ||
      latitudeValue < -90 ||
      latitudeValue > 90
    ) {
      setValidationError("纬度须在 -90 至 90 之间。")
      return
    }
    if (
      !Number.isFinite(longitudeValue) ||
      longitudeValue < -180 ||
      longitudeValue > 180
    ) {
      setValidationError("经度须在 -180 至 180 之间。")
      return
    }
    if (
      !Number.isFinite(radiusValue) ||
      radiusValue < 1 ||
      radiusValue > 1000
    ) {
      setValidationError("定位半径须在 1 至 1000 米之间。")
      return
    }
    setValidationError(null)
    const data: CreateStageRequest = {
      kind,
      duration_seconds: durationValue,
      latitude: latitudeValue,
      longitude: longitudeValue,
      radius_m: radiusValue,
    }
    try {
      await createStage.mutateAsync({ id: detail.id, data })
      await refreshLessonViews(queryClient, detail.id, detail.course_id)
      setDuration("")
      setLatitude("")
      setLongitude("")
      setRadius("")
    } catch {}
  }

  if (lessonQuery.isPending)
    return (
      <div className="w-full py-8">
        <LoadingRows />
      </div>
    )
  if (lessonQuery.error)
    return (
      <div className="w-full py-8">
        <ErrorState
          error={lessonQuery.error}
          retry={() => lessonQuery.refetch()}
        />
      </div>
    )
  if (!lesson)
    return (
      <div className="w-full py-8">
        <ErrorState error={new Error("课次不存在或对你不可见。")} />
      </div>
    )

  const hasCheckIn = lesson.stages.some((stage) => stage.kind === "check_in")
  const rosterTotal = roster.data?.total ?? 0
  const selectedStageReason = stageReason(
    lesson,
    kind,
    rosterTotal,
    roster.isPending,
    roster.error,
  )
  const stageFormDisabled =
    Boolean(selectedStageReason) || createStage.isPending
  const lateCutoff = new Date(Date.parse(lesson.starts_at) + 15 * 60_000)
  const section = teacherTab

  return (
    <div className="w-full py-8">
      <PageHeader
        title={lesson.title}
        description={`${formatTime(lesson.starts_at)} – ${formatTime(lesson.ends_at)}`}
        action={
          <Link
            className="min-h-11 inline-flex items-center py-2 font-medium underline underline-offset-4"
            to={`/${me.role}/courses/${lesson.course_id}`}
          >
            返回课程
          </Link>
        }
      />
      {isTeacher && (
        <UrlTabs
          items={[
            { value: "attendance", label: "考勤" },
            { value: "reviews", label: "审核" },
          ]}
        />
      )}
      {lessonState(lesson, now) === "waiting" && (
        <p role="status" className="mb-6 text-muted-foreground">
          等待课次开始
        </p>
      )}
      {lessonState(lesson, now) === "ended" && (
        <p role="status" className="mb-6 text-muted-foreground">
          课次已结束 · 以下为服务端汇总
        </p>
      )}

      {(!isTeacher || section === "attendance") && (
        <div className="space-y-10">
          <section aria-labelledby="stages-heading" className="space-y-5">
            <div>
              <h2 id="stages-heading">考勤阶段</h2>
              <p className="mt-2 text-sm text-muted-foreground">
                仅显示服务端创建的阶段；阶段窗口与课次状态以服务器响应为准。
              </p>
            </div>
            {lesson.stages.length === 0 ? (
              <EmptyState>尚未发起考勤阶段。</EmptyState>
            ) : (
              <ol className="divide-y border-y">
                {lesson.stages.map((stage) => {
                  const submission = submissionWindow(lesson, stage, now)
                  const canProject =
                    isTeacher &&
                    stage.kind === "check_in" &&
                    stageActive(lesson, stage, now)
                  const lateDeadline = stage.kind === "check_in"
                  return (
                    <li
                      key={stage.id}
                      className="grid gap-3 py-5 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-center"
                    >
                      <div className="min-w-0 space-y-1">
                        <div className="flex flex-wrap items-center gap-2">
                          <h3>
                            {names[stage.kind] ?? stage.kind} · 第{" "}
                            {stage.ordinal + 1} 阶段
                          </h3>
                          {stage.closed_at && (
                            <span className="text-sm text-muted-foreground">
                              已关闭
                            </span>
                          )}
                        </div>
                        <p className="text-sm text-muted-foreground">
                          开放：{formatTime(stage.opens_at)} · 关闭：
                          {formatTime(stage.closes_at)}
                        </p>
                        <p className="text-sm text-muted-foreground">
                          范围：{stage.latitude}, {stage.longitude} · 半径{" "}
                          {stage.radius_m} 米
                        </p>
                        {lateDeadline && (
                          <p className="text-sm text-muted-foreground">
                            迟到签到截止：{formatTime(lateCutoff.toISOString())}
                          </p>
                        )}
                        {!isTeacher && submission === "late" && (
                          <p className="text-sm font-medium">
                            此阶段已进入迟到签到时间窗。
                          </p>
                        )}
                      </div>
                      <div className="flex flex-wrap items-center gap-2 sm:justify-end">
                        {!isTeacher && submission && (
                          <Link
                            className="inline-flex min-h-11 items-center justify-center rounded-full bg-primary px-5 py-2 font-medium text-primary-foreground"
                            to={`/student/lessons/${lesson.id}/attend/${stage.id}`}
                          >
                            {submission === "late" ? "迟到签到" : "开始考勤"}
                          </Link>
                        )}
                        {!isTeacher && !submission && (
                          <span className="text-sm text-muted-foreground">
                            {noSubmissionReason(lesson, stage, now)}
                          </span>
                        )}
                        {canProject && (
                          <Link
                            className="inline-flex min-h-11 items-center justify-center rounded-full border px-5 py-2 font-medium"
                            to={`/teacher/lessons/${lesson.id}/project/${stage.id}`}
                          >
                            投屏
                          </Link>
                        )}
                        {isTeacher && stageActive(lesson, stage, now) && (
                          <Confirm
                            label="提前关闭"
                            title="提前关闭考勤阶段？"
                            description="关闭后学生不能再按常规窗口提交；签到阶段仍可能在课次开始后 15 分钟内接受迟到考勤。"
                            action={async () => {
                              await closeStage.mutateAsync({ id: stage.id })
                              await refreshLessonViews(
                                queryClient,
                                lesson.id,
                                lesson.course_id,
                              )
                            }}
                          />
                        )}
                      </div>
                    </li>
                  )
                })}
              </ol>
            )}

            {isTeacher && lessonState(lesson, now) !== "ended" && (
              <form
                onSubmit={(event) => {
                  void submitStage(event, lesson)
                }}
                className="grid gap-4 border-y py-5 sm:grid-cols-2"
              >
                <h3 className="sm:col-span-2">发起阶段</h3>
                <div className="grid gap-2 sm:col-span-2">
                  <label htmlFor="stage-kind" className="font-medium">
                    阶段类型
                  </label>
                  <Select
                    value={kind}
                    items={stageKinds.map((value) => ({
                      value,
                      label: names[value],
                    }))}
                    onValueChange={(value) => {
                      if (value) {
                        updateStageField(() => setKind(value as StageKind))
                      }
                    }}
                  >
                    <SelectTrigger
                      id="stage-kind"
                      type="button"
                      className="w-full"
                      aria-describedby="stage-kind-help"
                    >
                      <SelectValue placeholder="选择阶段类型" />
                    </SelectTrigger>
                    <SelectContent>
                      {stageKinds.map((stageKind) => {
                        const reason = stageReason(
                          lesson,
                          stageKind,
                          rosterTotal,
                          roster.isPending,
                          roster.error,
                        )
                        return (
                          <SelectItem
                            key={stageKind}
                            value={stageKind}
                            disabled={Boolean(reason)}
                          >
                            {names[stageKind] ?? stageKind}
                            {reason ? `（${reason}）` : ""}
                          </SelectItem>
                        )
                      })}
                    </SelectContent>
                  </Select>
                  <p
                    id="stage-kind-help"
                    className="text-sm text-muted-foreground"
                  >
                    签到只能发起一次；续签和签退必须符合当前阶段顺序。
                  </p>
                </div>
                <div className="grid gap-2">
                  <label htmlFor="stage-duration" className="font-medium">
                    窗口时长（秒）
                  </label>
                  <Input
                    id="stage-duration"
                    type="number"
                    min={1}
                    max={900}
                    step={1}
                    value={duration}
                    onChange={(event) =>
                      updateStageField(() => setDuration(event.target.value))
                    }
                    required
                    aria-describedby="stage-duration-help"
                  />
                  <p
                    id="stage-duration-help"
                    className="text-sm text-muted-foreground"
                  >
                    填写 1 至 900 秒的整数。
                  </p>
                </div>
                <div className="grid gap-2">
                  <label htmlFor="stage-radius" className="font-medium">
                    定位半径（米）
                  </label>
                  <Input
                    id="stage-radius"
                    type="number"
                    min={1}
                    max={1000}
                    step="any"
                    value={radius}
                    onChange={(event) =>
                      updateStageField(() => setRadius(event.target.value))
                    }
                    required
                    aria-describedby="stage-radius-help"
                  />
                  <p
                    id="stage-radius-help"
                    className="text-sm text-muted-foreground"
                  >
                    填写 1 至 1000 米。
                  </p>
                </div>
                <div className="grid gap-2">
                  <label htmlFor="stage-latitude" className="font-medium">
                    纬度
                  </label>
                  <Input
                    id="stage-latitude"
                    type="number"
                    min={-90}
                    max={90}
                    step="any"
                    value={latitude}
                    onChange={(event) =>
                      updateStageField(() => setLatitude(event.target.value))
                    }
                    required
                    aria-describedby="stage-coordinate-help"
                  />
                </div>
                <div className="grid gap-2">
                  <label htmlFor="stage-longitude" className="font-medium">
                    经度
                  </label>
                  <Input
                    id="stage-longitude"
                    type="number"
                    min={-180}
                    max={180}
                    step="any"
                    value={longitude}
                    onChange={(event) =>
                      updateStageField(() => setLongitude(event.target.value))
                    }
                    required
                    aria-describedby="stage-coordinate-help"
                  />
                </div>
                <p
                  id="stage-coordinate-help"
                  className="text-sm text-muted-foreground sm:col-span-2"
                >
                  手动输入 WGS84 坐标：纬度 -90 至 90，经度 -180 至 180。
                </p>
                <div className="flex flex-wrap items-center gap-3 sm:col-span-2">
                  <Button
                    type="button"
                    variant="outline"
                    disabled={locating}
                    onClick={useCurrentLocation}
                  >
                    {locating ? "正在定位…" : "使用当前位置"}
                  </Button>
                  {locationError && (
                    <p role="alert" className="text-sm text-destructive">
                      {locationError}
                    </p>
                  )}
                </div>
                <TeacherLocationMap
                  latitude={latitude}
                  longitude={longitude}
                  radius={radius}
                  onCoordinatesChange={handleMapCoordinatesChange}
                />
                {selectedStageReason && (
                  <p className="text-sm text-muted-foreground sm:col-span-2">
                    {selectedStageReason}。
                  </p>
                )}
                {validationError && (
                  <p
                    role="alert"
                    className="text-sm text-destructive sm:col-span-2"
                  >
                    {validationError}
                  </p>
                )}
                {createStage.error && (
                  <div className="sm:col-span-2">
                    <ErrorState error={createStage.error} />
                  </div>
                )}
                {isTeacher && lesson.stages.length === 0 && roster.error && (
                  <div className="sm:col-span-2">
                    <ErrorState
                      error={roster.error}
                      retry={() => roster.refetch()}
                    />
                  </div>
                )}
                <Button
                  type="submit"
                  disabled={stageFormDisabled}
                  className="w-full sm:w-fit"
                >
                  {createStage.isPending ? "正在发起…" : "发起阶段"}
                </Button>
              </form>
            )}

            {isTeacher && hasCheckIn && !lesson.closed_at && (
              <div className="flex flex-wrap items-center gap-3 border-t pt-5">
                <Confirm
                  label="结束课次"
                  title="结束这次课？"
                  description="课次结束后将不能再发起考勤阶段。"
                  action={async () => {
                    await closeLesson.mutateAsync({ id: lesson.id })
                    await refreshLessonViews(
                      queryClient,
                      lesson.id,
                      lesson.course_id,
                    )
                  }}
                />
              </div>
            )}
            {isTeacher && !hasCheckIn && !lesson.closed_at && (
              <p className="text-sm text-muted-foreground">
                发起签到阶段后才能结束课次。
              </p>
            )}
          </section>
          {isTeacher ? (
            <SummaryPanel lessonId={lesson.id} courseId={lesson.course_id} />
          ) : (
            <>
              <section
                aria-labelledby="leave-heading"
                className="flex flex-wrap items-center justify-between gap-3 border-y py-5"
              >
                <div>
                  <h2 id="leave-heading">请假申请</h2>
                  <p className="mt-1 text-sm text-muted-foreground">
                    查看本人请假与审核状态。
                  </p>
                </div>
                <Link
                  className="inline-flex min-h-11 items-center justify-center rounded-full border px-5 py-2 font-medium"
                  to={`/student/lessons/${lesson.id}/leave`}
                >
                  申请请假
                </Link>
              </section>
              <RecoveredAttempts lessonId={lesson.id} />
              <SummaryPanel lessonId={lesson.id} courseId={lesson.course_id} />
              <ReviewsPanel lessonId={lesson.id} courseId={lesson.course_id} />
            </>
          )}
        </div>
      )}
      {isTeacher && section === "reviews" && (
        <ReviewsPanel lessonId={lesson.id} courseId={lesson.course_id} />
      )}
    </div>
  )
}
