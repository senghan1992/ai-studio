/**
 * Multi-selection: an ordered list of block ids, last entry primary.
 *
 * A plain click replaces the selection; Ctrl/Cmd+click toggles one block in
 * it, as in Office. Pure helpers so the rule is unit-testable — the editors
 * own the state and call these.
 */

/** A plain click (or Escape): exactly one block, or nothing. */
export function selectSingle(ids, id) {
  void ids;
  return id ? [id] : [];
}

/** Ctrl/Cmd+click: flip one block's membership, keeping the rest in order. */
export function toggleInSelection(ids, id) {
  if (!id) return ids;
  return ids.includes(id) ? ids.filter((entry) => entry !== id) : [...ids, id];
}
