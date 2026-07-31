import { useRef, useState } from "react"
import { NavLink, useLocation } from "react-router-dom"
import { AnimatePresence, motion } from "motion/react"
import type { LucideIcon } from "lucide-react"
import { cn } from "@/lib/utils"

export type SidebarMotionItem = {
  to: string
  label: string
  icon: LucideIcon
  end?: boolean
}

type CollapsedSidebarRailProps = {
  items: SidebarMotionItem[]
  tooltipDelay?: number
}

export function CollapsedSidebarRail({
  items,
  tooltipDelay = 220,
}: CollapsedSidebarRailProps) {
  const location = useLocation()
  const [hoverIndex, setHoverIndex] = useState<number | null>(null)
  const [coords, setCoords] = useState({ clipPath: "", translateY: 0 })
  const [isEntering, setIsEntering] = useState(true)
  const measureRefs = useRef<(HTMLDivElement | null)[]>([])
  const buttonRefs = useRef<(HTMLAnchorElement | null)[]>([])
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const calculatePosition = (index: number) => {
    const activeLabel = measureRefs.current[index]
    const activeIcon = buttonRefs.current[index]
    if (!activeLabel || !activeIcon) return null

    const labelTop = activeLabel.offsetTop
    const labelHeight = activeLabel.offsetHeight
    const labelCenter = labelTop + labelHeight / 2
    const iconTop = activeIcon.offsetTop
    const iconHeight = activeIcon.offsetHeight
    const iconCenter = iconTop + iconHeight / 2
    const totalHeight = measureRefs.current.reduce(
      (acc, el) => acc + (el?.offsetHeight || 0),
      0,
    )
    const cTop = (labelTop / totalHeight) * 100
    const cBottom = 100 - ((labelTop + labelHeight) / totalHeight) * 100

    return {
      clipPath: `inset(${cTop}% 0 ${cBottom}% 0 round 8px)`,
      translateY: iconCenter - labelCenter,
    }
  }

  const handleMouseEnter = (index: number) => {
    const newCoords = calculatePosition(index)
    if (!newCoords) return

    if (hoverIndex === null) {
      if (timeoutRef.current) clearTimeout(timeoutRef.current)
      setIsEntering(true)
      timeoutRef.current = setTimeout(() => {
        setCoords(newCoords)
        setHoverIndex(index)
      }, tooltipDelay)
    } else {
      setCoords(newCoords)
      setHoverIndex(index)
    }
  }

  const handleMouseLeave = () => {
    if (timeoutRef.current) clearTimeout(timeoutRef.current)
    setHoverIndex(null)
    setCoords({ clipPath: "", translateY: 0 })
    setIsEntering(true)
  }

  const isActive = (to: string, end?: boolean) =>
    end ? location.pathname === to : location.pathname === to

  return (
    <div className="relative px-1 py-2" onMouseLeave={handleMouseLeave}>
      <AnimatePresence>
        {hoverIndex !== null && coords.clipPath !== "" ? (
          <motion.div
            className="pointer-events-none absolute top-2 left-[calc(100%+0.35rem)] z-50"
            initial={{ opacity: 0 }}
            animate={{ opacity: 1 }}
            exit={{ opacity: 0 }}
            transition={{ duration: 0.16 }}
          >
            <motion.div
              className="overflow-hidden rounded-lg border border-border/60 bg-popover shadow-xl"
              animate={{ clipPath: coords.clipPath, y: coords.translateY }}
              transition={{
                type: "spring",
                bounce: 0,
                duration: isEntering ? 0 : 0.38,
              }}
              onUpdate={() => {
                if (isEntering) setIsEntering(false)
              }}
            >
              <div className="flex flex-col py-1">
                {items.map((item) => (
                  <div
                    key={item.to}
                    className="flex h-9 items-center px-3 text-xs font-medium whitespace-nowrap text-popover-foreground"
                  >
                    {item.label}
                  </div>
                ))}
              </div>
            </motion.div>
          </motion.div>
        ) : null}
      </AnimatePresence>

      <div className="relative z-10 flex flex-col items-center gap-1 rounded-xl border border-sidebar-border/60 bg-sidebar/90 p-1.5 backdrop-blur-sm">
        {items.map((item, index) => {
          const active = isActive(item.to, item.end)
          const Icon = item.icon
          return (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.end}
              ref={(el) => {
                buttonRefs.current[index] = el
              }}
              onMouseEnter={() => handleMouseEnter(index)}
              className={cn(
                "relative flex size-9 items-center justify-center rounded-lg transition-colors",
                active
                  ? "bg-primary/15 text-primary"
                  : "text-muted-foreground hover:bg-sidebar-accent hover:text-foreground",
              )}
            >
              {active ? (
                <motion.span
                  layoutId="collapsed-sidebar-active"
                  className="absolute inset-0 rounded-lg bg-primary/15 ring-1 ring-primary/25"
                  transition={{ type: "spring", stiffness: 420, damping: 34 }}
                />
              ) : null}
              <span className="relative z-10">
                <Icon className="size-4" strokeWidth={1.6} />
              </span>
              <span className="sr-only">{item.label}</span>
            </NavLink>
          )
        })}
      </div>

      <div className="pointer-events-none absolute top-0 left-0 flex flex-col overflow-hidden whitespace-nowrap opacity-0">
        {items.map((item, index) => (
          <div
            key={`measure-${item.to}`}
            ref={(el) => {
              measureRefs.current[index] = el
            }}
            className="flex h-9 items-center px-3 text-xs font-medium whitespace-nowrap"
          >
            {item.label}
          </div>
        ))}
      </div>
    </div>
  )
}
