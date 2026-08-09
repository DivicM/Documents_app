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

export interface BackgroundPayload {
  mask: Uint8Array;
  maskWidth: number;
  maskHeight: number;
  colour: [number, number, number];
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
  /** Background replacement. Omit to print the photo as taken. */
  background?: BackgroundPayload | null;
  /** Exposure and white balance. Omit to print the photo as taken. */
  adjustments?: Adjustments | null;
}

export interface Adjustments {
  exposureEv: number;
  contrast: number;
  temperature: number;
  tint: number;
}

export function isNeutral(a: Adjustments): boolean {
  return a.exposureEv === 0 && a.contrast === 0 && a.temperature === 0 && a.tint === 0;
}

export interface Mask {
  width: number;
  height: number;
  data: Uint8Array;
  /** Execution provider that ran the model, or how the mask was produced. */
  backend: string;
  subjectRatio: number;
}

function toMask(r: {
  width: number;
  height: number;
  data: number[];
  backend: string;
  subject_ratio: number;
}): Mask {
  return {
    width: r.width,
    height: r.height,
    data: Uint8Array.from(r.data),
    backend: r.backend,
    subjectRatio: r.subject_ratio,
  };
}

type RawMask = Parameters<typeof toMask>[0];

/** Run background segmentation. Slow on the first call while the model loads. */
export async function segmentBackground(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
): Promise<Mask> {
  const r = await invoke<RawMask>("segment_background", {
    rgba: Array.from(rgba),
    width,
    height,
  });
  return toMask(r);
}

export async function addMaskStroke(stroke: {
  mode: "keep" | "erase";
  radius: number;
  feather: number;
  points: Array<[number, number]>;
}): Promise<Mask> {
  return toMask(await invoke<RawMask>("add_mask_stroke", { stroke }));
}

export async function undoMaskStroke(): Promise<Mask> {
  return toMask(await invoke<RawMask>("undo_mask_stroke"));
}

export async function resetMaskEdits(): Promise<Mask> {
  return toMask(await invoke<RawMask>("reset_mask_edits"));
}

/** Tighten or loosen the mask edge. 128 leaves the model's own edge alone. */
export async function setMaskThreshold(threshold: number): Promise<Mask> {
  return toMask(await invoke<RawMask>("set_mask_threshold", { threshold }));
}

export async function clearMask(): Promise<void> {
  return invoke<void>("clear_mask");
}

export async function backgroundUniformity(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
): Promise<number | null> {
  return invoke<number | null>("background_uniformity", {
    rgba: Array.from(rgba),
    width,
    height,
  });
}

/* Specs and validation. */

export interface SpecSummary {
  id: string;
  name: string;
  widthMm: number;
  heightMm: number;
  headHeightMm: number | null;
  minDpi: number;
  defaultCount: number;
  cutMarks: boolean;
  backgroundRgb: [number, number, number] | null;
  freeMode: boolean;
}

export async function listSpecs(lang = "hr"): Promise<SpecSummary[]> {
  const raw = await invoke<
    Array<{
      id: string;
      name: string;
      width_mm: number;
      height_mm: number;
      head_height_mm: number | null;
      min_dpi: number;
      default_count: number;
      cut_marks: boolean;
      background_rgb: [number, number, number] | null;
      free_mode: boolean;
    }>
  >("list_specs", { lang });

  return raw.map((s) => ({
    id: s.id,
    name: s.name,
    widthMm: s.width_mm,
    heightMm: s.height_mm,
    headHeightMm: s.head_height_mm,
    minDpi: s.min_dpi,
    defaultCount: s.default_count,
    cutMarks: s.cut_marks,
    backgroundRgb: s.background_rgb,
    freeMode: s.free_mode,
  }));
}

export type RuleStatus = "pass" | "warn" | "fail" | "not_checked";

export interface FixHint {
  kind: "set_head_height_mm" | "set_rotation_deg" | "set_dpi";
  value: number;
}

export interface RuleResult {
  ruleId: string;
  status: RuleStatus;
  severity: "error" | "warning";
  messageKey: string;
  params: Record<string, unknown>;
  fixHint: FixHint | null;
}

export interface Validation {
  results: RuleResult[];
  /** True when something failed at error severity. */
  blocking: boolean;
}

export interface ImageStats {
  clippedShadows: number;
  clippedHighlights: number;
  sharpness: number | null;
}

