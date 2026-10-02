import { useQuery, useQueryClient } from "@tanstack/react-query"
import type { QueryClient } from "@tanstack/react-query"
import {
  coursesListStudents,
  getCoursesListStudentsQueryKey,
} from "@/api/generated/client"
import type { PageStudentRosterItemItemsItem } from "@/api/generated/models"

export type RosterNameMap = Map<string, PageStudentRosterItemItemsItem>
const rosterPageSize = 100

export async function loadRosterNames(
  courseId: string,
  client: QueryClient,
  signal: AbortSignal,
): Promise<RosterNameMap> {
  const members: RosterNameMap = new Map()
  let offset = 0
  let total = 0
  do {
    if (signal.aborted) throw new DOMException("已取消", "AbortError")
    const params = { limit: rosterPageSize, offset }
    const queryKey = getCoursesListStudentsQueryKey(courseId, params)
    const page = await client.fetchQuery({
      queryKey,
      queryFn: ({ signal: pageSignal }) =>
        coursesListStudents(courseId, params, { signal: pageSignal }),
      staleTime: 30_000,
    })
    for (const member of page.items) members.set(member.user_id, member)
    total = page.total
    offset += page.items.length
    if (page.items.length === 0) break
  } while (offset < total)
  return members
}

export function useRosterNames(courseId: string, enabled: boolean) {
  const client = useQueryClient()
  const params = { limit: rosterPageSize, offset: 0 }
  return useQuery({
    queryKey: [
      "course-roster-name-map",
      getCoursesListStudentsQueryKey(courseId, params),
    ],
    queryFn: ({ signal }) => loadRosterNames(courseId, client, signal),
    enabled,
    staleTime: 30_000,
  })
}
