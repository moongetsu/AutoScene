import { NavLink, useLocation } from "react-router-dom"
import type { LucideIcon } from "lucide-react"
import { LayoutGroup, motion } from "motion/react"
import {
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
} from "@/components/ui/sidebar"
import type { SidebarMotionItem } from "@/components/app/CollapsedSidebarRail"

const listVariants = {
  hidden: {},
  show: {
    transition: { staggerChildren: 0.045, delayChildren: 0.02 },
  },
}

const itemVariants = {
  hidden: { opacity: 0, x: -8 },
  show: {
    opacity: 1,
    x: 0,
    transition: { type: "spring" as const, stiffness: 420, damping: 32 },
  },
}

type ExpandedSidebarNavProps = {
  items: SidebarMotionItem[]
}

export function ExpandedSidebarNav({ items }: ExpandedSidebarNavProps) {
  const location = useLocation()

  return (
    <LayoutGroup id="sidebar-workflow">
      <motion.div variants={listVariants} initial="hidden" animate="show">
        <SidebarMenu>
          {items.map(({ to, label, icon: Icon, end }) => {
            const active = location.pathname === to
            return (
              <SidebarMenuItem key={to}>
                <motion.div variants={itemVariants} className="relative">
                  <SidebarMenuButton asChild tooltip={label} isActive={active}>
                    <NavLink
                      to={to}
                      end={end}
                      className="relative overflow-hidden"
                    >
                      {active ? (
                        <motion.span
                          layoutId="expanded-sidebar-active"
                          className="absolute inset-0 rounded-[calc(var(--radius-sm)+2px)] bg-sidebar-accent"
                          transition={{
                            type: "spring",
                            stiffness: 420,
                            damping: 34,
                          }}
                        />
                      ) : null}
                      <motion.span
                        className="relative z-10 flex items-center gap-2"
                        whileHover={{ x: 2 }}
                        whileTap={{ scale: 0.98 }}
                        transition={{
                          type: "spring",
                          stiffness: 500,
                          damping: 28,
                        }}
                      >
                        <Icon strokeWidth={1.6} />
                        <span>{label}</span>
                      </motion.span>
                    </NavLink>
                  </SidebarMenuButton>
                </motion.div>
              </SidebarMenuItem>
            )
          })}
        </SidebarMenu>
      </motion.div>
    </LayoutGroup>
  )
}

type AnimatedSidebarMenuItemProps = {
  to: string
  label: string
  icon: LucideIcon
  isActive: boolean
}

export function AnimatedSidebarMenuItem({
  to,
  label,
  icon: Icon,
  isActive,
}: AnimatedSidebarMenuItemProps) {
  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild tooltip={label} isActive={isActive}>
        <NavLink to={to} className="relative overflow-hidden">
          {isActive ? (
            <motion.span
              layoutId="expanded-sidebar-footer-active"
              className="absolute inset-0 rounded-[calc(var(--radius-sm)+2px)] bg-sidebar-accent"
              transition={{ type: "spring", stiffness: 420, damping: 34 }}
            />
          ) : null}
          <motion.span
            className="relative z-10 flex items-center gap-2"
            whileHover={{ x: 2 }}
            whileTap={{ scale: 0.98 }}
          >
            <Icon strokeWidth={1.6} />
            <span>{label}</span>
          </motion.span>
        </NavLink>
      </SidebarMenuButton>
    </SidebarMenuItem>
  )
}
