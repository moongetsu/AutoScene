import type * as React from "react"
import { useEffect, useLayoutEffect, useRef, useState } from "react"
import { toast } from "sonner"
import { ArrowCounterClockwise } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Kbd } from "@/components/ui/kbd"
import { Segmented } from "@/components/ui/segmented"
import { Tooltip } from "@/components/ui/tooltip"
import {
  type Use,
  assign,
  isChanged,
  pauseMenuKeys,
  refusal,
  resetShortcut,
  sameBinding,
  saveKeymap,
  slotLabel,
  withPreset,
} from "@/lib/keymap"
import { isMac } from "@/lib/platform"
import {
  type Keymap,
  PRESETS,
  type PresetId,
  SHORTCUT_GROUPS,
  type Shortcut,
  type ShortcutRef,
  bindingCaps,
  bindingOf,
  bindingsOf,
  capGroups,
  getKeymap,
  modifierCaps,
  shortcutOf,
  slotCaps,
  slotRef,
  useKeymap,
} from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { SettingRow, SettingsGroup } from "../primitives"

type Preset = (typeof PRESETS)[number]

const presetOf = (km: Keymap) => PRESETS.find((p) => p.id === km.preset) ?? PRESETS[0]

/**
 * A toast that offers the keys as they were. One at a time: an older one's
 * Undo would also take back every change since.
 */
function undoable(title: string, before: Keymap, description?: string) {
  toast(title, { id: "keymap-undo", description, action: { label: "Undo", onClick: () => void saveKeymap(before) } })
}

/** Once the focused control has gone (a reset), put the focus where `find` says. */
const focusSoon = (find: () => HTMLElement | null | undefined) => window.setTimeout(() => find()?.focus())

/**
 * After a key is recorded, its repeats and its release are no one's: a held
 * Space or Enter would press the slot again, or reach a shortcut.
 */
function swallowUntilUp(e: KeyboardEvent) {
  const same = (x: KeyboardEvent) => (e.code ? x.code === e.code : x.key === e.key)
  const done = () => {
    window.removeEventListener("keydown", eat, true)
    window.removeEventListener("keyup", eat, true)
    window.removeEventListener("blur", done)
  }
  function eat(x: KeyboardEvent) {
    if (!same(x)) return
    // Pressed again: that one is meant.
    if (x.type === "keydown" && !x.repeat) return done()
    x.preventDefault()
    x.stopPropagation()
    if (x.type === "keyup") done()
  }
  window.addEventListener("keydown", eat, true)
  window.addEventListener("keyup", eat, true)
  window.addEventListener("blur", done)
}

const quoted = (took: Use[]) => [...new Set(took.map(slotLabel))].map((n) => `“${n}”`).join(" and ")

export function ShortcutsSection() {
  const km = useKeymap()
  // The slot listening for its new key (`slotRef`), one at a time.
  const [recording, setRecording] = useState<string | null>(null)
  const preset = presetOf(km)
  const changes = Object.keys(km.own).length

  const choose = (id: PresetId) => {
    const before = getKeymap()
    if (id === before.preset) return
    void saveKeymap(withPreset(id))
    const n = Object.keys(before.own).length
    if (n === 0) return
    const label = presetOf(withPreset(id)).label
    undoable(`Keys like in ${label}`, before, `Your ${n === 1 ? "change was" : `${n} changes were`} let go.`)
  }
  const resetAll = () => {
    const before = getKeymap()
    void saveKeymap(withPreset(before.preset))
    undoable(`Back to ${presetOf(before).label}'s keys`, before)
    // The button can't keep the focus: nothing is left to reset.
    focusSoon(() => document.querySelector<HTMLElement>('[aria-label="Keys like in"] [data-state="on"]'))
  }

  return (
    <>
      <SettingsGroup title="Keymap" description="Click a key below to change it, then press the new one.">
        <SettingRow label="Keys like in" description={preset.description}>
          <Segmented
            aria-label="Keys like in"
            value={km.preset}
            onChange={choose}
            options={PRESETS.map((p) => ({
              value: p.id,
              label: p.label,
              preview: Object.keys(p.keys).length > 0 ? <PresetPreview preset={p} /> : undefined,
            }))}
          />
        </SettingRow>
        {/* Always there, so the list never moves while a key is changed. */}
        <SettingRow
          label="Your changes"
          description={
            changes === 0
              ? `None: every key is ${preset.label}'s.`
              : `${changes === 1 ? "1 key differs" : `${changes} keys differ`} from ${preset.label}'s.`
          }
        >
          <Button size="sm" disabled={changes === 0} onClick={resetAll}>
            <ArrowCounterClockwise /> Reset all
          </Button>
        </SettingRow>
      </SettingsGroup>

      {SHORTCUT_GROUPS.map((g) => (
        <SettingsGroup key={g.title} title={g.title} description={g.description}>
          {g.shortcuts.map((s) => (
            <ShortcutRow key={s.id} s={s} km={km} preset={preset} recording={recording} onRecording={setRecording} />
          ))}
        </SettingsGroup>
      ))}
    </>
  )
}

