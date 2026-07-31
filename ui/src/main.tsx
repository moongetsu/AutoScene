import { StrictMode } from "react"
import { createRoot } from "react-dom/client"
import "./index.css"
import App from "./App.tsx"
import { hideBootSplash, startBootWatch } from "./boot"

const rootEl = document.getElementById("root")!

let mounted = false

function mountApp() {
  if (mounted) return
  mounted = true
  createRoot(rootEl).render(
    <StrictMode>
      <App />
    </StrictMode>,
  )
  // Let first paint settle, then fade splash
  requestAnimationFrame(() => {
    window.setTimeout(() => hideBootSplash(), 120)
  })
}

startBootWatch(mountApp)