export async function analyseImage(
  rgba: Uint8ClampedArray,
  width: number,
  height: number,
  face?: { x: number; y: number; width: number; height: number } | null,
): Promise<ImageStats> {
  const r = await invoke<{
    clipped_shadows: number;
    clipped_highlights: number;
    sharpness: number | null;
  }>("analyse_image", {
    rgba: Array.from(rgba),
    width,
    height,
    faceX: face ? Math.max(0, Math.round(face.x)) : null,
    faceY: face ? Math.max(0, Math.round(face.y)) : null,
    faceWidth: face ? Math.round(face.width) : null,
    faceHeight: face ? Math.round(face.height) : null,
  });
  return {
    clippedShadows: r.clipped_shadows,
    clippedHighlights: r.clipped_highlights,
    sharpness: r.sharpness,
  };
}

export async function validatePhoto(req: {
  specId: string;
  headPx: number;
  cropX: number;
  cropY: number;
  cropWidth: number;
  cropHeight: number;
  headCentreX: number;
  headCentreY: number;
  eyeDistancePx: number;
  rollDeg: number;
  dpi: number;
  ageYears?: number | null;
  backgroundStddev?: number | null;
  sharpness?: number | null;
  clippedShadows?: number | null;
  clippedHighlights?: number | null;
}): Promise<Validation> {
  const r = await invoke<{
    results: Array<{
      rule_id: string;
      status: RuleStatus;
      severity: "error" | "warning";
      message_key: string;
      params: Record<string, unknown>;
      fix_hint: FixHint | null;
    }>;
    blocking: boolean;
  }>("validate_photo", {
    req: {
      spec_id: req.specId,
      head_px: req.headPx,
      crop_x: req.cropX,
      crop_y: req.cropY,
      crop_width: req.cropWidth,
      crop_height: req.cropHeight,
      head_centre_x: req.headCentreX,
      head_centre_y: req.headCentreY,
      eye_distance_px: req.eyeDistancePx,
      roll_deg: req.rollDeg,
      dpi: req.dpi,
      age_years: req.ageYears ?? null,
      background_stddev: req.backgroundStddev ?? null,
      sharpness: req.sharpness ?? null,
      clipped_shadows: req.clippedShadows ?? null,
      clipped_highlights: req.clippedHighlights ?? null,
    },
  });

  return {
    blocking: r.blocking,
    results: r.results.map((x) => ({
      ruleId: x.rule_id,
      status: x.status,
      severity: x.severity,
      messageKey: x.message_key,
      params: x.params,
      fixHint: x.fix_hint,
    })),
  };
}

/* Mixed sheets: several photo sizes on one piece of paper. */

export interface PhotoGroup {
  widthMm: number;
  heightMm: number;
  count: number;
}

export interface GroupedPlacement extends Placement {
  /** Index into the requested groups. */
  group: number;
}

export interface MixedLayout {
  placements: GroupedPlacement[];
  /** Copies of each group that did not fit, by group index. */
  unplaced: number[];
}

function toGroups(groups: PhotoGroup[]) {
  return groups.map((g) => ({
    width_mm: g.widthMm,
    height_mm: g.heightMm,
    count: g.count,
  }));
}

export async function solveMixedLayout(req: {
  paperWidthMm: number;
  paperHeightMm: number;
  groups: PhotoGroup[];
  marginMm: number;
  gutterMm: number;
}): Promise<MixedLayout> {
  const r = await invoke<{
    placements: Array<{
      x_mm: number;
      y_mm: number;
      width_mm: number;
      height_mm: number;
      rotated: boolean;
      group: number;
    }>;
    unplaced: number[];
  }>("solve_mixed_layout", {
    req: {
      paper_width_mm: req.paperWidthMm,
      paper_height_mm: req.paperHeightMm,
      groups: toGroups(req.groups),
      margin_mm: req.marginMm,
      gutter_mm: req.gutterMm,
    },
  });

  return {
    placements: r.placements.map((p) => ({
      xMm: p.x_mm,
      yMm: p.y_mm,
      widthMm: p.width_mm,
      heightMm: p.height_mm,
      rotated: p.rotated,
      group: p.group,
    })),
    unplaced: r.unplaced,
  };
}

/** Serialise a photo payload for either print command. */
function photoToWire(photo: PhotoPayload) {
  return {
    rgba: Array.from(photo.rgba),
    width: photo.width,
    height: photo.height,
    crop_x: photo.cropX,
    crop_y: photo.cropY,
    crop_width: photo.cropWidth,
    crop_height: photo.cropHeight,
    rotation_deg: photo.rotationDeg,
    background: photo.background
      ? {
          mask: Array.from(photo.background.mask),
          mask_width: photo.background.maskWidth,
          mask_height: photo.background.maskHeight,
          colour: photo.background.colour,
        }
      : null,
    adjustments: photo.adjustments
      ? {
          exposure_ev: photo.adjustments.exposureEv,
          contrast: photo.adjustments.contrast,
          temperature: photo.adjustments.temperature,
          tint: photo.adjustments.tint,
        }
      : null,
  };
}

