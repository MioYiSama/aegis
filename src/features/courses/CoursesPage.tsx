import { useState, type FormEvent } from "react"
import { useNavigate } from "react-router"
import { useQueryClient } from "@tanstack/react-query"
import {
  getCoursesListCoursesQueryKey,
  useCoursesCreateCourse,
  useCoursesListCourses,
} from "@/api/generated/client"
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
} from "@/components/common"
import { useMe } from "@/features/auth/session"

export function CoursesPage() {
  const me = useMe()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [page, setPage] = useState(0)
  const [title, setTitle] = useState("")
  const [validationError, setValidationError] = useState<string | null>(null)
  const courses = useCoursesListCourses({ limit: 20, offset: page * 20 })
  const createCourse = useCoursesCreateCourse()

  async function submitCourse(event: FormEvent<HTMLFormElement>) {
    event.preventDefault()
    const normalized = title.trim()
    if ([...normalized].length < 1 || [...normalized].length > 128) {
      setValidationError("课程名称须为 1 至 128 个字符。")
      return
    }
    setValidationError(null)
    try {
      const course = await createCourse.mutateAsync({
        data: { title: normalized },
      })
      await queryClient.invalidateQueries({
        queryKey: getCoursesListCoursesQueryKey(),
      })
      navigate(`/teacher/courses/${course.id}`)
    } catch {}
  }

  return (
    <div className="w-full py-8">
      <PageHeader
        title="课程"
        description={
          me.role === "teacher"
            ? "管理课程、课次与学生名单。"
            : "由教师加入课程后，在这里查看课次。"
        }
      />
      {me.role === "teacher" && (
        <form
          onSubmit={(event) => {
            void submitCourse(event)
          }}
          className="mb-8 grid gap-4 border-y py-5 sm:grid-cols-[minmax(0,1fr)_auto] sm:items-end"
        >
          <div className="grid gap-2">
            <label htmlFor="course-title" className="font-medium">
              课程名称
            </label>
            <Input
              id="course-title"
              value={title}
              maxLength={256}
              onChange={(event) => {
                setTitle(event.target.value)
                setValidationError(null)
                createCourse.reset()
              }}
              required
              aria-describedby="course-title-help"
            />
            <p id="course-title-help" className="text-sm text-muted-foreground">
              首尾空白会在提交前移除，名称最多 128 个字符。
            </p>
          </div>
          <Button
            type="submit"
            disabled={createCourse.isPending}
            className="w-full sm:w-fit"
          >
            {createCourse.isPending ? "正在创建…" : "创建课程"}
          </Button>
          {validationError && (
            <p role="alert" className="text-sm text-destructive sm:col-span-2">
              {validationError}
            </p>
          )}
          {createCourse.error && (
            <div className="sm:col-span-2">
              <ErrorState error={createCourse.error} />
            </div>
          )}
        </form>
      )}
      {courses.isPending ? (
        <LoadingRows />
      ) : courses.error ? (
        <ErrorState error={courses.error} retry={() => courses.refetch()} />
      ) : (
        courses.data && (
          <>
            {courses.data.items.length === 0 ? (
              <EmptyState>
                {me.role === "teacher"
                  ? "尚无课程，请创建课程。"
                  : "尚无课程，请联系教师添加你的学号。"}
              </EmptyState>
            ) : (
              <div className="divide-y border-y">
                {courses.data.items.map((course) => (
                  <Link
                    key={course.id}
                    className="flex min-h-20 items-center justify-between gap-4 py-5"
                    to={`/${me.role}/courses/${course.id}`}
                  >
                    <span className="min-w-0 font-medium">{course.title}</span>
                    <CaretRight size={18} weight="regular" aria-hidden />
                  </Link>
                ))}
              </div>
            )}
            <Pagination
              page={page}
              total={courses.data.total}
              onChange={setPage}
            />
          </>
        )
      )}
    </div>
  )
}
