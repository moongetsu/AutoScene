import { Toaster as Sonner } from "sonner"

export function Toaster() {
  return (
    <Sonner
      theme="dark"
      position="top-right"
      offset={{ top: 64, right: 16 }}
      gap={8}
      toastOptions={{
        unstyled: true,
        classNames: {
          toast:
            "glass-strong flex w-[380px] items-start gap-3 rounded-2xl px-4 py-3.5 text-md text-fg-1 [&>[data-icon]]:mt-0.5 [&>[data-icon]>svg]:size-4",
          title: "font-medium",
          description: "mt-0.5 text-sm text-fg-2",
          content: "min-w-0 flex-1 break-words",
          actionButton:
            "ml-auto shrink-0 self-center whitespace-nowrap rounded-full bg-cta px-3 py-1 text-sm font-semibold text-on-cta",
          cancelButton: "ml-auto shrink-0 self-center whitespace-nowrap rounded-full bg-active px-3 py-1 text-sm text-fg-2",
          error: "[&>[data-icon]]:text-danger",
          success: "[&>[data-icon]]:text-fg-1",
        },
      }}
    />
  )
}
