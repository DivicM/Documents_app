import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Diagnostics } from "./components/Diagnostics";
import { DropZone } from "./components/DropZone";
import { FormatPicker } from "./components/FormatPicker";
import { PhotoCanvas, type Anchors, type HandleName } from "./components/PhotoCanvas";
import { PhotoPreview } from "./components/PhotoPreview";
import { Section } from "./components/Section";
import { SettingsDialog } from "./components/SettingsDialog";
import { SheetPreview } from "./components/SheetPreview";
import { Stepper } from "./components/Stepper";
import { StepBar } from "./components/StepBar";
import {
  canRedo,
  canUndo,
  createHistory,
  push,
  redo,
  statesEqual,
  undo,
  type EditState,
  type History,
} from "./lib/history";
import { formatError, t } from "./lib/i18n";
import * as ipc from "./lib/ipc";
import type {
  Calibration,
  CropResult,
  FaceDetection,
  Layout,
  Printer,
  PrinterCapabilities,
} from "./lib/ipc";
import { formatMm } from "./lib/units";

const PAPERS = [
  { id: "10x15", labelKey: "paper.10x15", widthMm: 100, heightMm: 150 },
  { id: "13x18", labelKey: "paper.13x18", widthMm: 130, heightMm: 180 },
  { id: "a4", labelKey: "paper.a4", widthMm: 210, heightMm: 297 },
] as const;

/**
 * The calibration square is always printed on A4, so its correction must be
 * stored under A4 too. Using the currently selected photo paper here would
 * save the measurement under a key the print path never reads back.
 */
const CALIBRATION_PAPER = { widthMm: 210, heightMm: 297 };

/**
 * Head height used for every crop, in millimetres.
 *
 * Fixed at the user's request: they frame the photograph themselves and do not
 * want the control. Note this sits above the Croatian legal range of 31.5–36mm
 * for a 35x45 photo, so the compliance panel will report head_height as failing
 * on Croatian specs. That is deliberate and the check is left working rather
 * than suppressed.
 */
const HEAD_HEIGHT_MM = 42;

/**
 * How much one press of the crop's +/- changes the head height, in millimetres.
 *
 * A larger head on the same print is a tighter crop, so these buttons adjust
 * the head height rather than scaling anything on screen.
 */
const HEAD_HEIGHT_STEP_MM = 1;
/** Bounds for that adjustment. Outside these the crop stops being usable. */
const HEAD_HEIGHT_MIN_MM = 20;
const HEAD_HEIGHT_MAX_MM = 60;

/** Text scale bounds, matching the clamp the backend applies on save. */
const FONT_SCALE_MIN = 50;
const FONT_SCALE_MAX = 200;

/** The spec whose print size the user types in themselves. */
const CUSTOM_SPEC_ID = "free-custom";
/** Bounds for that size. Wide enough for anything that fits on A4. */
const CUSTOM_MIN_MM = 20;
const CUSTOM_MAX_MM = 200;

/**
 * Parse a typed size, or return why it is not one.
 *
 * Returns the number when the text is a usable size, otherwise the i18n key
 * of the reason. Empty is its own case: while a field is being cleared it is
 * not yet wrong, so it must not be scolded the way "-5" is.
 */
function parseCustomMm(text: string): { mm: number } | { errorKey: string } {
  const trimmed = text.trim();
  if (trimmed === "") return { errorKey: "picker.custom_empty" };

  // Comma is the Croatian decimal separator, and the numeric keypad prints it.
  const value = Number(trimmed.replace(",", "."));
  // Rejects "12abc", "--", "1e5" oddities and anything Number() gives up on.
  if (!Number.isFinite(value)) return { errorKey: "picker.custom_not_a_number" };
  if (value <= 0) return { errorKey: "picker.custom_not_positive" };
  if (value < CUSTOM_MIN_MM || value > CUSTOM_MAX_MM) {
    return { errorKey: "picker.custom_out_of_range" };
  }
  return { mm: value };
}

/**
 * Longest edge of the copy used for detection, preview and editing.
 *
 * A 3600x5400 photograph is 74MB of RGBA. Running face detection, background
 * segmentation and every tone adjustment over all of it costs seconds per
 * keystroke, and none of that work needs the resolution: the models run at
 * 320-640px internally, and the preview is a few hundred pixels on screen.
 *
 * The full-resolution pixels are kept separately and used only for printing,
 * which happens once. 1600px is comfortably above what any of these consumers
 * needs while cutting the pixel count of a 3600x5400 photo by about 11x.
 */
const WORKING_MAX_EDGE_PX = 1600;

/** The reduced copy everything on the editing screen works from. */
interface WorkingImage {
  data: Uint8ClampedArray;
  w: number;
  h: number;
  /** The same pixels as a canvas, for drawing without another decode. */
  canvas: HTMLCanvasElement;
}

/** Downscale to at most `WORKING_MAX_EDGE_PX` on the longest edge. */
function toWorkingSize(img: HTMLImageElement): WorkingImage | null {
  const scale = Math.min(
    1,
    WORKING_MAX_EDGE_PX / Math.max(img.naturalWidth, img.naturalHeight),
  );
  const w = Math.max(1, Math.round(img.naturalWidth * scale));
  const h = Math.max(1, Math.round(img.naturalHeight * scale));

  const canvas = document.createElement("canvas");
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d", { willReadFrequently: true });
  if (!ctx) return null;
  ctx.imageSmoothingQuality = "high";
  ctx.drawImage(img, 0, 0, w, h);
  return { data: ctx.getImageData(0, 0, w, h).data, w, h, canvas };
}

/** The four steps, in order. */
const STEP_PICK = 0;
const STEP_FORMAT = 1;
const STEP_EDIT = 2;
const STEP_PRINT = 3;

