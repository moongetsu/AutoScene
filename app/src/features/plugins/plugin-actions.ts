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

/**
 * Let plugins AutoScene hasn't reviewed run, or not (off, their runs stop).
 * On, AutoScene asks first, in a dialog of the system's own: whether they
 * may run now.
 */
export async function allowUnreviewed(on: boolean): Promise<boolean> {
  try {
    const settings = await commands.pluginsAllowUnreviewed(on)
    queryClient.setQueryData(qk.settings, settings)
    void queryClient.invalidateQueries({ queryKey: qk.plugins })
    return settings.allowUnreviewedPlugins ?? false
  } catch (e) {
    toast.error("Couldn't save the setting", { description: errorMessage(e) })
    return false
  }
}

/** Put `p` as the backend gave it back into the plugin list on hand. */
export function keepPlugin(p: PluginInfo) {
  queryClient.setQueryData<PluginInfo[]>(qk.plugins, (list) => list?.map((x) => (x.id === p.id ? p : x)))
}
