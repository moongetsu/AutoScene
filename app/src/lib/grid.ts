/** Columns that fit `width` with at least `min` px each, and the width each gets. */
export function gridColumns(width: number, min: number, gap: number): { columns: number; itemWidth: number } {
  const columns = Math.max(1, Math.floor((width + gap) / (min + gap)))
  return { columns, itemWidth: Math.max(0, (width - gap * (columns - 1)) / columns) }
}

/**
 * Clicking in a grid of selectable pictures, like a file browser: Shift
 * adds the range from the last picture clicked; Ctrl/⌘ — or any click once
 * something is selected — flips one; a plain click starts a selection.
 */
export function clickSelect<K>(
  keys: readonly K[],
  selected: ReadonlySet<K>,
  anchor: K | null,
  index: number,
  { shift, toggle }: { shift: boolean; toggle: boolean },
): { selected: Set<K>; anchor: K | null } {
  const key = keys[index]
  const next = new Set(selected)
  if (shift && anchor != null) {
    const a = keys.indexOf(anchor)
    const [lo, hi] = a < 0 ? [index, index] : [Math.min(a, index), Math.max(a, index)]
    for (let k = lo; k <= hi; k++) next.add(keys[k])
    return { selected: next, anchor }
  }
  if (toggle || selected.size > 0) {
    if (next.has(key)) next.delete(key)
    else next.add(key)
  } else {
    next.add(key)
  }
  return { selected: next, anchor: key }
}