export default function App() {
  const [step, setStep] = useState(STEP_PICK);
  /** Shown on the first screen once a file has been chosen. */
  const [imageName, setImageName] = useState<string | null>(null);

  const [printers, setPrinters] = useState<Printer[]>([]);
  const [selectedPrinter, setSelectedPrinter] = useState<string>("");
  const [caps, setCaps] = useState<PrinterCapabilities | null>(null);
  const [calibration, setCalibration] = useState<Calibration | null>(null);

  const [paperId, setPaperId] = useState<string>("10x15");
  const [photoWidthMm, setPhotoWidthMm] = useState(35);
  const [photoHeightMm, setPhotoHeightMm] = useState(45);
  /**
   * What is typed in the custom-size fields, as text.
   *
   * Kept separately from the numbers above so the field can be empty while
   * being edited: binding an input straight to a number turns "clear the
   * field" into the value 0, which cannot be deleted and is not a size anyone
   * meant. The number is only updated when the text parses to a valid one.
   */
  const [customWidthText, setCustomWidthText] = useState("35");
  const [customHeightText, setCustomHeightText] = useState("45");
  const [count, setCount] = useState(6);
  const [marginMm, setMarginMm] = useState(3);
  const [gutterMm, setGutterMm] = useState(2);
  const [alignTopLeft, setAlignTopLeft] = useState(false);
  const [cutMarks, setCutMarks] = useState(true);
  /**
   * Print a guide under each photo instead of around it.
   *
   * Separate from `cutMarks` rather than a mode of it, because the two are
   * asked for independently: the full frame wins when both are on, since it
   * already contains the bottom edge.
   */
  const [bottomMark, setBottomMark] = useState(false);

  const [layout, setLayout] = useState<Layout | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  /**
   * Set while a print job is being sent, to keep a second one from starting.
   *
   * Rendering a sheet at device resolution takes long enough to double-click
   * through, and every extra job is a wasted sheet of photo paper that cannot
   * be recalled once the spooler has it. One flag covers the sheet and both
   * calibration prints because they all reach the same printer.
   */
  const [printing, setPrinting] = useState(false);

  const [measuredX, setMeasuredX] = useState("50");
  const [measuredY, setMeasuredY] = useState("50");

  // Photo and face detection.
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  /**
   * The reduced copy shown and edited, in state so a render follows it.
   *
   * `imagePixels` holds the same thing in a ref for the callbacks that need it
   * synchronously; this is the version React draws from.
   */
  const [working, setWorking] = useState<WorkingImage | null>(null);
  const [detection, setDetection] = useState<FaceDetection | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [crop, setCrop] = useState<CropResult | null>(null);
  const [cropError, setCropError] = useState<string | null>(null);
  /** Head height the solver says would fit, offered as a one-click fix. */
  const [suggestedHeadMm, setSuggestedHeadMm] = useState<number | null>(null);
  const [headHeightMm, setHeadHeightMm] = useState(HEAD_HEIGHT_MM);
  /** User-dragged anchors. Missing entries fall back to the detected ones. */
  const [anchorOverride, setAnchorOverride] = useState<Partial<Anchors>>({});
  /** Explicit rotation, overriding the angle derived from the eye line. */
  const [rotationOverride, setRotationOverride] = useState<number | null>(null);

  // Background removal.
  const [mask, setMask] = useState<ipc.Mask | null>(null);
  const [segmenting, setSegmenting] = useState(false);
  const [replaceBackground, setReplaceBackground] = useState(true);
  const [bgColour, setBgColour] = useState<[number, number, number]>([255, 255, 255]);
  const [showMask, setShowMask] = useState(false);
  const [maskThreshold, setMaskThreshold] = useState(128);

  // Document specification and validation.
  const [specs, setSpecs] = useState<ipc.SpecSummary[]>([]);
  /** Empty string means the free "custom" mode. */
  const [specId, setSpecId] = useState("");
  /**
   * Rules still run on every edit; only the panel that displayed them was
   * removed from the UI. Keeping the result means the checks are one component
   * away from being shown again, and the IPC contract stays exercised.
   */
  const [, setValidation] = useState<ipc.Validation | null>(null);
  const [imageStats, setImageStats] = useState<ipc.ImageStats | null>(null);
  const [bgStddev, setBgStddev] = useState<number | null>(null);

  // Exposure and white balance.
  const [exposureEv, setExposureEv] = useState(0);
  const [contrast, setContrast] = useState(0);
  const [temperature, setTemperature] = useState(0);
  const [tint, setTint] = useState(0);

  // Presets.
  const [presets, setPresets] = useState<ipc.PresetSummary[]>([]);
  const [presetName, setPresetName] = useState("");

  // Mixed sheet: several formats on one piece of paper.
  const [mixedMode, setMixedMode] = useState(false);
  const [groups, setGroups] = useState<ipc.PhotoGroup[]>([
    { widthMm: 35, heightMm: 45, count: 2 },
    { widthMm: 30, heightMm: 35, count: 4 },
  ]);
  const [mixedLayout, setMixedLayout] = useState<ipc.MixedLayout | null>(null);

  // Settings that persist between sessions.
  const [settingsOpen, setSettingsOpen] = useState(false);
  // Both off: photographs print upright, the way they are looked at. The
  // solver no longer rotates for density either, so a sheet holds fewer
  // copies but every face stands the right way up.
  const [quarterTurn, setQuarterTurn] = useState(false);
  const [turnPhoto, setTurnPhoto] = useState(false);
  const [settingsStatus, setSettingsStatus] = useState<string | null>(null);
  const [fontScale, setFontScale] = useState(100);
  const [startMaximized, setStartMaximized] = useState(false);
  const [theme, setTheme] = useState<"light" | "dark">("light");

  // Undo/redo over edit parameters. Snapshots are cheap because they hold
  // numbers, not pixels.
  const [history, setHistory] = useState<History<EditState>>(() =>
    createHistory<EditState>({
      anchorOverride: {},
      rotationOverride: null,
      headHeightMm: HEAD_HEIGHT_MM,
      photoWidthMm: 35,
      photoHeightMm: 45,
      count: 6,
      marginMm: 3,
      gutterMm: 2,
      alignTopLeft: false,
      cutMarks: true,
      replaceBackground: true,
      bgColour: [255, 255, 255],
      exposureEv: 0,
      contrast: 0,
      temperature: 0,
      tint: 0,
    }),
  );
  /** Set while applying an undo, so the restore is not recorded as an edit. */
  const restoring = useRef(false);
  /**
   * The working copy's pixels, for callbacks that need them synchronously.
   *
   * Reduced, not full resolution: detection, segmentation and tone all run on
   * these. Printing reads the original separately.
   */
  const imagePixels = useRef<WorkingImage | null>(null);
  /**
   * The photo with its background already replaced, used for previewing.
   *
   * Built here rather than in the preview component so both the sheet and the
   * printer work from the same composited pixels.
   */
  const [composited, setComposited] = useState<HTMLCanvasElement | null>(null);

  // The rule from the brief: an override wins, otherwise the automatic value.
  const anchors: Anchors | null = useMemo(() => {
    if (!detection) return null;
    return {
      chin: anchorOverride.chin ?? detection.chin,
      crown: anchorOverride.crown ?? detection.crown,
      rightEye: anchorOverride.rightEye ?? detection.rightEye,
      leftEye: anchorOverride.leftEye ?? detection.leftEye,
    };
  }, [detection, anchorOverride]);

  /**
   * Straightening angle: the user's explicit value if set, otherwise derived
   * from the eye line. Deriving rather than reusing the detected roll means
   * dragging an eye updates the tilt, which is why the eyes are draggable.
   */
  const rotationDeg = useMemo(() => {
    if (rotationOverride !== null) return rotationOverride;
    if (!anchors) return 0;
    const dx = anchors.leftEye.x - anchors.rightEye.x;
    const dy = anchors.leftEye.y - anchors.rightEye.y;
    return (Math.atan2(dy, dx) * 180) / Math.PI;
  }, [rotationOverride, anchors]);

  const chosenPaper = useMemo(
    () => PAPERS.find((p) => p.id === paperId) ?? PAPERS[0],
    [paperId],
  );

  /**
   * The paper actually used, which is what the driver reports when it can.
   *
   * A dye-sublimation photo printer is configured for one paper size and
   * ignores anything else sent to it, so laying out for the size picked in the
   * app is what put photographs off the edge of the sheet. The preview must
   * show the same sheet the printer will use.
   */
  const paper = useMemo(() => {
    if (caps && caps.paperWidthMm > 1 && caps.paperHeightMm > 1) {
      return {
        id: chosenPaper.id,
        labelKey: chosenPaper.labelKey,
        widthMm: caps.paperWidthMm,
        heightMm: caps.paperHeightMm,
      };
    }
    return chosenPaper;
  }, [caps, chosenPaper]);

  /** True when the printer overrode the chosen paper, so the UI can say so. */
  const paperOverridden =
    Math.abs(paper.widthMm - chosenPaper.widthMm) > 0.5 ||
    Math.abs(paper.heightMm - chosenPaper.heightMm) > 0.5;

  const refreshPrinters = useCallback(async () => {
    try {
      const list = await ipc.listPrinters();
      setPrinters(list);
      const preferred = list.find((p) => p.isDefault) ?? list[0];
      if (preferred) setSelectedPrinter((current) => current || preferred.name);
    } catch (e) {
      setError(formatError(e));
    }
  }, []);

  useEffect(() => {
    void refreshPrinters();
  }, [refreshPrinters]);

  // Load the bundled specs once.
  useEffect(() => {
    ipc
      .listSpecs("hr")
      .then(setSpecs)
      .catch((e) => setError(formatError(e)));
  }, []);

  /** Selecting a spec fills in its dimensions, so the numbers come from one place. */
  const onSelectSpec = useCallback(
    (id: string) => {
      setSpecId(id);
      setValidation(null);
      if (!id) return;
      const s = specs.find((x) => x.id === id);
      if (!s) return;
      setPhotoWidthMm(s.widthMm);
      setPhotoHeightMm(s.heightMm);
      // Seed the custom fields from whatever was last selected, so switching
      // to the custom format starts from a real size rather than stale text.
      setCustomWidthText(String(s.widthMm));
      setCustomHeightText(String(s.heightMm));
      // Head height stays at HEAD_HEIGHT_MM rather than following the spec:
      // the user frames the photograph themselves.
      setCount(s.defaultCount);
      setCutMarks(s.cutMarks);
      if (s.backgroundRgb) setBgColour(s.backgroundRgb);
    },
    [specs],
  );

  // Load the saved presets once.
  useEffect(() => {
    ipc
      .listPresets()
      .then(setPresets)
      .catch((e) => setError(formatError(e)));
  }, []);

  /**
   * Apply the text scale by overriding the root font size.
   *
   * Text sizes in the stylesheet are in `em`, so they follow this one value.
   * An earlier attempt used `zoom`, which scaled photographs and layout along
   * with the type and so read as magnifying the window rather than enlarging
   * the text.
   *
   * 14px is the design base declared on `:root`.
   */
  useEffect(() => {
    document.documentElement.style.fontSize = `${(14 * fontScale) / 100}px`;
  }, [fontScale]);

  /**
   * Apply the theme by setting an attribute the stylesheet keys off.
   *
   * All colours are CSS variables, so the dark theme is a block of overrides
   * rather than a second stylesheet — nothing here has to know which rules
   * exist.
   */
  useEffect(() => {
    document.documentElement.dataset.theme = theme;
  }, [theme]);

  // Restore the persisted sheet settings on startup.
  useEffect(() => {
    ipc
      .getSheetSettings()
      .then((s) => {
        setPaperId(s.paperId);
        setCount(s.count);
        setMarginMm(s.marginMm);
        setGutterMm(s.gutterMm);
        setAlignTopLeft(s.alignTopLeft);
        setCutMarks(s.cutMarks);
        setQuarterTurn(s.quarterTurn);
        setFontScale(s.fontScalePercent);
        setStartMaximized(s.startMaximized);
        setTheme(s.theme);
        setTurnPhoto(s.turnPhoto);
        // Only if that printer is still installed; otherwise the default
        // chosen by refreshPrinters stands.
        if (s.printer) setSelectedPrinter((current) => current || s.printer);
      })
      .catch(() => {
        // Unreadable settings must not stop the app; defaults are fine.
      });
  }, []);

  /** Write the sheet settings so they survive a restart. */
  const onSaveSettings = useCallback(async () => {
    setSettingsStatus(null);
    try {
      await ipc.saveSheetSettings({
        paperId,
        count,
        marginMm,
        gutterMm,
        alignTopLeft,
        cutMarks,
        quarterTurn,
        turnPhoto,
        printer: selectedPrinter,
        fontScalePercent: fontScale,
        startMaximized,
        theme,
      });
      setSettingsStatus(t("settings.saved"));
    } catch (e) {
      setSettingsStatus(formatError(e));
    }
  }, [
    paperId,
    count,
    marginMm,
    gutterMm,
    alignTopLeft,
    cutMarks,
    quarterTurn,
    turnPhoto,
    selectedPrinter,
    fontScale,
    startMaximized,
    theme,
  ]);

  // Recompute the mixed layout whenever the groups or paper change.
  useEffect(() => {
    if (!mixedMode) {
      setMixedLayout(null);
      return;
    }
    let cancelled = false;

    (async () => {
      try {
        const result = await ipc.solveMixedLayout({
          paperWidthMm: paper.widthMm,
          paperHeightMm: paper.heightMm,
          groups,
          marginMm,
          gutterMm,
          quarterTurn,
        });
        if (!cancelled) {
          setMixedLayout(result);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) {
          setMixedLayout(null);
          setError(formatError(e));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [mixedMode, groups, paper.widthMm, paper.heightMm, marginMm, gutterMm, quarterTurn]);

  // Printer capabilities and calibration both depend on printer + paper.
  useEffect(() => {
    if (!selectedPrinter) return;
    let cancelled = false;

    (async () => {
      try {
        // Queried with the chosen size, not the effective one: the effective
        // size is derived from this answer, and using it here would be
        // circular. The backend only uses these as a fallback anyway.
        const c = await ipc.printerCapabilities(
          selectedPrinter,
          chosenPaper.widthMm,
          chosenPaper.heightMm,
        );
        if (!cancelled) setCaps(c);
      } catch {
        if (!cancelled) setCaps(null);
      }

      try {
        // Always A4: that is the paper the calibration square is printed on.
        const cal = await ipc.getCalibration(
          selectedPrinter,
          CALIBRATION_PAPER.widthMm,
          CALIBRATION_PAPER.heightMm,
          false,
        );
        if (!cancelled) setCalibration(cal);
      } catch {
        if (!cancelled) setCalibration(null);
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [selectedPrinter, chosenPaper.widthMm, chosenPaper.heightMm]);

  // Recompute the layout whenever any input changes.
  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const result = await ipc.solveLayout({
          paperWidthMm: paper.widthMm,
          paperHeightMm: paper.heightMm,
          photoWidthMm,
          photoHeightMm,
          count,
          marginMm,
          gutterMm,
          alignTopLeft,
          quarterTurn,
        });
        if (!cancelled) {
          setLayout(result);
          setError(null);
        }
      } catch (e) {
        if (!cancelled) {
          setLayout(null);
          setError(formatError(e));
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [
    paper.widthMm,
    paper.heightMm,
    photoWidthMm,
    photoHeightMm,
    count,
    marginMm,
    gutterMm,
    alignTopLeft,
    quarterTurn,
  ]);

  /** Read the image into a canvas and hand the raw pixels to the detector. */
  const runDetection = useCallback(async (img: HTMLImageElement) => {
    setDetecting(true);
    setError(null);
    setDetection(null);
    setCrop(null);
    setCropError(null);
    setAnchorOverride({});
    // A new photo must not inherit the previous photo's mask.
    setMask(null);
    setShowMask(false);
    setMaskThreshold(128);
    void ipc.clearMask().catch(() => {});

    try {
      // Everything on this screen works from the reduced copy. The
      // full-resolution pixels are read only when printing.
      const work = toWorkingSize(img);
      if (!work) throw new Error(t("error.image.canvas_unavailable"));
      imagePixels.current = work;
      setWorking(work);

      const found = await ipc.detectFace(work.data, work.w, work.h);
      setDetection(found);
      if (!found) setError(t("face.none_found"));

      // Sharpness and exposure feed the validator; measured over the face so a
      // busy background cannot make a soft portrait look sharp.
      try {
        setImageStats(
          await ipc.analyseImage(work.data, work.w, work.h, found?.faceBox ?? null),
        );
      } catch {
        setImageStats(null);
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setDetecting(false);
    }
  }, []);

  const onPickImage = async (file: File) => {
    const url = URL.createObjectURL(file);
    const img = new Image();
    img.onload = () => {
      setImage(img);
      setImageName(file.name);
      void runDetection(img);
      // The bitmap is decoded; the blob URL is no longer needed.
      URL.revokeObjectURL(url);
    };
    img.onerror = () => {
      setError(t("error.image.decode_failed"));
      URL.revokeObjectURL(url);
    };
    img.src = url;
  };

  /**
   * Return to the first screen for a new photo.
   *
   * Clears the image and everything derived from it, but deliberately keeps the
   * chosen format, printer and sheet settings: the next photo is usually for
   * the same document on the same paper.
   */
  const startOver = useCallback(() => {
    setStep(STEP_PICK);
    setImage(null);
    setImageName(null);
    setWorking(null);
    setDetection(null);
    setCrop(null);
    setCropError(null);
    setAnchorOverride({});
    setRotationOverride(null);
    setMask(null);
    setShowMask(false);
    setMaskThreshold(128);
    setValidation(null);
    setImageStats(null);
    setBgStddev(null);
    setExposureEv(0);
    setContrast(0);
    setTemperature(0);
    setTint(0);
    setStatus(null);
    setError(null);
    imagePixels.current = null;
    void ipc.clearMask().catch(() => {});
  }, []);

  // Recompute the crop whenever the anchors or the requested head size change.
  useEffect(() => {
    const px = imagePixels.current;
    if (!image || !anchors || !px) {
      setCrop(null);
      return;
    }
    let cancelled = false;

    (async () => {
      try {
        // Working-copy dimensions, not the original's: the anchors came from
        // detection on that copy, and mixing the two coordinate spaces would
        // put the crop in the wrong place.
        const result = await ipc.computeCrop({
          chinX: anchors.chin.x,
          chinY: anchors.chin.y,
          crownX: anchors.crown.x,
          crownY: anchors.crown.y,
          imageWidth: px.w,
          imageHeight: px.h,
          photoWidthMm: photoWidthMm,
          photoHeightMm: photoHeightMm,
          headHeightMm,
        });
        if (!cancelled) {
          setCrop(result);
          setCropError(null);
          setSuggestedHeadMm(null);
        }
      } catch (e) {
        if (!cancelled) {
          setCrop(null);
          setCropError(formatError(e));
          // The overflow error carries the head height that would fit, so the
          // user can apply it instead of guessing.
          const params =
            e && typeof e === "object" && "params" in e
              ? (e as { params?: Record<string, unknown> }).params
              : undefined;
          const suggested = params?.minHeadMm;
          setSuggestedHeadMm(typeof suggested === "number" ? suggested : null);
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [image, anchors, photoWidthMm, photoHeightMm, headHeightMm]);

  /** The edit state as it stands right now. */
  const currentState: EditState = useMemo(
    () => ({
      anchorOverride,
      rotationOverride,
      headHeightMm,
      photoWidthMm,
      photoHeightMm,
      count,
      marginMm,
      gutterMm,
      alignTopLeft,
      cutMarks,
      replaceBackground,
      bgColour,
      exposureEv,
      contrast,
      temperature,
      tint,
    }),
    [
      anchorOverride,
      rotationOverride,
      headHeightMm,
      photoWidthMm,
      photoHeightMm,
      count,
      marginMm,
      gutterMm,
      alignTopLeft,
      cutMarks,
      replaceBackground,
      bgColour,
      exposureEv,
      contrast,
      temperature,
      tint,
    ],
  );

  // Record a snapshot once the state settles. Debounced so dragging a slider
  // produces one undo step rather than one per pixel of travel.
  useEffect(() => {
    if (restoring.current) {
      restoring.current = false;
      return;
    }
    if (statesEqual(history.present, currentState)) return;

    const timer = window.setTimeout(() => {
      setHistory((h) => (statesEqual(h.present, currentState) ? h : push(h, currentState)));
    }, 350);
    return () => window.clearTimeout(timer);
  }, [currentState, history.present]);

  /** Apply a state from the history back onto the individual controls. */
  const applyState = useCallback((s: EditState) => {
    restoring.current = true;
    setAnchorOverride(s.anchorOverride);
    setRotationOverride(s.rotationOverride);
    setHeadHeightMm(s.headHeightMm);
    setPhotoWidthMm(s.photoWidthMm);
    setPhotoHeightMm(s.photoHeightMm);
    // The fields show the restored size, not what was typed before the undo.
    setCustomWidthText(String(s.photoWidthMm));
    setCustomHeightText(String(s.photoHeightMm));
    setCount(s.count);
    setMarginMm(s.marginMm);
    setGutterMm(s.gutterMm);
    setAlignTopLeft(s.alignTopLeft);
    setCutMarks(s.cutMarks);
    setReplaceBackground(s.replaceBackground);
    setBgColour(s.bgColour);
    setExposureEv(s.exposureEv);
    setContrast(s.contrast);
    setTemperature(s.temperature);
    setTint(s.tint);
  }, []);

  const onUndo = useCallback(() => {
    setHistory((h) => {
      if (!canUndo(h)) return h;
      const next = undo(h);
      applyState(next.present);
      return next;
    });
  }, [applyState]);

  const onRedo = useCallback(() => {
    setHistory((h) => {
      if (!canRedo(h)) return h;
      const next = redo(h);
      applyState(next.present);
      return next;
    });
  }, [applyState]);

  // Ctrl+Z / Ctrl+Y, the shortcuts users reach for without being told.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;
      const key = e.key.toLowerCase();
      if (key === "z" && !e.shiftKey) {
        e.preventDefault();
        onUndo();
      } else if (key === "y" || (key === "z" && e.shiftKey)) {
        e.preventDefault();
        onRedo();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onUndo, onRedo]);

  const adjustments = useMemo(
    () => ({ exposureEv, contrast, temperature, tint }),
    [exposureEv, contrast, temperature, tint],
  );

  // Rebuild the previewed photo whenever the mask, colour or tone changes.
  //
  // Applies the same steps in the same order as the printer: tone, then
  // background, then crop. Diverging here is the classic "right on screen,
  // wrong on paper" bug.
  useEffect(() => {
    const px = imagePixels.current;
    const needsWork = (replaceBackground && mask) || !ipc.isNeutral(adjustments);
    if (!needsWork || !px || !image) {
      setComposited(null);
      return;
    }
    let cancelled = false;

    const canvas = document.createElement("canvas");
    canvas.width = px.w;
    canvas.height = px.h;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const out = ctx.createImageData(px.w, px.h);
    out.data.set(px.data);

    // Tone first, mirroring the Rust path.
    if (!ipc.isNeutral(adjustments)) {
      const exposure = Math.pow(2, adjustments.exposureEv);
      const gains = [
        1 + adjustments.temperature * 0.3,
        1 + adjustments.tint * 0.3,
        1 - adjustments.temperature * 0.3,
      ];
      const contrastGain = 1 + adjustments.contrast;
      // One lookup table per channel, as in the renderer.
      const lut = [0, 1, 2].map((c) => {
        const table = new Uint8ClampedArray(256);
        for (let v = 0; v < 256; v++) {
          let x = (v / 255) * exposure * gains[c];
          x = 0.5 + (x - 0.5) * contrastGain;
          table[v] = Math.round(Math.min(255, Math.max(0, x * 255)));
        }
        return table;
      });
      for (let i = 0; i < out.data.length; i += 4) {
        out.data[i] = lut[0][out.data[i]];
        out.data[i + 1] = lut[1][out.data[i + 1]];
        out.data[i + 2] = lut[2][out.data[i + 2]];
      }
    }

    if (!replaceBackground || !mask) {
      ctx.putImageData(out, 0, 0);
      if (!cancelled) setComposited(canvas);
      return () => {
        cancelled = true;
      };
    }

    // Bilinear, matching composite_background in mask.rs. The mask is roughly
    // six times smaller than the photo, and picking the nearest texel shows
    // that magnification as blocky stair-steps along the hair and chin.
    const sx = mask.width / px.w;
    const sy = mask.height / px.h;
    const sampleMask = (u: number, v: number): number => {
      const mx = Math.min(Math.max(u - 0.5, 0), mask.width - 1);
      const my = Math.min(Math.max(v - 0.5, 0), mask.height - 1);
      const x0 = Math.floor(mx);
      const y0 = Math.floor(my);
      const x1 = Math.min(x0 + 1, mask.width - 1);
      const y1 = Math.min(y0 + 1, mask.height - 1);
      const fx = mx - x0;
      const fy = my - y0;
      const top =
        mask.data[y0 * mask.width + x0] * (1 - fx) + mask.data[y0 * mask.width + x1] * fx;
      const bottom =
        mask.data[y1 * mask.width + x0] * (1 - fx) + mask.data[y1 * mask.width + x1] * fx;
      return top * (1 - fy) + bottom * fy;
    };

    for (let y = 0; y < px.h; y++) {
      // Sample from the centre of each destination pixel, not its corner.
      const v = (y + 0.5) * sy;
      for (let x = 0; x < px.w; x++) {
        const alpha = sampleMask((x + 0.5) * sx, v) / 255;
        const i = (y * px.w + x) * 4;
        for (let c = 0; c < 3; c++) {
          // Blend from the already-toned pixel, not the original, so tone and
          // background compose the same way they do in the renderer.
          out.data[i + c] = Math.round(
            bgColour[c] + (out.data[i + c] - bgColour[c]) * alpha,
          );
        }
      }
    }
    ctx.putImageData(out, 0, 0);
    if (!cancelled) setComposited(canvas);

    return () => {
      cancelled = true;
    };
  }, [mask, replaceBackground, bgColour, image, adjustments]);

  // Re-validate whenever anything a rule depends on changes.
  useEffect(() => {
    if (!specId || !crop || !anchors || !detection) {
      setValidation(null);
      return;
    }
    let cancelled = false;

    (async () => {
      try {
        const headPx = Math.abs(anchors.chin.y - anchors.crown.y);
        const eyeDx = anchors.leftEye.x - anchors.rightEye.x;
        const eyeDy = anchors.leftEye.y - anchors.rightEye.y;
        const result = await ipc.validatePhoto({
          specId,
          headPx,
          cropX: crop.rect.x,
          cropY: crop.rect.y,
          cropWidth: crop.rect.width,
          cropHeight: crop.rect.height,
          headCentreX: (anchors.chin.x + anchors.crown.x) / 2,
          headCentreY: (anchors.chin.y + anchors.crown.y) / 2,
          eyeDistancePx: Math.hypot(eyeDx, eyeDy),
          rollDeg: rotationDeg,
          dpi: 300,
          backgroundStddev: bgStddev,
          sharpness: imageStats?.sharpness ?? null,
          clippedShadows: imageStats?.clippedShadows ?? null,
          clippedHighlights: imageStats?.clippedHighlights ?? null,
        });
        if (!cancelled) setValidation(result);
      } catch (e) {
        if (!cancelled) setError(formatError(e));
      }
    })();

    return () => {
      cancelled = true;
    };
  }, [specId, crop, anchors, detection, rotationDeg, bgStddev, imageStats]);

  const onAnchorMove = useCallback((which: HandleName, x: number, y: number) => {
    setAnchorOverride((prev) => ({ ...prev, [which]: { x, y } }));
  }, []);

  const resetAnchor = (which: HandleName) => {
    setAnchorOverride((prev) => {
      const next = { ...prev };
      delete next[which];
      return next;
    });
  };

  /**
   * Move the crop by shifting both anchors together.
   *
   * The crop is derived from the anchors rather than stored, so nudging it
   * means moving what it is built from. Doing it this way keeps a single
   * source of truth instead of a crop that could drift from the head points.
   */
  const onCropNudge = useCallback(
    (dx: number, dy: number) => {
      const px = imagePixels.current;
      if (!anchors || !px) return;
      // Anchors live in working-copy coordinates, so clamp to those bounds.
      const clampX = (v: number) => Math.max(0, Math.min(px.w, v));
      const clampY = (v: number) => Math.max(0, Math.min(px.h, v));
      setAnchorOverride((prev) => ({
        ...prev,
        chin: {
          x: clampX(anchors.chin.x + dx),
          y: clampY(anchors.chin.y + dy),
        },
        crown: {
          x: clampX(anchors.crown.x + dx),
          y: clampY(anchors.crown.y + dy),
        },
      }));
    },
    [anchors],
  );

  const hasAnyOverride =
    Object.keys(anchorOverride).length > 0 || rotationOverride !== null;

  const runSegmentation = useCallback(async () => {
    const px = imagePixels.current;
    if (!px) return;
    setSegmenting(true);
    setError(null);
    try {
      const m = await ipc.segmentBackground(px.data, px.w, px.h);
      setMask(m);
      setShowMask(true);
      // With a mask available, background uniformity becomes measurable.
      try {
        setBgStddev(await ipc.backgroundUniformity(px.data, px.w, px.h));
      } catch {
        setBgStddev(null);
      }
    } catch (e) {
      setError(formatError(e));
    } finally {
      setSegmenting(false);
    }
  }, []);

  const onThresholdChange = async (value: number) => {
    setMaskThreshold(value);
    try {
      setMask(await ipc.setMaskThreshold(value));
    } catch (e) {
      setError(formatError(e));
    }
  };

  const onPrintSquare = async (applyCalibration: boolean) => {
    if (!selectedPrinter || printing) return;
    setStatus(null);
    setError(null);
    setPrinting(true);
    try {
      const jobId = await ipc.printCalibrationSquare(selectedPrinter, applyCalibration);
      setStatus(t("print.sent", { jobId }));
    } catch (e) {
      setError(formatError(e));
    } finally {
      setPrinting(false);
    }
  };

  const onSaveCalibration = async () => {
    if (!selectedPrinter) return;
    setStatus(null);
    setError(null);
    try {
      const cal = await ipc.saveCalibration({
        printer: selectedPrinter,
        paperWidthMm: CALIBRATION_PAPER.widthMm,
        paperHeightMm: CALIBRATION_PAPER.heightMm,
        borderless: false,
        nominalMm: 50,
        measuredXMm: Number(measuredX.replace(",", ".")),
        measuredYMm: Number(measuredY.replace(",", ".")),
      });
      setCalibration(cal);
      setStatus(t("calibration.saved"));
    } catch (e) {
      setError(formatError(e));
    }
  };

  /**
   * The photo payload for printing, or null when there is nothing to composite.
   *
   * Shared by the manual, mixed and automatic print paths so all three send
   * identical pixels.
   */
  const buildPhotoPayload = useCallback((): ipc.PhotoPayload | null => {
    const px = imagePixels.current;
    if (!px || !crop || !image) return null;

    // Printing is the one place that reads the photograph at full resolution.
    // Everything on screen works from the reduced copy for speed, but the
    // print must not inherit that loss, so the original is read here and the
    // crop — expressed in working-copy pixels — is scaled up to match.
    const full = document.createElement("canvas");
    full.width = image.naturalWidth;
    full.height = image.naturalHeight;
    const fctx = full.getContext("2d", { willReadFrequently: true });
    if (!fctx) return null;
    fctx.drawImage(image, 0, 0);
    const fullData = fctx.getImageData(0, 0, full.width, full.height).data;

    // One factor for both axes: toWorkingSize scales uniformly.
    const k = image.naturalWidth / px.w;

    return {
      rgba: fullData,
      width: full.width,
      height: full.height,
      cropX: crop.rect.x * k,
      cropY: crop.rect.y * k,
      cropWidth: crop.rect.width * k,
      cropHeight: crop.rect.height * k,
      rotationDeg,
      background:
        replaceBackground && mask
          ? {
              mask: mask.data,
              maskWidth: mask.width,
              maskHeight: mask.height,
              colour: bgColour,
            }
          : null,
      adjustments: ipc.isNeutral(adjustments) ? null : adjustments,
    };
  }, [crop, image, rotationDeg, replaceBackground, mask, bgColour, adjustments]);

  const onPrintSheet = useCallback(async () => {
    if (!selectedPrinter || printing) return;
    setStatus(null);
    setError(null);
    setPrinting(true);
    try {
      // Print the photo when one is loaded and its crop is valid; otherwise
      // fall back to plain rectangles so the layout can still be checked.
      const photo = buildPhotoPayload();

      const jobId = mixedMode
        ? await ipc.printMixedSheet({
            printer: selectedPrinter,
            paperWidthMm: paper.widthMm,
            paperHeightMm: paper.heightMm,
            groups,
            marginMm,
            gutterMm,
            quarterTurn,
            turnPhoto,
            cutMarks,
            bottomMark,
            photo,
          })
        : await ipc.printSheet({
            printer: selectedPrinter,
            paperWidthMm: paper.widthMm,
            paperHeightMm: paper.heightMm,
            photoWidthMm,
            photoHeightMm,
            count,
            marginMm,
            gutterMm,
            alignTopLeft,
            quarterTurn,
            turnPhoto,
            cutMarks,
            bottomMark,
            photo,
          });
      setStatus(t("print.sent", { jobId }));
      return jobId;
    } catch (e) {
      setError(formatError(e));
      return null;
    } finally {
      setPrinting(false);
    }
  }, [
    selectedPrinter,
    printing,
    buildPhotoPayload,
    mixedMode,
    paper.widthMm,
    paper.heightMm,
    groups,
    photoWidthMm,
    photoHeightMm,
    count,
    marginMm,
    gutterMm,
    alignTopLeft,
    quarterTurn,
    turnPhoto,
    cutMarks,
    bottomMark,
  ]);

  const onSavePreset = async () => {
    setStatus(null);
    setError(null);
    try {
      const list = await ipc.savePreset(presetName, {
        specId,
        photoWidthMm,
        photoHeightMm,
        headHeightMm,
        paperId,
        count,
        marginMm,
        gutterMm,
        alignTopLeft,
        cutMarks,
        replaceBackground,
        backgroundRgb: bgColour,
        exposureEv,
        contrast,
        temperature,
        tint,
      });
      setPresets(list);
      setStatus(t("preset.saved", { name: presetName.trim() }));
    } catch (e) {
      setError(formatError(e));
    }
  };

  const onLoadPreset = async (name: string) => {
    if (!name) return;
    setStatus(null);
    setError(null);
    try {
      const p = await ipc.loadPreset(name);
      // Applied through the same setters the controls use, so undo/redo records
      // loading a preset as one ordinary edit.
      setSpecId(p.specId);
      setPhotoWidthMm(p.photoWidthMm);
      setPhotoHeightMm(p.photoHeightMm);
      // A preset can carry the custom format, so the fields must show its size.
      setCustomWidthText(String(p.photoWidthMm));
      setCustomHeightText(String(p.photoHeightMm));
      setHeadHeightMm(p.headHeightMm);
      setPaperId(p.paperId);
      setCount(p.count);
      setMarginMm(p.marginMm);
      setGutterMm(p.gutterMm);
      setAlignTopLeft(p.alignTopLeft);
      setCutMarks(p.cutMarks);
      setReplaceBackground(p.replaceBackground);
      setBgColour(p.backgroundRgb);
      setExposureEv(p.exposureEv);
      setContrast(p.contrast);
      setTemperature(p.temperature);
      setTint(p.tint);
      setPresetName(name);
      setStatus(t("preset.loaded", { name }));
    } catch (e) {
      setError(formatError(e));
    }
  };

  const onDeletePreset = async (name: string) => {
    if (!name || !window.confirm(t("preset.confirm_delete", { name }))) return;
    setStatus(null);
    setError(null);
    try {
      setPresets(await ipc.deletePreset(name));
      setStatus(t("preset.deleted", { name }));
    } catch (e) {
      setError(formatError(e));
    }
  };

  const updateGroup = (index: number, patch: Partial<ipc.PhotoGroup>) => {
    setGroups((gs) => gs.map((g, i) => (i === index ? { ...g, ...patch } : g)));
  };

  const rotated = layout?.placements[0]?.rotated ?? false;

  /** Whether the current step is complete enough to move on. */
  const customWidth = useMemo(() => parseCustomMm(customWidthText), [customWidthText]);
  const customHeight = useMemo(() => parseCustomMm(customHeightText), [customHeightText]);

  /**
   * Accept the typed text, and commit it as the print size when it is valid.
   *
   * The text is always kept, so a half-typed or empty field is not fought
   * with; the number behind it simply stops following until the text makes
   * sense again. That leaves the last good size in place rather than a zero.
   */
  const onCustomWidth = useCallback((text: string) => {
    setCustomWidthText(text);
    const parsed = parseCustomMm(text);
    if ("mm" in parsed) setPhotoWidthMm(parsed.mm);
  }, []);

  const onCustomHeight = useCallback((text: string) => {
    setCustomHeightText(text);
    const parsed = parseCustomMm(text);
    if ("mm" in parsed) setPhotoHeightMm(parsed.mm);
  }, []);

  const canAdvance =
    step === STEP_PICK
      ? image !== null
      : step === STEP_FORMAT
        ? // A custom format must have two usable sizes before moving on,
          // otherwise the next step would lay out the last valid size while
          // the field on screen says something else.
          specId !== "" &&
          (specId !== CUSTOM_SPEC_ID ||
            ("mm" in customWidth && "mm" in customHeight))
        : step === STEP_EDIT
          ? crop !== null
          : false;

  /** Tighten or loosen the crop by one step. */
  const onCropZoom = useCallback((direction: 1 | -1) => {
    setHeadHeightMm((mm) =>
      Math.min(
        HEAD_HEIGHT_MAX_MM,
        Math.max(HEAD_HEIGHT_MIN_MM, mm + direction * HEAD_HEIGHT_STEP_MM),
      ),
    );
  }, []);

  /** Print, then return to the start ready for the next person. */
  const onPrintAndFinish = async () => {
    const jobId = await onPrintSheet();
    if (jobId === null || jobId === undefined) return;
    startOver();
    setStatus(t("print.done"));
  };

  return (
    <main className="app">
      {/* A menu bar rather than a title row: Settings belongs where an
          application's menus live, not buried in the print step. */}
      <header className="app-header">
        <div className="menubar">
          <button type="button" className="menubar-item" onClick={() => setSettingsOpen(true)}>
            {t("settings.open")}
          </button>
          <button
            type="button"
            className="menubar-item"
            disabled={!image}
            onClick={startOver}
          >
            {t("wizard.start_over")}
          </button>
          <span className="menubar-gap" />
          <button
            type="button"
            className="menubar-item"
            disabled={step !== STEP_EDIT || !canUndo(history)}
            onClick={onUndo}
            title={t("edit.undo_hint")}
          >
            ↶ {t("edit.undo")}
          </button>
          <button
            type="button"
            className="menubar-item"
            disabled={step !== STEP_EDIT || !canRedo(history)}
            onClick={onRedo}
            title={t("edit.undo_hint")}
          >
            ↷ {t("edit.redo")}
          </button>
        </div>
        <h1>{t("app.title")}</h1>
      </header>

      {/* The editor needs every pixel of height it can get, and the step is
          already obvious from what is on screen. */}
      {step !== STEP_EDIT && <StepBar current={step} onGoTo={setStep} />}

      {step === STEP_PICK && (
        <div className="step-pane step-pane-narrow">
          <DropZone onPick={(f) => void onPickImage(f)} loadedName={imageName} />
          {detecting && <p className="status">{t("face.detecting")}</p>}
          {error && <div className="error">{error}</div>}
          {error && <Diagnostics />}
          {status && <div className="status">{status}</div>}
        </div>
      )}

      {step === STEP_FORMAT && (
        // A row, so the size panel sits in the space beside the list rather
        // than inside it: the list itself is left exactly as it was.
        <div className="format-row">
          <div className="step-pane format-list">
            <FormatPicker
              specs={specs}
              selectedId={specId}
              onSelect={onSelectSpec}
              // Shown on the custom card so it reflects what is typed in the
              // panel beside the list.
              customWidthMm={photoWidthMm}
              customHeightMm={photoHeightMm}
            />
            {error && <div className="error">{error}</div>}
          </div>

          {/* The custom size is the print size itself, which is the state
              every other format sets when it is selected. Typing here
              therefore feeds the layout, the preview and the print path
              without any of them needing to know about custom formats. */}
          {specId === CUSTOM_SPEC_ID && (
            <aside className="custom-size">
              <h3>{t("picker.custom_title")}</h3>
              <label>
                {t("picker.custom_width")}
                {/* Text rather than number: a number input cannot hold an
                    empty string, so clearing it would leave a 0 behind. */}
                <input
                  type="text"
                  inputMode="decimal"
                  aria-invalid={"errorKey" in customWidth}
                  value={customWidthText}
                  onChange={(e) => onCustomWidth(e.target.value)}
                />
              </label>
              {"errorKey" in customWidth && (
                <p className="custom-size-error">
                  {t(customWidth.errorKey, { min: CUSTOM_MIN_MM, max: CUSTOM_MAX_MM })}
                </p>
              )}

              <label>
                {t("picker.custom_height")}
                <input
                  type="text"
                  inputMode="decimal"
                  aria-invalid={"errorKey" in customHeight}
                  value={customHeightText}
                  onChange={(e) => onCustomHeight(e.target.value)}
                />
              </label>
              {"errorKey" in customHeight && (
                <p className="custom-size-error">
                  {t(customHeight.errorKey, { min: CUSTOM_MIN_MM, max: CUSTOM_MAX_MM })}
                </p>
              )}
            </aside>
          )}
        </div>
      )}

      {/*
        Steps 3 and 4 share one grid. The panels are hidden rather than
        unmounted so the photo canvas, its detection state and the loaded pixels
        survive moving between them — remounting would re-run detection and
        discard the user's anchor edits.
      */}
      <SettingsDialog
        open={settingsOpen}
        onClose={() => setSettingsOpen(false)}
        footer={
          <>
            {settingsStatus && <span className="status">{settingsStatus}</span>}
            <button type="button" className="primary" onClick={() => void onSaveSettings()}>
              {t("settings.save")}
            </button>
          </>
        }
        panes={[
          <div key="layout">
          <label>
            {t("paper.label")}
            <select value={paperId} onChange={(e) => setPaperId(e.target.value)}>
              {PAPERS.map((p) => (
                <option key={p.id} value={p.id}>
                  {t(p.labelKey)}
                </option>
              ))}
            </select>
          </label>

          <label>
            {t("layout.margin")}
            <input
              type="number"
              min={0}
              step={0.5}
              value={marginMm}
              onChange={(e) => setMarginMm(Number(e.target.value))}
            />
          </label>

          <label>
            {t("layout.gutter")}
            <input
              type="number"
              min={0}
              step={0.5}
              value={gutterMm}
              onChange={(e) => setGutterMm(Number(e.target.value))}
            />
          </label>

          <label className="checkbox">
            <input
              type="checkbox"
              checked={alignTopLeft}
              onChange={(e) => setAlignTopLeft(e.target.checked)}
            />
            {t("layout.align_top_left")}
          </label>

          <label className="checkbox">
            <input
              type="checkbox"
              checked={cutMarks}
              onChange={(e) => setCutMarks(e.target.checked)}
            />
            {t("layout.cut_marks")}
          </label>
          <p className="hint">{t("layout.cut_marks_hint")}</p>

          <label className="checkbox">
            <input
              type="checkbox"
              checked={quarterTurn}
              onChange={(e) => setQuarterTurn(e.target.checked)}
            />
            {t("settings.quarter_turn")}
          </label>
          <p className="hint">{t("settings.quarter_turn_hint")}</p>

          <label className="checkbox">
            <input
              type="checkbox"
              checked={turnPhoto}
              onChange={(e) => setTurnPhoto(e.target.checked)}
            />
            {t("settings.turn_photo")}
          </label>
          <p className="hint">{t("settings.turn_photo_hint")}</p>

          {/* Applied live so the effect is visible while choosing, and saved
              with the rest of the settings. */}
          <Stepper
            label={t("settings.font_scale")}
            value={fontScale}
            step={10}
            min={FONT_SCALE_MIN}
            max={FONT_SCALE_MAX}
            decimals={0}
            unit=" %"
            isAuto={fontScale === 100}
            onChange={setFontScale}
            onReset={() => setFontScale(100)}
          />
          <p className="hint">{t("settings.font_scale_hint")}</p>

          {/* Applied live, like the text scale: seeing it is the point. */}
          <label className="checkbox">
            <input
              type="checkbox"
              checked={theme === "dark"}
              onChange={(e) => setTheme(e.target.checked ? "dark" : "light")}
            />
            {t("settings.dark_mode")}
          </label>
          <p className="hint">{t("settings.dark_mode_hint")}</p>

          {/* Takes effect on the next start: resizing the window now would be
              a surprise while the user is in a dialog. */}
          <label className="checkbox">
            <input
              type="checkbox"
              checked={startMaximized}
              onChange={(e) => setStartMaximized(e.target.checked)}
            />
            {t("settings.start_maximized")}
          </label>
          <p className="hint">{t("settings.start_maximized_hint")}</p>

          <label>
            {t("format.count")}
            <input
              type="number"
              min={1}
              max={100}
              value={count}
              onChange={(e) => setCount(Number(e.target.value))}
            />
          </label>
          </div>,

          <div key="mixed">
          <p className="hint">{t("mixed.explain")}</p>

          <label className="checkbox">
            <input
              type="checkbox"
              checked={mixedMode}
              onChange={(e) => setMixedMode(e.target.checked)}
            />
            {t("mixed.enable")}
          </label>

          {mixedMode && (
            <>
              {groups.map((g, i) => (
                <div className="group-row" key={i}>
                  <span className="anchor-label">{t("mixed.group", { n: i + 1 })}</span>
                  <div className="row">
                    <input
                      type="number"
                      min={5}
                      step={0.5}
                      value={g.widthMm}
                      onChange={(e) => updateGroup(i, { widthMm: Number(e.target.value) })}
                      title={t("format.width")}
                    />
                    <span className="numeric">×</span>
                    <input
                      type="number"
                      min={5}
                      step={0.5}
                      value={g.heightMm}
                      onChange={(e) => updateGroup(i, { heightMm: Number(e.target.value) })}
                      title={t("format.height")}
                    />
                    <input
                      type="number"
                      min={1}
                      max={100}
                      value={g.count}
                      onChange={(e) => updateGroup(i, { count: Number(e.target.value) })}
                      title={t("format.count")}
                    />
                    <button
                      type="button"
                      className="small"
                      disabled={groups.length <= 1}
                      onClick={() => setGroups((gs) => gs.filter((_, j) => j !== i))}
                    >
                      {t("mixed.remove")}
                    </button>
                  </div>
                </div>
              ))}

              <button
                type="button"
                className="small"
                onClick={() =>
                  setGroups((gs) => [...gs, { widthMm: 35, heightMm: 45, count: 2 }])
                }
              >
                {t("mixed.add")}
              </button>

              {groups.length === 0 && <p className="hint">{t("mixed.empty")}</p>}
            </>
          )}
          </div>,

          <div key="presets">
          <p className="hint">{t("preset.explain")}</p>

          <label>
            {t("preset.name")}
            <input
              type="text"
              value={presetName}
              onChange={(e) => setPresetName(e.target.value)}
              placeholder={t("preset.name")}
            />
          </label>

          <button
            type="button"
            disabled={!presetName.trim()}
            onClick={() => void onSavePreset()}
          >
            {t("preset.save")}
          </button>
          <p className="hint">{t("preset.overwrite_hint")}</p>

          {presets.length === 0 ? (
            <p className="hint">{t("preset.none")}</p>
          ) : (
            presets.map((p) => (
              <div className="row" key={p.name}>
                <span className="anchor-label">{p.name}</span>
                <button
                  type="button"
                  className="small"
                  onClick={() => void onLoadPreset(p.name)}
                >
                  {t("preset.load")}
                </button>
                <button
                  type="button"
                  className="small"
                  onClick={() => void onDeletePreset(p.name)}
                >
                  {t("preset.delete")}
                </button>
              </div>
            ))
          )}
          </div>,

          <div key="calibration">
          <p className="hint">{t("calibration.explain")}</p>

          <div className="info">
            {calibration && calibration.calibratedAt ? (
              <>
                <div>
                  {t("calibration.done_at", {
                    date: new Date(calibration.calibratedAt).toLocaleDateString("hr-HR"),
                  })}
                </div>
                <div>
                  {t("calibration.correction", {
                    x: formatMm((calibration.scaleX - 1) * 100, 2),
                    y: formatMm((calibration.scaleY - 1) * 100, 2),
                  })}
                </div>
              </>
            ) : (
              t("calibration.never")
            )}
          </div>

          <button
            type="button"
            onClick={() => void onPrintSquare(false)}
            disabled={printing || !selectedPrinter}
          >
            {t("calibration.print_square")}
          </button>

          <label>
            {t("calibration.measured_x")}
            <input
              type="text"
              inputMode="decimal"
              value={measuredX}
              onChange={(e) => setMeasuredX(e.target.value)}
            />
          </label>

          <label>
            {t("calibration.measured_y")}
            <input
              type="text"
              inputMode="decimal"
              value={measuredY}
              onChange={(e) => setMeasuredY(e.target.value)}
            />
          </label>

          <button
            type="button"
            onClick={() => void onSaveCalibration()}
            disabled={!selectedPrinter}
          >
            {t("calibration.save")}
          </button>

          {calibration && calibration.calibratedAt && (
            <button
              type="button"
              onClick={() => void onPrintSquare(true)}
              disabled={printing || !selectedPrinter}
            >
              {t("calibration.print_verify")}
            </button>
          )}
          </div>,
        ]}
      />

      <div
        className={`columns ${step === STEP_EDIT ? "columns-edit" : "columns-print"}`}
        hidden={step !== STEP_EDIT && step !== STEP_PRINT}
      >
        <section className="panel editor-photo" hidden={step !== STEP_EDIT}>
          {image && (
            <>
              {/* Fixed 3:2 stage, so the panel does not resize with the photo
                  and shift every control below it. */}
              <div className="editor-stage">
                <PhotoCanvas
                  image={working?.canvas ?? null}
                  detection={detection}
                  anchors={anchors}
                  crop={crop}
                  rotationDeg={rotationDeg}
                  mask={mask}
                  showMask={showMask}
                  onAnchorMove={onAnchorMove}
                  onCropNudge={onCropNudge}
                />
              </div>
            </>
          )}
        </section>

        {/* The finished photo gets a column of its own, between the canvas it
            comes from and the controls that shape it. */}
        <section className="panel editor-preview" hidden={step !== STEP_EDIT}>
          {image && (
            <PhotoPreview
              image={composited ?? working?.canvas ?? null}
              crop={crop}
              rotationDeg={rotationDeg}
              widthMm={photoWidthMm}
              heightMm={photoHeightMm}
              maxEdgePx={340}
              onZoom={onCropZoom}
              canZoomIn={headHeightMm < HEAD_HEIGHT_MAX_MM}
              canZoomOut={headHeightMm > HEAD_HEIGHT_MIN_MM}
            />
          )}
        </section>

        <section className="panel editor-controls" hidden={step !== STEP_EDIT}>
          {image && (
            <>
              {detection && (
                <>
                  <Section
                    title={t("editor.group_crop")}
                    badge={hasAnyOverride ? t("face.reset") : null}
                  >
                  <Stepper
                    label={t("face.rotation")}
                    value={rotationDeg}
                    step={0.1}
                    min={-15}
                    max={15}
                    decimals={1}
                    unit="°"
                    isAuto={rotationOverride === null}
                    onChange={setRotationOverride}
                    onReset={() => setRotationOverride(null)}
                  />

                  {/* The drag instruction is a tooltip rather than a
                      paragraph: useful once, then permanent clutter. */}
                  {(["crown", "chin", "rightEye", "leftEye"] as const).map((name) => (
                    <div className="row" key={name} title={t("face.drag_hint")}>
                      <span className="anchor-label">{t(`face.${name}`)}</span>
                      <button
                        type="button"
                        className="small"
                        disabled={!anchorOverride[name]}
                        onClick={() => resetAnchor(name)}
                      >
                        {t("face.reset")}
                      </button>
                    </div>
                  ))}

                  <button
                    type="button"
                    disabled={!hasAnyOverride}
                    onClick={() => {
                      setAnchorOverride({});
                      setRotationOverride(null);
                    }}
                  >
                    {t("face.reset_all")}
                  </button>
                  </Section>

                  <Section
                    title={t("editor.group_tone")}
                    badge={ipc.isNeutral(adjustments) ? null : "●"}
                  >
                  {(
                    [
                      ["adjust.exposure", exposureEv, setExposureEv, -3, 3, 0.1],
                      ["adjust.contrast", contrast, setContrast, -1, 1, 0.05],
                      ["adjust.temperature", temperature, setTemperature, -1, 1, 0.05],
                      ["adjust.tint", tint, setTint, -1, 1, 0.05],
                    ] as const
                  ).map(([key, value, setter, min, max, step]) => (
                    <Stepper
                      key={key}
                      label={t(key)}
                      value={value}
                      step={step}
                      min={min}
                      max={max}
                      isAuto={value === 0}
                      onChange={setter}
                    />
                  ))}
                  <button
                    type="button"
                    className="small"
                    disabled={
                      exposureEv === 0 && contrast === 0 && temperature === 0 && tint === 0
                    }
                    onClick={() => {
                      setExposureEv(0);
                      setContrast(0);
                      setTemperature(0);
                      setTint(0);
                    }}
                  >
                    {t("adjust.reset")}
                  </button>
                  </Section>

                  <Section
                    title={t("editor.group_background")}
                    badge={mask && replaceBackground ? "●" : null}
                  >
                  <button
                    type="button"
                    onClick={() => void runSegmentation()}
                    disabled={segmenting}
                  >
                    {segmenting ? t("bg.working") : t("bg.remove")}
                  </button>

                  {mask && (
                    <>
                      <div className="info">
                        {t("bg.done", {
                          backend: mask.backend,
                          ratio: Math.round(mask.subjectRatio * 100),
                        })}
                      </div>

                      <label className="checkbox">
                        <input
                          type="checkbox"
                          checked={replaceBackground}
                          onChange={(e) => setReplaceBackground(e.target.checked)}
                        />
                        {t("bg.enabled")}
                      </label>

                      <label className="checkbox">
                        <input
                          type="checkbox"
                          checked={showMask}
                          onChange={(e) => setShowMask(e.target.checked)}
                        />
                        {t("bg.show_mask")}
                      </label>

                      {/* White is the offered default; the swatch stays so any
                          other colour can still be chosen. */}
                      <label>
                        {t("bg.colour")}
                        <div className="row">
                          <input
                            type="color"
                            value={`#${bgColour
                              .map((v) => v.toString(16).padStart(2, "0"))
                              .join("")}`}
                            onChange={(e) => {
                              const hex = e.target.value;
                              setBgColour([
                                parseInt(hex.slice(1, 3), 16),
                                parseInt(hex.slice(3, 5), 16),
                                parseInt(hex.slice(5, 7), 16),
                              ]);
                            }}
                          />
                          <button
                            type="button"
                            className="small"
                            onClick={() => setBgColour([255, 255, 255])}
                          >
                            {t("bg.colour_white")}
                          </button>
                        </div>
                      </label>

                      <label>
                        <span className="label-row">{t("bg.threshold")}</span>
                        <div className="row">
                          <input
                            type="range"
                            min={40}
                            max={220}
                            value={maskThreshold}
                            onChange={(e) =>
                              void onThresholdChange(Number(e.target.value))
                            }
                          />
                          <span className="numeric">{maskThreshold}</span>
                          <button
                            type="button"
                            className="small"
                            disabled={maskThreshold === 128}
                            onClick={() => void onThresholdChange(128)}
                          >
                            {t("face.reset")}
                          </button>
                        </div>
                      </label>
                    </>
                  )}
                  </Section>
                </>
              )}

              {cropError && (
                <div className="error">
                  {cropError}
                  {suggestedHeadMm !== null && (
                    <button
                      type="button"
                      className="small"
                      style={{ marginTop: 8 }}
                      onClick={() => setHeadHeightMm(Math.ceil(suggestedHeadMm * 10) / 10)}
                    >
                      {t("face.apply_suggested", {
                        mm: formatMm(suggestedHeadMm, 1),
                      })}
                    </button>
                  )}
                </div>
              )}
            </>
          )}

          {error && <div className="error">{error}</div>}
          {error && <Diagnostics />}
          {status && <div className="status">{status}</div>}
        </section>

        <section className="panel preview-panel" hidden={step !== STEP_PRINT}>
          <h2>{t("preview.sheet")}</h2>

          <SheetPreview
            layout={mixedMode ? mixedLayout : layout}
            paperWidthMm={paper.widthMm}
            paperHeightMm={paper.heightMm}
            hardwareMarginMm={
              caps
                ? {
                    left: caps.marginLeftMm,
                    top: caps.marginTopMm,
                    right: caps.marginRightMm,
                    bottom: caps.marginBottomMm,
                  }
                : undefined
            }
            showCutMarks={cutMarks}
            showBottomMark={bottomMark && !cutMarks}
            image={composited ?? working?.canvas ?? null}
            crop={crop}
            rotationDeg={rotationDeg}
            turnPhoto={turnPhoto}
          />

          {!mixedMode && layout && (
            <div className="info">
              <div>{t("layout.capacity", { capacity: layout.capacityPerSheet })}</div>
              <div>{t("layout.sheets_needed", { sheets: layout.sheetsNeeded })}</div>
              {rotated && <div className="hint">{t("layout.rotated")}</div>}
            </div>
          )}

          {mixedMode && mixedLayout && (
            <div className="info">
              <div>
                {t("mixed.all_placed", { count: mixedLayout.placements.length })}
              </div>
              {/* What does not fit is named explicitly, never silently dropped. */}
              {mixedLayout.unplaced.map((n, i) =>
                n > 0 ? (
                  <div className="hint" key={i}>
                    {t("mixed.unplaced", {
                      count: n,
                      width: groups[i]?.widthMm ?? 0,
                      height: groups[i]?.heightMm ?? 0,
                    })}
                  </div>
                ) : null,
              )}
            </div>
          )}

          {!crop && image && (
            <p className="hint">{t("preview.no_crop")}</p>
          )}
          {!image && <p className="hint">{t("preview.no_image")}</p>}
        </section>

        <section className="panel" hidden={step !== STEP_PRINT}>
          <h2>{t("step.print")}</h2>

          <label>
            {t("printer.label")}
            <select
              value={selectedPrinter}
              onChange={(e) => setSelectedPrinter(e.target.value)}
            >
              {printers.length === 0 && <option value="">{t("printer.none")}</option>}
              {printers.map((p) => (
                <option key={p.name} value={p.name}>
                  {p.name}
                </option>
              ))}
            </select>
          </label>

          <button type="button" onClick={() => void refreshPrinters()}>
            {t("printer.refresh")}
          </button>

          {caps && (
            <div className="info">
              <div>{t("printer.dpi", { dpiX: caps.dpiX, dpiY: caps.dpiY })}</div>
              <div>
                {t("printer.paper_from_driver", {
                  width: formatMm(paper.widthMm, 1),
                  height: formatMm(paper.heightMm, 1),
                })}
              </div>
              <div>
                {t("printer.margins", {
                  left: formatMm(caps.marginLeftMm, 2),
                  top: formatMm(caps.marginTopMm, 2),
                  right: formatMm(caps.marginRightMm, 2),
                  bottom: formatMm(caps.marginBottomMm, 2),
                })}
              </div>
            </div>
          )}

          {paperOverridden && (
            <p className="hint">
              {t("printer.paper_overridden", {
                width: formatMm(paper.widthMm, 1),
                height: formatMm(paper.heightMm, 1),
              })}
            </p>
          )}

          <button
            type="button"
            className="primary"
            onClick={() => void onPrintAndFinish()}
            disabled={
              printing ||
              !selectedPrinter ||
              (mixedMode
                ? !mixedLayout || mixedLayout.placements.length === 0
                : !layout || layout.placements.length === 0)
            }
          >
            {printing ? t("print.sending") : t("print.button")}
          </button>

          {/* Beside the print button as well as in the settings: whether to
              print the frame is decided per job, at the moment of printing,
              rather than being a setting to go and find. Both controls drive
              the same state, so they always agree. */}
          <label className="checkbox print-option">
            <input
              type="checkbox"
              checked={cutMarks}
              onChange={(e) => setCutMarks(e.target.checked)}
            />
            {t("layout.cut_marks")}
          </label>

          {/* Disabled while the full frame is on, because that frame already
              draws the bottom edge: leaving it clickable would offer a choice
              that changes nothing. */}
          <label className="checkbox print-option">
            <input
              type="checkbox"
              checked={bottomMark && !cutMarks}
              disabled={cutMarks}
              onChange={(e) => setBottomMark(e.target.checked)}
            />
            {t("layout.bottom_mark")}
          </label>
        </section>
      </div>

      {/* Printing is its own action on the last step, so no Next there. */}
      {step !== STEP_PRINT && (
        <nav className="wizard-nav">
          <button
            type="button"
            disabled={step === STEP_PICK}
            onClick={() => setStep((s) => Math.max(STEP_PICK, s - 1))}
          >
            {t("wizard.back")}
          </button>
          <button
            type="button"
            className="primary"
            disabled={!canAdvance}
            onClick={() => setStep((s) => Math.min(STEP_PRINT, s + 1))}
          >
            {t("wizard.next")} →
          </button>
        </nav>
      )}

      {step === STEP_PRINT && (
        <nav className="wizard-nav">
          <button type="button" onClick={() => setStep(STEP_EDIT)}>
            {t("wizard.back")}
          </button>
        </nav>
      )}
    </main>
  );
}