/** Where a preset's keys differ from AutoScene's, as a card under its option. */
function PresetPreview({ preset }: { preset: Preset }) {
  const rows = Object.entries(preset.keys).map(([ref, bindings]) => {
    const at = ref.lastIndexOf(".")
    const s = shortcutOf(ref.slice(0, at) as ShortcutRef)
    return { ref, label: slotLabel({ s, slot: Number(ref.slice(at + 1)) }), caps: bindingCaps(bindings[0]) }
  })
  return (
    <div className="p-1">
      <div className="eyebrow mb-2.5 text-fg-3">Unlike AutoScene's keys</div>
      <ul className="space-y-2">
        {rows.map((r) => (
          <li key={r.ref} className="flex items-center justify-between gap-4 text-sm text-fg-1">
            <span>{r.label}</span>
            <Kbd keys={r.caps} />
          </li>
        ))}
      </ul>
    </div>
  )
}

function ShortcutRow({
  s,
  km,
  preset,
  recording,
  onRecording,
}: {
  s: Shortcut
  km: Keymap
  preset: Preset
  recording: string | null
  onRecording: (ref: string | null) => void
}) {
  const reset = (e: React.MouseEvent<HTMLButtonElement>) => {
    const before = getKeymap()
    const { next, took } = resetShortcut(before, s)
    void saveKeymap(next)
    if (took.length > 0) undoable(`Took them back from ${quoted(took)}`, before)
    // The reset goes with the change: the focus moves on to the row's key.
    const row = e.currentTarget.parentElement
    focusSoon(() => row?.querySelector<HTMLElement>("button[aria-pressed]"))
  }
  return (
    <SettingRow label={s.label} className="min-h-12 py-2">
      {isChanged(s, km) && (
        <Tooltip label={`Back to ${preset.label}'s ${s.slots.length > 1 ? "keys" : "key"}`}>
          <Button variant="ghost" size="icon-sm" aria-label={`Reset “${s.label}”`} onClick={reset} className="animate-fade-in">
            <ArrowCounterClockwise />
          </Button>
        </Tooltip>
      )}
      {s.fixed ? (
        <Kbd keys={capGroups(s, km).flat()} className="px-1.5" />
      ) : (
        s.slots.map((_, i) => (
          <KeySlot
            key={slotRef(s, i)}
            s={s}
            slot={i}
            km={km}
            active={recording === slotRef(s, i)}
            onActive={(on) => onRecording(on ? slotRef(s, i) : null)}
          />
        ))
      )}
    </SettingRow>
  )
}

/** How long a refused key's reason stays before the slot asks again. */
const HINT_MS = 1800

/**
 * One key of a shortcut: a click listens for the new one (Esc stops, Tab
 * moves on). While it listens it has every key to itself — no shortcut and
 * no menu answers — and only modifiers keep it waiting, shown as held.
 */
