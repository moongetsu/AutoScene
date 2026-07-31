import { Check, Copy } from "lucide-react"
import { useState } from "react"
import { Button } from "@/components/ui/button"
import { copyText } from "@/lib/clipboard"
import { cn } from "@/lib/utils"

type CopyButtonProps = {
  text: string
  label?: string
  toastLabel?: string
  variant?: "default" | "outline" | "ghost" | "secondary"
  size?: "default" | "sm" | "xs" | "icon" | "icon-sm"
  className?: string
  children?: React.ReactNode
}

export function CopyButton({
  text,
  label = "Copy",
  toastLabel,
  variant = "outline",
  size = "sm",
  className,
  children,
}: CopyButtonProps) {
  const [copied, setCopied] = useState(false)

  return (
    <Button
      variant={variant}
      size={size}
      className={cn("gap-1.5", className)}
      onClick={async () => {
        const ok = await copyText(text, toastLabel ?? "Copied to clipboard")
        if (ok) {
          setCopied(true)
          window.setTimeout(() => setCopied(false), 1600)
        }
      }}
    >
      {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
      {children ?? label}
    </Button>
  )
}
