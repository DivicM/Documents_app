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

export interface FaceDetection {
  faceBox: { x: number; y: number; width: number; height: number };
  rightEye: { x: number; y: number };
  leftEye: { x: number; y: number };
  nose: { x: number; y: number };
  chin: { x: number; y: number };
  crown: { x: number; y: number };
  confidence: number;
  rollDeg: number;
  eyeDistancePx: number;
  anchorsEstimated: boolean;
}

/**
 * Detect the subject's face in raw canvas pixels.
 *
 * Pixels rather than a file: the webview has already decoded the image, and
 * sending them straight across avoids re-encoding and keeps a JPEG decoder out
 * of the Rust build.
 */
export async function detectFace(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
): Promise<FaceDetection | null> {
  const r = await invoke<{
    face_box: { x: number; y: number; width: number; height: number };
    right_eye: { x: number; y: number };
    left_eye: { x: number; y: number };
    nose: { x: number; y: number };
    chin: { x: number; y: number };
    crown: { x: number; y: number };
    confidence: number;
    roll_deg: number;
    eye_distance_px: number;
    anchors_estimated: boolean;
  } | null>("detect_face", { rgba: Array.from(rgba), width, height });

  if (!r) return null;
  return {
    faceBox: r.face_box,
    rightEye: r.right_eye,
    leftEye: r.left_eye,
    nose: r.nose,
    chin: r.chin,
    crown: r.crown,
    confidence: r.confidence,
    rollDeg: r.roll_deg,
    eyeDistancePx: r.eye_distance_px,
    anchorsEstimated: r.anchors_estimated,
  };
}

export interface CropResult {
  rect: { x: number; y: number; width: number; height: number };
  maxLosslessDpi: number;
}

export async function computeCrop(req: {
  chinX: number;
  chinY: number;
  crownX: number;
  crownY: number;
  imageWidth: number;
  imageHeight: number;
  photoWidthMm: number;
  photoHeightMm: number;
  headHeightMm: number;
  chinFromBottomMm?: number | null;
}): Promise<CropResult> {
  const r = await invoke<{
    rect: { x: number; y: number; width: number; height: number };
    max_lossless_dpi: number;
  }>("compute_crop", {
    req: {
      chin_x: req.chinX,
      chin_y: req.chinY,
      crown_x: req.crownX,
      crown_y: req.crownY,
      image_width: req.imageWidth,
      image_height: req.imageHeight,
      photo_width_mm: req.photoWidthMm,
      photo_height_mm: req.photoHeightMm,
      head_height_mm: req.headHeightMm,
      chin_from_bottom_mm: req.chinFromBottomMm ?? null,
    },
  });
  return { rect: r.rect, maxLosslessDpi: r.max_lossless_dpi };
}

export interface PhotoPayload {
  rgba: Uint8ClampedArray;
  width: number;
  height: number;
  cropX: number;
  cropY: number;
  cropWidth: number;
  cropHeight: number;
  /** Head tilt to straighten, in degrees. */
  rotationDeg: number;
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
  /** Omit to print the layout as plain rectangles, without using photo paper. */
  photo?: PhotoPayload | null;
}): Promise<number> {
  return invoke<number>("print_sheet", {
    req: {
      printer: args.printer,
      paper_width_mm: args.paperWidthMm,
      paper_height_mm: args.paperHeightMm,
      photo_width_mm: args.photoWidthMm,
      photo_height_mm: args.photoHeightMm,
      count: args.count,
      margin_mm: args.marginMm,
      gutter_mm: args.gutterMm,
      align_top_left: args.alignTopLeft,
      photo: args.photo
        ? {
            rgba: Array.from(args.photo.rgba),
            width: args.photo.width,
            height: args.photo.height,
            crop_x: args.photo.cropX,
            crop_y: args.photo.cropY,
            crop_width: args.photo.cropWidth,
            crop_height: args.photo.cropHeight,
            rotation_deg: args.photo.rotationDeg,
          }
        : null,
    },
  });
}
