import { useState } from "react"
import { CircleNotch } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { baseName, mediaTitle } from "@/lib/media"
import { useRelink } from "@/state/relink"
import { relinkAnyway } from "./relink"

/** Asks before a file that doesn't look like the missing episode takes its place. */
export function RelinkDialog() {
  const question = useRelink((s) => s.question)
  const close = useRelink((s) => s.close)
  const [busy, setBusy] = useState(false)
  return (
    <Dialog open={question != null} onOpenChange={(o) => !o && !busy && close()}>
      {question && (
        <DialogContent
          title={`Use “${baseName(question.path)}” for ${mediaTitle(question.media)}?`}
          description={`It doesn't look like the same episode. ${question.reason} If you use it anyway, it's analysed again, and your decisions go back onto the faces that look like theirs.`}
        >
          <DialogFooter>
            <Button variant="ghost" disabled={busy} onClick={close}>
              Cancel
            </Button>
            <Button
              variant="primary"
              disabled={busy}
              onClick={() => {
                setBusy(true)
                void relinkAnyway(question.pid, question.media, question.path).finally(() => {
                  setBusy(false)
                  // Only this question: another may have come meanwhile.
                  if (useRelink.getState().question === question) close()
                })
              }}
            >
              {busy && <CircleNotch className="animate-spin" />}
              Use it anyway
            </Button>
          </DialogFooter>
        </DialogContent>
      )}
    </Dialog>
  )
}
