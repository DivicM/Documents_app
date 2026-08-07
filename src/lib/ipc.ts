/**
 * Typed wrappers around the Tauri commands.
 *
 * Every call the UI makes goes through here, so command names and payload
 * shapes are defined once and checked by the compiler.
 */

import { invoke } from "@tauri-apps/api/core";

export interface Printer {
  name: string;
  driver: string;
  isDefault: boolean;
}

export interface PrinterCapabilities {
  dpiX: number;
  dpiY: number;
  marginLeftMm: number;
  marginTopMm: number;
  marginRightMm: number;
  marginBottomMm: number;
}

export interface Placement {
  xMm: number;
  yMm: number;
  widthMm: number;
  heightMm: number;
  rotated: boolean;
}

export interface Layout {
  placements: Placement[];
  capacityPerSheet: number;
  sheetsNeeded: number;
}

export interface LayoutRequest {
  paperWidthMm: number;
  paperHeightMm: number;
  photoWidthMm: number;
  photoHeightMm: number;
  count: number;
  marginMm: number;
  gutterMm: number;
  alignTopLeft: boolean;
}

export interface Calibration {
  scaleX: number;
  scaleY: number;
  offsetXMm: number;
  offsetYMm: number;
  calibratedAt: string;
}

export interface ResolutionCheck {
  requiredPxW: number;
  requiredPxH: number;
  sourcePxW: number;
  sourcePxH: number;
  wouldUpscale: boolean;
  maxLosslessDpi: number;
}

/* Rust uses snake_case on the wire; these map to and from camelCase. */

export async function listPrinters(): Promise<Printer[]> {
  const raw = await invoke<Array<{ name: string; driver: string; is_default: boolean }>>(
    "list_printers",
  );
  return raw.map((p) => ({ name: p.name, driver: p.driver, isDefault: p.is_default }));
}

export async function printerCapabilities(
  printer: string,
  paperWidthMm: number,
  paperHeightMm: number,
): Promise<PrinterCapabilities> {
  const r = await invoke<{
    dpi_x: number;
    dpi_y: number;
    margin_left_mm: number;
    margin_top_mm: number;
    margin_right_mm: number;
    margin_bottom_mm: number;
  }>("printer_capabilities", {
    printer,
    paperWidthMm,
    paperHeightMm,
  });
  return {
    dpiX: r.dpi_x,
    dpiY: r.dpi_y,
    marginLeftMm: r.margin_left_mm,
    marginTopMm: r.margin_top_mm,
    marginRightMm: r.margin_right_mm,
    marginBottomMm: r.margin_bottom_mm,
  };
}

export async function solveLayout(req: LayoutRequest): Promise<Layout> {
  const r = await invoke<{
    placements: Array<{
      x_mm: number;
      y_mm: number;
      width_mm: number;
      height_mm: number;
      rotated: boolean;
    }>;
    capacity_per_sheet: number;
    sheets_needed: number;
  }>("solve_layout", {
    req: {
      paper_width_mm: req.paperWidthMm,
      paper_height_mm: req.paperHeightMm,
      photo_width_mm: req.photoWidthMm,
      photo_height_mm: req.photoHeightMm,
      count: req.count,
      margin_mm: req.marginMm,
      gutter_mm: req.gutterMm,
      align_top_left: req.alignTopLeft,
    },
  });

  return {
    placements: r.placements.map((p) => ({
      xMm: p.x_mm,
      yMm: p.y_mm,
      widthMm: p.width_mm,
      heightMm: p.height_mm,
      rotated: p.rotated,
    })),
    capacityPerSheet: r.capacity_per_sheet,
    sheetsNeeded: r.sheets_needed,
  };
}

export async function checkResolution(
  sourcePxW: number,
  sourcePxH: number,
  targetWidthMm: number,
  targetHeightMm: number,
  dpi: number,
): Promise<ResolutionCheck> {
  const r = await invoke<{
    required_px_w: number;
    required_px_h: number;
    source_px_w: number;
    source_px_h: number;
    would_upscale: boolean;
    max_lossless_dpi: number;
  }>("check_resolution", {
    sourcePxW,
    sourcePxH,
    targetWidthMm,
    targetHeightMm,
    dpi,
  });
  return {
    requiredPxW: r.required_px_w,
    requiredPxH: r.required_px_h,
    sourcePxW: r.source_px_w,
    sourcePxH: r.source_px_h,
    wouldUpscale: r.would_upscale,
    maxLosslessDpi: r.max_lossless_dpi,
  };
}

export async function getCalibration(
  printer: string,
  paperWidthMm: number,
  paperHeightMm: number,
  borderless: boolean,
): Promise<Calibration | null> {
  const r = await invoke<{
    scale_x: number;
    scale_y: number;
    offset_x_mm: number;
    offset_y_mm: number;
    calibrated_at: string;
  } | null>("get_calibration", { printer, paperWidthMm, paperHeightMm, borderless });

  if (!r) return null;
  return {
    scaleX: r.scale_x,
    scaleY: r.scale_y,
    offsetXMm: r.offset_x_mm,
    offsetYMm: r.offset_y_mm,
    calibratedAt: r.calibrated_at,
  };
}

export async function saveCalibration(args: {
  printer: string;
  paperWidthMm: number;
  paperHeightMm: number;
  borderless: boolean;
  nominalMm: number;
  measuredXMm: number;
  measuredYMm: number;
}): Promise<Calibration> {
  const r = await invoke<{
    scale_x: number;
    scale_y: number;
    offset_x_mm: number;
    offset_y_mm: number;
    calibrated_at: string;
  }>("save_calibration", { ...args, nowRfc3339: new Date().toISOString() });

  return {
    scaleX: r.scale_x,
    scaleY: r.scale_y,
    offsetXMm: r.offset_x_mm,
    offsetYMm: r.offset_y_mm,
    calibratedAt: r.calibrated_at,
  };
}

/**
 * Print the 50x50mm test square.
 *
 * `applyCalibration` is false for the first print, which must show the
 * printer's raw error, and true to verify a stored correction.
 */
export async function printCalibrationSquare(
  printer: string,
  applyCalibration: boolean,
): Promise<number> {
  return invoke<number>("print_calibration_square", { printer, applyCalibration });
}

export async function printSheet(args: {
  printer: string;
  paperWidthMm: number;
  paperHeightMm: number;
  photoWidthMm: number;
  photoHeightMm: number;
  count: number;
  marginMm: number;
  gutterMm: number;
  alignTopLeft: boolean;
}): Promise<number> {
  return invoke<number>("print_sheet", args);
}
