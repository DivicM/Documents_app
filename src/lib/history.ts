/**
 * Undo/redo over editable state.
 *
 * Snapshots parameters, never pixels: the whole edit state is a handful of
 * numbers, so a snapshot costs nothing and an undo is a plain assignment. The
 * alternative, storing rendered bitmaps, would be orders of magnitude more
 * memory for the same result.
 */

/** Everything a user edit can change. */
export interface EditState<TPoint = { x: number; y: number }> {
  anchorOverride: Partial<Record<"chin" | "crown" | "rightEye" | "leftEye", TPoint>>;
  rotationOverride: number | null;
  headHeightMm: number;
  photoWidthMm: number;
  photoHeightMm: number;
  count: number;
  marginMm: number;
  gutterMm: number;
  alignTopLeft: boolean;
  cutMarks: boolean;
  replaceBackground: boolean;
  bgColour: [number, number, number];
  exposureEv: number;
  contrast: number;
  temperature: number;
  tint: number;
}

export interface History<T> {
  past: T[];
  present: T;
  future: T[];
}

/** Snapshots beyond this are dropped; deep history is never used in practice. */
const MAX_DEPTH = 50;

export function createHistory<T>(initial: T): History<T> {
  return { past: [], present: initial, future: [] };
}

/**
 * Record a new state.
 *
 * Redo is cleared, because branching after an undo would make the forward
 * history refer to a timeline the user abandoned.
 */
export function push<T>(history: History<T>, next: T): History<T> {
  const past = [...history.past, history.present];
  return {
    past: past.length > MAX_DEPTH ? past.slice(past.length - MAX_DEPTH) : past,
    present: next,
    future: [],
  };
}

export function undo<T>(history: History<T>): History<T> {
  if (history.past.length === 0) return history;
  const previous = history.past[history.past.length - 1];
  return {
    past: history.past.slice(0, -1),
    present: previous,
    future: [history.present, ...history.future],
  };
}

export function redo<T>(history: History<T>): History<T> {
  if (history.future.length === 0) return history;
  const [next, ...rest] = history.future;
  return {
    past: [...history.past, history.present],
    present: next,
    future: rest,
  };
}

export function canUndo<T>(history: History<T>): boolean {
  return history.past.length > 0;
}

export function canRedo<T>(history: History<T>): boolean {
  return history.future.length > 0;
}

/**
 * Whether two states differ in any way the user would notice.
 *
 * Used to avoid recording a snapshot for a change that is not really one, such
 * as a slider emitting the value it already had.
 */
export function statesEqual(a: EditState, b: EditState): boolean {
  if (
    a.rotationOverride !== b.rotationOverride ||
    a.headHeightMm !== b.headHeightMm ||
    a.photoWidthMm !== b.photoWidthMm ||
    a.photoHeightMm !== b.photoHeightMm ||
    a.count !== b.count ||
    a.marginMm !== b.marginMm ||
    a.gutterMm !== b.gutterMm ||
    a.alignTopLeft !== b.alignTopLeft ||
    a.cutMarks !== b.cutMarks ||
    a.replaceBackground !== b.replaceBackground ||
    a.exposureEv !== b.exposureEv ||
    a.contrast !== b.contrast ||
    a.temperature !== b.temperature ||
    a.tint !== b.tint
  ) {
    return false;
  }
  if (a.bgColour.some((v, i) => v !== b.bgColour[i])) return false;

  const keys = ["chin", "crown", "rightEye", "leftEye"] as const;
  for (const k of keys) {
    const pa = a.anchorOverride[k];
    const pb = b.anchorOverride[k];
    if (!pa !== !pb) return false;
    if (pa && pb && (pa.x !== pb.x || pa.y !== pb.y)) return false;
  }
  return true;
}
