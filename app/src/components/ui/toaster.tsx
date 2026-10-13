import { Toaster as Sonner } from "sonner"

export function Toaster() {
  return (
    <Sonner
      theme="dark"
      position="bottom-right"
      offset={16}
      gap={8}
      toastOptions={{
        unstyled: true,
        classNames: {
          toast:
            "glass flex w-[356px] items-start gap-3 rounded-lg px-4 py-3 text-sm text-fg-1 [&>[data-icon]]:mt-0.5 [&>[data-icon]>svg]:size-4",
          title: "font-medium",
          description: "mt-0.5 text-xs text-fg-2",
          content: "min-w-0 flex-1 break-words",
          actionButton: "ml-auto shrink-0 self-center whitespace-nowrap rounded-sm bg-accent px-2 py-1 text-xs font-medium text-on-accent",
          cancelButton: "ml-auto shrink-0 self-center whitespace-nowrap rounded-sm bg-active px-2 py-1 text-xs text-fg-2",
          error: "[&>[data-icon]]:text-danger",
          success: "[&>[data-icon]]:text-accent",
        },
      }}
    />
  )
}
