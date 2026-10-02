import { useState, type FormEvent } from "react"
import { useNavigate, useParams, useSearchParams } from "react-router"
import { useQueryClient } from "@tanstack/react-query"
import {
  getCoursesListLessonsQueryKey,
  getCoursesListStudentsQueryKey,
  useCoursesAddStudent,
  useCoursesCreateLesson,
  useCoursesGetCourse,
  useCoursesListLessons,
  useCoursesListStudents,
  useCoursesRemoveStudent,
} from "@/api/generated/client"
import type { CreateLessonRequest } from "@/api/generated/models"
import { ApiRequestError } from "@/api/http"
import { CaretRight } from "@phosphor-icons/react"
import {
  Button,
  EmptyState,
  ErrorState,
  Input,
  Link,
  LoadingRows,
  PageHeader,
  Pagination,
  UrlTabs,
  Confirm,
  formatTime,
} from "@/components/common"
import { useMe } from "@/features/auth/session"

const pageSize = 20

export function CoursePage() {
  const { courseId: courseParam } = useParams()
  const courseId = courseParam ?? ""
  const me = useMe()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [searchParams] = useSearchParams()
  const isTeacher = me.role === "teacher"
  const currentTab =
    searchParams.get("tab") === "roster" && isTeacher ? "roster" : "lessons"
  const [lessonsPage, setLessonsPage] = useState(0)
  const [rosterPage, setRosterPage] = useState(0)
  const [lessonTitle, setLessonTitle] = useState("")
  const [startsAt, setStartsAt] = useState("")
  const [endsAt, setEndsAt] = useState("")
  const [lessonFormError, setLessonFormError] = useState<string | null>(null)
  const [studentNo, setStudentNo] = useState("")
  const [studentFormError, setStudentFormError] = useState<string | null>(null)

  const course = useCoursesGetCourse(courseId, {
    query: { enabled: Boolean(courseId) },
  })
  const lessons = useCoursesListLessons(
    courseId,
    { limit: pageSize, offset: lessonsPage * pageSize },
    { query: { enabled: Boolean(courseId) } },
  )
  const roster = useCoursesListStudents(
    courseId,
    { limit: pageSize, offset: rosterPage * pageSize },
    {
      query: {
        enabled: Boolean(courseId) && isTeacher && currentTab === "roster",
      },
    },
  )
  const createLesson = useCoursesCreateLesson()
  const addStudent = useCoursesAddStudent()
  const removeStudent = useCoursesRemoveStudent()

  async function submitLesson(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const title = lessonTitle.trim()
    const startTimestamp = new Date(startsAt).getTime()
    const endTimestamp = new Date(endsAt).getTime()
    if ([...title].length < 1 || [...title].length > 128) {
      setLessonFormError("课次名称须为 1 至 128 个字符。")
      return
    }
    if (!Number.isFinite(startTimestamp) || !Number.isFinite(endTimestamp)) {
      setLessonFormError("请选择有效的开始和结束时间。")
      return
    }
    if (endTimestamp <= startTimestamp) {
      setLessonFormError("结束时间必须晚于开始时间。")
      return
    }
    const starts_at = new Date(startTimestamp).toISOString()
    const ends_at = new Date(endTimestamp).toISOString()
    setLessonFormError(null)
    const data: CreateLessonRequest = { title, starts_at, ends_at }
    try {
      const created = await createLesson.mutateAsync({ courseId, data })
      await queryClient.invalidateQueries({
        queryKey: getCoursesListLessonsQueryKey(courseId),
      })
      navigate(`/teacher/lessons/${created.id}`)
    } catch {}
  }

  async function submitStudent(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const value = studentNo.trim()
    if ([...value].length < 1 || [...value].length > 64) {
      setStudentFormError("学号须为 1 至 64 个字符。")
      return
    }
    setStudentFormError(null)
    try {
      await addStudent.mutateAsync({ courseId, data: { student_no: value } })
      setStudentNo("")
      await queryClient.invalidateQueries({
        queryKey: getCoursesListStudentsQueryKey(courseId),
      })
    } catch {}
  }

  if (course.isPending)
    return (
      <div className="w-full py-8">
        <LoadingRows />
      </div>
    )
  if (course.error)
    return (
      <div className="w-full py-8">
        <ErrorState error={course.error} retry={() => course.refetch()} />
      </div>
    )
  if (!course.data)
    return (
      <div className="w-full py-8">
        <ErrorState error={new Error("课程不存在或对你不可见。")} />
      </div>
    )

  return (
    <div className="w-full py-8">
      <PageHeader
        title={course.data.title}
        description="查看课程课次与相关信息。"
      />
      {isTeacher && (
        <UrlTabs
          items={[
            { value: "lessons", label: "课次" },
            { value: "roster", label: "名单" },
          ]}
        />
      )}

      {currentTab === "lessons" && (
        <section aria-labelledby="lessons-heading" className="space-y-6">
          <h2 id="lessons-heading">课次</h2>
          {isTeacher && (
            <form
              onSubmit={(event) => {
                void submitLesson(event)
              }}
              className="grid gap-4 border-y py-5 sm:grid-cols-2"
            >
              <h3 className="sm:col-span-2">新增课次</h3>
              <div className="grid gap-2 sm:col-span-2">
                <label htmlFor="lesson-title" className="font-medium">
                  课次名称
                </label>
                <Input
                  id="lesson-title"
                  value={lessonTitle}
                  maxLength={256}
                  onChange={(event) => {
                    setLessonTitle(event.target.value)
                    setLessonFormError(null)
                    createLesson.reset()
                  }}
                  required
                  aria-describedby="lesson-title-help"
                />
                <p
                  id="lesson-title-help"
                  className="text-sm text-muted-foreground"
                >
                  提交时会去除首尾空白，最多 128 个字符。
                </p>
              </div>
              <div className="grid gap-2">
                <label htmlFor="lesson-starts-at" className="font-medium">
                  开始时间
                </label>
                <Input
                  id="lesson-starts-at"
                  type="datetime-local"
                  value={startsAt}
                  onChange={(event) => {
                    setStartsAt(event.target.value)
                    setLessonFormError(null)
                    createLesson.reset()
                  }}
                  required
                  aria-describedby="lesson-time-help"
                />
              </div>
              <div className="grid gap-2">
                <label htmlFor="lesson-ends-at" className="font-medium">
                  结束时间
                </label>
                <Input
                  id="lesson-ends-at"
                  type="datetime-local"
                  value={endsAt}
                  onChange={(event) => {
                    setEndsAt(event.target.value)
                    setLessonFormError(null)
                    createLesson.reset()
                  }}
                  required
                  aria-describedby="lesson-time-help"
                />
              </div>
              <p
                id="lesson-time-help"
                className="text-sm text-muted-foreground sm:col-span-2"
              >
                时间按本地时区选择并转换为 RFC3339
                提交；结束时间必须晚于开始时间。
              </p>
              {lessonFormError && (
                <p
                  role="alert"
                  className="text-sm text-destructive sm:col-span-2"
                >
                  {lessonFormError}
                </p>
              )}
              {createLesson.error && (
                <div className="sm:col-span-2">
                  <ErrorState error={createLesson.error} />
                </div>
              )}
              <Button
                type="submit"
                disabled={createLesson.isPending}
                className="w-full sm:w-fit"
              >
                {createLesson.isPending ? "正在创建…" : "创建课次"}
              </Button>
            </form>
          )}

          {lessons.isPending ? (
            <LoadingRows />
          ) : lessons.error ? (
            <ErrorState error={lessons.error} retry={() => lessons.refetch()} />
          ) : (
            lessons.data && (
              <>
                {lessons.data.items.length === 0 ? (
                  <EmptyState>
                    {isTeacher
                      ? "尚无课次，请创建课次。"
                      : "当前课程尚无课次。"}
                  </EmptyState>
                ) : (
                  <div className="divide-y border-y">
                    {lessons.data.items.map((lesson) => (
                      <Link
                        key={lesson.id}
                        to={`/${me.role}/lessons/${lesson.id}`}
                        className="flex min-h-20 flex-col justify-center gap-1 py-4 sm:flex-row sm:items-center sm:justify-between sm:gap-4"
                      >
                        <span className="min-w-0 font-medium">
                          {lesson.title}
                        </span>
                        <span className="text-sm text-muted-foreground">
                          {formatTime(lesson.starts_at)} –{" "}
                          {formatTime(lesson.ends_at)}
                        </span>
                        <CaretRight size={18} weight="regular" aria-hidden />
                      </Link>
                    ))}
                  </div>
                )}
                <Pagination
                  page={lessonsPage}
                  total={lessons.data.total}
                  onChange={setLessonsPage}
                />
              </>
            )
          )}
        </section>
      )}

      {isTeacher && currentTab === "roster" && (
        <section aria-labelledby="roster-heading" className="space-y-6">
          <div>
            <h2 id="roster-heading">学生名单</h2>
            <p className="mt-2 text-sm text-muted-foreground">
              移除学生不会改变已冻结课次的历史名单。
            </p>
          </div>
          <form
            onSubmit={(event) => {
              void submitStudent(event)
            }}
            className="grid gap-4 border-y py-5 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end"
          >
            <div className="grid gap-2">
              <label htmlFor="student-number" className="font-medium">
                已注册学生学号
              </label>
              <Input
                id="student-number"
                value={studentNo}
                maxLength={128}
                onChange={(event) => {
                  setStudentNo(event.target.value)
                  setStudentFormError(null)
                  addStudent.reset()
                }}
                required
                aria-describedby="student-number-help"
              />
              <p
                id="student-number-help"
                className="text-sm text-muted-foreground"
              >
                输入学生注册时使用的学号，最多 64 个字符。
              </p>
            </div>
            <Button
              type="submit"
              disabled={addStudent.isPending}
              className="w-full sm:w-fit"
            >
              {addStudent.isPending ? "正在添加…" : "添加学生"}
            </Button>
            {studentFormError && (
              <p
                role="alert"
                className="text-sm text-destructive sm:col-span-2"
              >
                {studentFormError}
              </p>
            )}
            {addStudent.error instanceof ApiRequestError &&
            (addStudent.error.status === 404 ||
              addStudent.error.status === 409) ? (
              <p
                role="alert"
                className="text-sm text-destructive sm:col-span-2"
              >
                {addStudent.error.status === 404
                  ? "未找到学生或课程不可见。"
                  : "该学生已在课程名单中。"}
              </p>
            ) : (
              addStudent.error && (
                <div className="sm:col-span-2">
                  <ErrorState error={addStudent.error} />
                </div>
              )
            )}
          </form>
          {roster.isPending ? (
            <LoadingRows />
          ) : roster.error ? (
            <ErrorState error={roster.error} retry={() => roster.refetch()} />
          ) : (
            roster.data && (
              <>
                {roster.data.items.length === 0 ? (
                  <EmptyState>名单为空，请添加已注册学生。</EmptyState>
                ) : (
                  <div className="divide-y border-y">
                    {roster.data.items.map((student) => (
                      <div
                        key={student.user_id}
                        className="flex min-h-20 flex-wrap items-center justify-between gap-3 py-4"
                      >
                        <div className="min-w-0">
                          <p className="font-medium">{student.display_name}</p>
                          <p className="text-sm text-muted-foreground">
                            学号：{student.student_no}
                          </p>
                        </div>
                        <Confirm
                          label="移除"
                          title="从课程名单移除学生？"
                          description={`将移除 ${student.display_name}（${student.student_no}）。已冻结课次的历史名单不会改变。`}
                          action={async () => {
                            await removeStudent.mutateAsync({
                              courseId,
                              userId: student.user_id,
                            })
                            if (
                              roster.data?.items.length === 1 &&
                              rosterPage > 0
                            )
                              setRosterPage(rosterPage - 1)
                            await queryClient.invalidateQueries({
                              queryKey:
                                getCoursesListStudentsQueryKey(courseId),
                            })
                          }}
                        />
                      </div>
                    ))}
                  </div>
                )}
                <Pagination
                  page={rosterPage}
                  total={roster.data.total}
                  onChange={setRosterPage}
                />
              </>
            )
          )}
        </section>
      )}
    </div>
  )
}