function KeySlot({
  s,
  slot,
  km,
  active,
  onActive,
}: {
  s: Shortcut
  slot: number
  km: Keymap
  active: boolean
  onActive: (on: boolean) => void
}) {
  const button = useRef<HTMLButtonElement>(null)
  const latest = useRef(onActive)
  useLayoutEffect(() => {
    latest.current = onActive
  })
  const [held, setHeld] = useState<string[]>([])
  const [hint, setHint] = useState<string | null>(null)

  useEffect(() => {
    if (!active) return
    const stop = () => latest.current(false)
    let timer: number | undefined
    const say = (why: string) => {
      setHint(why)
      window.clearTimeout(timer)
      timer = window.setTimeout(() => setHint(null), HINT_MS)
    }
    const modifiers = (e: KeyboardEvent) => modifierCaps({ mod: isMac ? e.metaKey : e.ctrlKey, shift: e.shiftKey, alt: e.altKey })
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Tab" && !e.ctrlKey && !e.metaKey && !e.altKey) return stop()
      e.preventDefault()
      e.stopPropagation()
      // A key held since before, or still held: only a fresh press counts.
      if (e.repeat) return
      if (e.key === "Escape") return stop()
      if (isMac ? e.ctrlKey : e.metaKey) return say(isMac ? "Use ⌘ instead of Control" : "The Windows key is Windows'")
      const b = bindingOf(e)
      if (!b) return setHeld(modifiers(e))
      const before = getKeymap()
      // The key it shows already: nothing changes (its other keys stay too).
      const shown = bindingsOf(s, slot, before)[0]
      if (shown && sameBinding(shown, b)) {
        swallowUntilUp(e)
        return stop()
      }
      const why = refusal(b, s, slot, before)
      if (why) return say(why)
      const { next, took } = assign(before, s, slot, b)
      swallowUntilUp(e)
      stop()
      void saveKeymap(next)
      if (took.length > 0) undoable(`Took ${bindingCaps(b).join(" ")} from ${quoted(took)}`, before)
    }
    const onKeyUp = (e: KeyboardEvent) => {
      e.stopPropagation()
      setHeld(modifiers(e))
    }
    const onPointerDown = (e: PointerEvent) => {
      if (!(e.target instanceof Node && button.current?.contains(e.target))) stop()
    }
    pauseMenuKeys(true)
    window.addEventListener("keydown", onKeyDown, true)
    window.addEventListener("keyup", onKeyUp, true)
    window.addEventListener("pointerdown", onPointerDown, true)
    window.addEventListener("blur", stop)
    return () => {
      window.clearTimeout(timer)
      pauseMenuKeys(false)
      window.removeEventListener("keydown", onKeyDown, true)
      window.removeEventListener("keyup", onKeyUp, true)
      window.removeEventListener("pointerdown", onPointerDown, true)
      window.removeEventListener("blur", stop)
    }
  }, [active, s, slot])

  const caps = slotCaps(s, slot, km)
  const own = slotRef(s, slot) in km.own
  const label = slotLabel({ s, slot })
  const several = s.slots.length > 1
  return (
    <>
      <Tooltip label={several ? label : "Change the key"} hint={several ? "Click to change" : undefined} disabled={active}>
        <button
          ref={button}
          type="button"
          aria-label={active ? `Press the new key for “${label}”` : `${label}: ${caps.join(" ") || "no key"}. Change`}
          aria-pressed={active}
          onClick={() => {
            // Nothing held, nothing refused, from the start.
            setHeld([])
            setHint(null)
            onActive(!active)
          }}
          className={cn(
            "inline-flex h-8 items-center gap-2 rounded-lg px-1.5 text-sm font-medium outline-none",
            "transition-[background-color,box-shadow,color] duration-[var(--dur-fast)] ease-out focus-visible:ring-2 focus-visible:ring-accent-line",
            !active && "hover:bg-white/[0.07]",
            active && "bg-accent-subtle px-2.5 text-accent ring-1 ring-accent-line",
            active && hint && "bg-danger-subtle text-danger ring-danger/40",
          )}
        >
          {active ? (
            <>
              <span aria-hidden className={cn("size-1.5 rounded-full", hint ? "bg-danger" : "bg-accent motion-safe:animate-pulse")} />
              {hint ?? (held.length > 0 ? <Kbd keys={[...held, "…"]} tone="accent" /> : "Press a key")}
            </>
          ) : caps.length > 0 ? (
            <Kbd keys={caps} tone={own ? "accent" : undefined} />
          ) : (
            <span className="rounded-[5px] border border-dashed border-white/15 px-1.5 py-0.5 text-2xs text-fg-3">No key</span>
          )}
        </button>
      </Tooltip>
      {/* The button's name says what it waits for; why a key was refused is read out here. */}
      <span role="status" className="sr-only">
        {active ? hint : null}
      </span>
    </>
  )
}
