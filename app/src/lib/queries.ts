import { QueryClient, useQuery } from "@tanstack/react-query"
import { commands } from "./ipc"

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Data changes are pushed as events that invalidate queries; never poll.
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
})

export const qk = {
  appInfo: ["appInfo"] as const,
}

export function useAppInfo() {
  return useQuery({ queryKey: qk.appInfo, queryFn: () => commands.appInfo() })
}
