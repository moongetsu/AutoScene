import { toast } from "sonner"
import type { ActionKind, PluginInfo } from "@/bindings"
import { commands, errorMessage } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"

/**
 * Run a plugin's export action on an export, or its command (in the open
 * project, if any). The run shows in Activity; a toast says when it can't
 * start, or the same run waits or runs already.
 */
export async function runPlugin(
  run: { pluginId: string; actionId: string; title: string },
  kind: ActionKind,
  projectId: string | null,
  exportId: string | null,
) {
  try {
    const job = await commands.pluginRun(run.pluginId, run.actionId, kind, projectId, exportId)
    if (job == null) toast(`${run.title} is already waiting or running`, { description: "It's in Activity." })
  } catch (e) {
    toast.error(`Couldn't run ${run.title}`, { description: errorMessage(e) })
  }
}

/** Put `p` as the backend gave it back into the plugin list on hand. */
export function keepPlugin(p: PluginInfo) {
  queryClient.setQueryData<PluginInfo[]>(qk.plugins, (list) => list?.map((x) => (x.id === p.id ? p : x)))
}