export async function printMixedSheet(args: {
  printer: string;
  paperWidthMm: number;
  paperHeightMm: number;
  groups: PhotoGroup[];
  marginMm: number;
  gutterMm: number;
  photo?: PhotoPayload | null;
}): Promise<number> {
  return invoke<number>("print_mixed_sheet", {
    req: {
      printer: args.printer,
      paper_width_mm: args.paperWidthMm,
      paper_height_mm: args.paperHeightMm,
      groups: toGroups(args.groups),
      margin_mm: args.marginMm,
      gutter_mm: args.gutterMm,
      photo: args.photo ? photoToWire(args.photo) : null,
    },
  });
}

/* Presets: named sets of settings, stored in config.toml. */

export interface Preset {
  specId: string;
  photoWidthMm: number;
  photoHeightMm: number;
  headHeightMm: number;
  paperId: string;
  count: number;
  marginMm: number;
  gutterMm: number;
  alignTopLeft: boolean;
  cutMarks: boolean;
  replaceBackground: boolean;
  backgroundRgb: [number, number, number];
  exposureEv: number;
  contrast: number;
  temperature: number;
  tint: number;
}

export interface PresetSummary {
  name: string;
  savedAt: string;
}

interface RawPreset {
  spec_id: string;
  photo_width_mm: number;
  photo_height_mm: number;
  head_height_mm: number;
  paper_id: string;
  count: number;
  margin_mm: number;
  gutter_mm: number;
  align_top_left: boolean;
  cut_marks: boolean;
  replace_background: boolean;
  background_rgb: [number, number, number];
  exposure_ev: number;
  contrast: number;
  temperature: number;
  tint: number;
}

function fromRawPreset(r: RawPreset): Preset {
  return {
    specId: r.spec_id,
    photoWidthMm: r.photo_width_mm,
    photoHeightMm: r.photo_height_mm,
    headHeightMm: r.head_height_mm,
    paperId: r.paper_id,
    count: r.count,
    marginMm: r.margin_mm,
    gutterMm: r.gutter_mm,
    alignTopLeft: r.align_top_left,
    cutMarks: r.cut_marks,
    replaceBackground: r.replace_background,
    backgroundRgb: r.background_rgb,
    exposureEv: r.exposure_ev,
    contrast: r.contrast,
    temperature: r.temperature,
    tint: r.tint,
  };
}

function toRawPreset(p: Preset): RawPreset {
  return {
    spec_id: p.specId,
    photo_width_mm: p.photoWidthMm,
    photo_height_mm: p.photoHeightMm,
    head_height_mm: p.headHeightMm,
    paper_id: p.paperId,
    count: p.count,
    margin_mm: p.marginMm,
    gutter_mm: p.gutterMm,
    align_top_left: p.alignTopLeft,
    cut_marks: p.cutMarks,
    replace_background: p.replaceBackground,
    background_rgb: p.backgroundRgb,
    exposure_ev: p.exposureEv,
    contrast: p.contrast,
    temperature: p.temperature,
    tint: p.tint,
  };
}

function toSummaries(raw: Array<{ name: string; saved_at: string }>): PresetSummary[] {
  return raw.map((s) => ({ name: s.name, savedAt: s.saved_at }));
}

export async function listPresets(): Promise<PresetSummary[]> {
  return toSummaries(await invoke<Array<{ name: string; saved_at: string }>>("list_presets"));
}

export async function savePreset(name: string, preset: Preset): Promise<PresetSummary[]> {
  return toSummaries(
    await invoke<Array<{ name: string; saved_at: string }>>("save_preset", {
      name,
      preset: toRawPreset(preset),
      nowRfc3339: new Date().toISOString(),
    }),
  );
}

export async function loadPreset(name: string): Promise<Preset> {
  return fromRawPreset(await invoke<RawPreset>("load_preset", { name }));
}

export async function deletePreset(name: string): Promise<PresetSummary[]> {
  return toSummaries(
    await invoke<Array<{ name: string; saved_at: string }>>("delete_preset", { name }),
  );
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
      photo: args.photo ? photoToWire(args.photo) : null,
    },
  });
}
