import type { AppError } from "@/bindings"

export { commands, events } from "@/bindings"
export type * from "@/bindings"

/** Commands throw `AppError` objects; anything else is an unexpected failure. */
export function isAppError(e: unknown): e is AppError {
  return typeof e === "object" && e !== null && "code" in e && "message" in e
}

/** Human-readable message for any thrown value. */
export function errorMessage(e: unknown): string {
  if (isAppError(e)) return e.message
  if (e instanceof Error) return e.message
  return String(e)
}
