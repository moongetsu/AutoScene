/**
 * requestVideoFrameCallback is not in TypeScript's DOM lib yet, and
 * tsconfig.app.json only pulls ["ES2023", "DOM", "DOM.Iterable"].
 * Declared here rather than adding a dependency for two method signatures.
 */
interface VideoFrameCallbackMetadata {
  presentationTime: DOMHighResTimeStamp
  expectedDisplayTime: DOMHighResTimeStamp
  width: number
  height: number
  mediaTime: number
  presentedFrames: number
  processingDuration?: number
}

type VideoFrameRequestCallback = (
  now: DOMHighResTimeStamp,
  metadata: VideoFrameCallbackMetadata,
) => void

interface HTMLVideoElement {
  requestVideoFrameCallback?: (callback: VideoFrameRequestCallback) => number
  cancelVideoFrameCallback?: (handle: number) => void
}
