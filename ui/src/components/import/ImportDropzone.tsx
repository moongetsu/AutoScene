import { useCallback, useState } from "react"
import { motion, AnimatePresence } from "motion/react"
import { FolderOpen, Upload } from "lucide-react"
import { Button as AnimatedButton } from "@/components/ui/animated-button"
import {
  UploadProgressBar,
  type UploadProgressState,
} from "@/components/import/UploadProgressBar"
import { cn } from "@/lib/utils"

type ImportDropzoneProps = {
  busy: boolean
  gpuLabel?: string
  upload: UploadProgressState | null
  /** Drag-drop / file without a filesystem path → one-time upload write. */
  onFile: (file: File) => void
  /** Browse → link existing path (no second copy). */
  onBrowse: () => void
}

function SprocketRail({ side, active }: { side: "left" | "right"; active: boolean }) {
  return (
    <motion.div
      aria-hidden
      initial={false}
      animate={{ opacity: active ? 0.55 : 0.08 }}
      transition={{ type: "spring", stiffness: 280, damping: 28 }}
      className={cn(
        "pointer-events-none absolute inset-y-10 flex w-2.5 flex-col justify-between",
        side === "left" ? "left-4" : "right-4",
      )}
    >
      {Array.from({ length: 10 }).map((_, i) => (
        <span
          key={i}
          className="h-2.5 w-full rounded-[2px] bg-primary/70 ring-1 ring-primary/20"
        />
      ))}
    </motion.div>
  )
}

export function ImportDropzone({
  busy,
  gpuLabel,
  upload,
  onFile,
  onBrowse,
}: ImportDropzoneProps) {
  const [dragging, setDragging] = useState(false)

  const handleFile = useCallback(
    (file: File | null) => {
      if (!file || busy) return
      onFile(file)
    },
    [busy, onFile],
  )

  return (
    <div className="relative">
      <motion.div
        onDragOver={(e) => {
          e.preventDefault()
          setDragging(true)
        }}
        onDragLeave={() => setDragging(false)}
        onDrop={(e) => {
          e.preventDefault()
          setDragging(false)
          handleFile(e.dataTransfer.files?.[0] ?? null)
        }}
        initial={{ opacity: 0, y: 16 }}
        animate={{
          opacity: 1,
          y: 0,
          borderColor: dragging
            ? "oklch(0.78 0.14 65 / 0.55)"
            : "oklch(0.94 0.01 85 / 0.1)",
        }}
        transition={{ type: "spring", stiffness: 260, damping: 28 }}
        className={cn(
          "relative flex min-h-[min(72dvh,680px)] flex-col overflow-hidden rounded-[1.35rem] border bg-[oklch(0.085_0.014_280)]",
          dragging && "bg-[oklch(0.1_0.02_280)]",
        )}
      >
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 bg-[radial-gradient(90%_70%_at_35%_40%,oklch(0.16_0.02_280)_0%,transparent_62%)]"
        />
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 bg-[radial-gradient(120%_90%_at_50%_120%,oklch(0.78_0.14_65/0.07),transparent_45%)]"
        />

        <div aria-hidden className="pointer-events-none absolute inset-5">
          <span className="absolute left-0 top-0 h-8 w-px bg-primary/35" />
          <span className="absolute left-0 top-0 h-px w-8 bg-primary/35" />
          <span className="absolute right-0 top-0 h-8 w-px bg-primary/35" />
          <span className="absolute right-0 top-0 h-px w-8 bg-primary/35" />
          <span className="absolute bottom-0 left-0 h-8 w-px bg-primary/25" />
          <span className="absolute bottom-0 left-0 h-px w-8 bg-primary/25" />
          <span className="absolute bottom-0 right-0 h-8 w-px bg-primary/25" />
          <span className="absolute bottom-0 right-0 h-px w-8 bg-primary/25" />
        </div>

        <SprocketRail side="left" active={dragging} />
        <SprocketRail side="right" active={dragging} />

        <motion.div
          aria-hidden
          className="pointer-events-none absolute inset-x-10 bottom-0 h-px origin-left bg-primary/70"
          animate={{ scaleX: dragging ? 1 : 0.42, opacity: dragging ? 1 : 0.55 }}
          transition={{ type: "spring", stiffness: 220, damping: 26 }}
        />

        <div className="relative z-10 flex flex-1 flex-col justify-between px-10 py-11 sm:px-14 sm:py-14">
          <div className="max-w-xl">
            <p className="text-[11px] uppercase tracking-[0.2em] text-muted-foreground">
              Import
            </p>

            <motion.h2
              animate={{ y: dragging ? -3 : 0 }}
              transition={{ type: "spring", stiffness: 340, damping: 24 }}
              className="font-heading mt-4 text-[2.65rem] font-semibold leading-[1.1] tracking-[-0.03em] sm:text-5xl lg:text-[3.1rem]"
            >
              Drop your{" "}
              <span className="text-primary">footage</span>
            </motion.h2>

            <p className="mt-4 max-w-md text-[15px] leading-relaxed text-muted-foreground">
              MP4, MKV, MOV. Browse links the file path (no second copy). Drag-drop uploads once
              when a path is unavailable. Face scan starts after import.
            </p>
          </div>

          <div className="mt-12 flex flex-col gap-8 lg:flex-row lg:items-end lg:justify-between">
            <div>
              <div className="mb-5 flex flex-wrap gap-1.5">
                {["MP4", "MKV", "MOV", "AVI"].map((ext) => (
                  <span
                    key={ext}
                    className="timecode rounded px-1.5 py-0.5 text-[10px] uppercase tracking-[0.16em] text-muted-foreground/80"
                  >
                    {ext}
                  </span>
                ))}
              </div>
              <div className="flex flex-wrap items-center gap-3">
                <AnimatedButton
                  className="min-h-11 gap-2 px-5"
                  size="lg"
                  disabled={busy}
                  onClick={onBrowse}
                >
                  {busy ? (
                    <>
                      <Upload className="size-4 animate-pulse" />
                      Working…
                    </>
                  ) : (
                    <>
                      <FolderOpen className="size-4" />
                      Browse
                    </>
                  )}
                </AnimatedButton>
              </div>
              {gpuLabel ? (
                <p className="mt-4 text-xs text-muted-foreground/80">{gpuLabel}</p>
              ) : null}
            </div>

            <motion.p
              animate={{ opacity: dragging ? 1 : 0.45 }}
              className="max-w-[12rem] text-right text-[11px] leading-relaxed text-muted-foreground"
            >
              {dragging ? "Drop to upload" : "Or drag a video here"}
            </motion.p>
          </div>
        </div>

        <AnimatePresence>
          {upload ? (
            <motion.div
              initial={{ opacity: 0, y: 12 }}
              animate={{ opacity: 1, y: 0 }}
              exit={{ opacity: 0, y: 8 }}
              className="relative z-10"
            >
              <UploadProgressBar upload={upload} />
            </motion.div>
          ) : null}
        </AnimatePresence>
      </motion.div>
    </div>
  )
}
