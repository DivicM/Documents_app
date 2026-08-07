/**
 * Millimetres and pixels as distinct types.
 *
 * Both are numbers at runtime, but the branding makes them incompatible at
 * compile time, so passing a pixel count where millimetres are expected is a
 * type error rather than a photo that prints at the wrong size.
 */

declare const mmBrand: unique symbol;
declare const pxBrand: unique symbol;

export type Mm = number & { readonly [mmBrand]: true };
export type Px = number & { readonly [pxBrand]: true };

export const mm = (value: number): Mm => value as Mm;
export const px = (value: number): Px => value as Px;

export const MM_PER_INCH = 25.4;

/** Convert millimetres to pixels at a given DPI, rounded to whole pixels. */
export function mmToPx(value: Mm, dpi: number): Px {
  return px(Math.round((value / MM_PER_INCH) * dpi));
}

/** Unrounded, for positions inside a raster where rounding would drift. */
export function mmToPxExact(value: Mm, dpi: number): number {
  return (value / MM_PER_INCH) * dpi;
}

export function pxToMm(value: Px, dpi: number): Mm {
  return mm((value * MM_PER_INCH) / dpi);
}

/**
 * Scale factor that fits `paperMm` into `availablePx` on screen.
 *
 * The canvas preview and the printed sheet come from the same layout in mm;
 * only this factor differs. Keeping it explicit is what stops the preview and
 * the paper from disagreeing.
 */
export function fitScale(paperMm: Mm, availablePx: number): number {
  if (paperMm <= 0 || availablePx <= 0) return 1;
  return availablePx / paperMm;
}

/** Format a millimetre value for display, in Croatian convention (comma). */
export function formatMm(value: number, decimals = 1): string {
  return value.toFixed(decimals).replace(".", ",");
}
