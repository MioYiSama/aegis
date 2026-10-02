import { createBrowserRouter, Navigate, RouterProvider } from "react-router"
import { useAuthMe } from "@/api/generated/client"
import { AuthPage } from "@/features/auth/AuthPage"
import { AccountPage } from "@/features/auth/AccountPage"
import { CoursesPage } from "@/features/courses/CoursesPage"
import { CoursePage } from "@/features/courses/CoursePage"
import { LessonPage } from "@/features/lessons/LessonPage"
import { ProjectionPage } from "@/features/projection/ProjectionPage"
import { EnrollPage } from "@/features/capture/EnrollPage"
import { AttendPage } from "@/features/capture/AttendPage"
import { LeavePage } from "@/features/reviews/LeavePage"
import { ProtectedLayout } from "./layout"
import { ApiRequestError } from "@/api/http"
import { ErrorState, LoadingRows } from "@/components/common"
function Home() {
  const me = useAuthMe()
  if (me.isPending) return <LoadingRows />
  if (me.data) return <Navigate to={"/" + me.data.role + "/courses"} replace />
  if (me.error instanceof ApiRequestError && me.error.status === 401)
    return <Navigate to="/login" replace />
  return <ErrorState error={me.error} retry={() => me.refetch()} />
}
export const router = createBrowserRouter([
  { path: "/", element: <Home /> },
  { path: "/login", element: <AuthPage /> },
  { path: "/register", element: <AuthPage register /> },
  {
    path: "/teacher",
    element: <ProtectedLayout role="teacher" />,
    children: [
      { path: "courses", element: <CoursesPage /> },
      { path: "courses/:courseId", element: <CoursePage /> },
      { path: "lessons/:lessonId", element: <LessonPage /> },
      {
        path: "lessons/:lessonId/project/:stageId",
        element: <ProjectionPage />,
      },
    ],
  },
  {
    path: "/student",
    element: <ProtectedLayout role="student" />,
    children: [
      { path: "courses", element: <CoursesPage /> },
      { path: "courses/:courseId", element: <CoursePage /> },
      { path: "lessons/:lessonId", element: <LessonPage /> },
      { path: "lessons/:lessonId/attend/:stageId", element: <AttendPage /> },
      { path: "lessons/:lessonId/leave", element: <LeavePage /> },
      { path: "account", element: <AccountPage /> },
      { path: "enroll", element: <EnrollPage /> },
    ],
  },
  {
    path: "*",
    element: (
      <main className="p-8">
        <ErrorState
          error={new ApiRequestError(404, "not_found", "页面不存在")}
        />
      </main>
    ),
  },
])
export function AppRouter() {
  return <RouterProvider router={router} />
}
