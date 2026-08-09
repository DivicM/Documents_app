import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { CompliancePanel } from "./components/CompliancePanel";
import { PhotoCanvas, type Anchors, type HandleName } from "./components/PhotoCanvas";
import { SheetPreview } from "./components/SheetPreview";
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

export default function App() {
  const [printers, setPrinters] = useState<Printer[]>([]);
  const [selectedPrinter, setSelectedPrinter] = useState<string>("");
  const [caps, setCaps] = useState<PrinterCapabilities | null>(null);
  const [calibration, setCalibration] = useState<Calibration | null>(null);

  const [paperId, setPaperId] = useState<string>("10x15");
  const [photoWidthMm, setPhotoWidthMm] = useState(35);
  const [photoHeightMm, setPhotoHeightMm] = useState(45);
  const [count, setCount] = useState(6);
  const [marginMm, setMarginMm] = useState(3);
  const [gutterMm, setGutterMm] = useState(2);
  const [alignTopLeft, setAlignTopLeft] = useState(false);
  const [cutMarks, setCutMarks] = useState(true);

  const [layout, setLayout] = useState<Layout | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);

  const [measuredX, setMeasuredX] = useState("50");
  const [measuredY, setMeasuredY] = useState("50");

  // Photo and face detection.
  const [image, setImage] = useState<HTMLImageElement | null>(null);
  const [detection, setDetection] = useState<FaceDetection | null>(null);
  const [detecting, setDetecting] = useState(false);
  const [crop, setCrop] = useState<CropResult | null>(null);
  const [cropError, setCropError] = useState<string | null>(null);
  /** Head height the solver says would fit, offered as a one-click fix. */
  const [suggestedHeadMm, setSuggestedHeadMm] = useState<number | null>(null);
  const [headHeightMm, setHeadHeightMm] = useState(33.75);
  /** User-dragged anchors. Missing entries fall back to the detected ones. */
  const [anchorOverride, setAnchorOverride] = useState<Partial<Anchors>>({});
  /** Explicit rotation, overriding the angle derived from the eye line. */
  const [rotationOverride, setRotationOverride] = useState<number | null>(null);

  // Background removal.
  const [mask, setMask] = useState<ipc.Mask | null>(null);
  const [segmenting, setSegmenting] = useState(false);
  const [replaceBackground, setReplaceBackground] = useState(true);
  const [bgColour, setBgColour] = useState<[number, number, number]>([235, 235, 235]);
  const [showMask, setShowMask] = useState(false);
  const [brushMode, setBrushMode] = useState<"keep" | "erase" | null>(null);
  const [brushRadius, setBrushRadius] = useState(24);
  const [strokeCount, setStrokeCount] = useState(0);
  const [maskThreshold, setMaskThreshold] = useState(128);
  /** When on, the next click on the photo picks the background colour. */
  const [pickingColour, setPickingColour] = useState(false);

  // Document specification and validation.
  const [specs, setSpecs] = useState<ipc.SpecSummary[]>([]);
  /** Empty string means the free "custom" mode. */
  const [specId, setSpecId] = useState("");
  const [validation, setValidation] = useState<ipc.Validation | null>(null);
  const [imageStats, setImageStats] = useState<ipc.ImageStats | null>(null);
  const [bgStddev, setBgStddev] = useState<number | null>(null);

  // Exposure and white balance.
  const [exposureEv, setExposureEv] = useState(0);
  const [contrast, setContrast] = useState(0);
  const [temperature, setTemperature] = useState(0);
  const [tint, setTint] = useState(0);

  // Undo/redo over edit parameters. Snapshots are cheap because they hold
  // numbers, not pixels.
  const [history, setHistory] = useState<History<EditState>>(() =>
    createHistory<EditState>({
      anchorOverride: {},
      rotationOverride: null,
      headHeightMm: 33.75,
      photoWidthMm: 35,
      photoHeightMm: 45,
      count: 6,
      marginMm: 3,
      gutterMm: 2,
      alignTopLeft: false,
      cutMarks: true,
      replaceBackground: true,
      bgColour: [235, 235, 235],
      exposureEv: 0,
      contrast: 0,
      temperature: 0,
      tint: 0,
    }),
  );
  /** Set while applying an undo, so the restore is not recorded as an edit. */
  const restoring = useRef(false);
  const fileInputRef = useRef<HTMLInputElement>(null);
  /** Pixels of the loaded photo, kept so printing need not decode it again. */
  const imagePixels = useRef<{ data: Uint8ClampedArray; w: number; h: number } | null>(null);
  /**
   * The photo with its background already replaced, used for previewing.
   *
   * Built here rather than in the preview component so both the sheet and the
   * printer work from the same composited pixels.
   */
  const [composited, setComposited] = useState<HTMLImageElement | null>(null);

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

  const paper = useMemo(
    () => PAPERS.find((p) => p.id === paperId) ?? PAPERS[0],
    [paperId],
  );

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
      if (s.headHeightMm !== null) setHeadHeightMm(s.headHeightMm);
      setCount(s.defaultCount);
      setCutMarks(s.cutMarks);
      if (s.backgroundRgb) setBgColour(s.backgroundRgb);
    },
    [specs],
  );

  // Printer capabilities and calibration both depend on printer + paper.
  useEffect(() => {
    if (!selectedPrinter) return;
    let cancelled = false;

    (async () => {
      try {
        const c = await ipc.printerCapabilities(
          selectedPrinter,
          paper.widthMm,
          paper.heightMm,
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
  }, [selectedPrinter, paper.widthMm, paper.heightMm]);

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
    setStrokeCount(0);
    setShowMask(false);
    setBrushMode(null);
    setMaskThreshold(128);
    setPickingColour(false);
    void ipc.clearMask().catch(() => {});

    try {
      const canvas = document.createElement("canvas");
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext("2d", { willReadFrequently: true });
      if (!ctx) throw new Error(t("error.image.decode_failed"));
      ctx.drawImage(img, 0, 0);
      const data = ctx.getImageData(0, 0, canvas.width, canvas.height);
      // Kept for printing, so the file is decoded exactly once.
      imagePixels.current = { data: data.data, w: canvas.width, h: canvas.height };

      const found = await ipc.detectFace(data.data, canvas.width, canvas.height);
      setDetection(found);
      if (!found) setError(t("face.none_found"));

      // Sharpness and exposure feed the validator; measured over the face so a
      // busy background cannot make a soft portrait look sharp.
      try {
        setImageStats(
          await ipc.analyseImage(data.data, canvas.width, canvas.height, found?.faceBox ?? null),
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

  // Recompute the crop whenever the anchors or the requested head size change.
  useEffect(() => {
    if (!image || !anchors) {
      setCrop(null);
      return;
    }
    let cancelled = false;

    (async () => {
      try {
        const result = await ipc.computeCrop({
          chinX: anchors.chin.x,
          chinY: anchors.chin.y,
          crownX: anchors.crown.x,
          crownY: anchors.crown.y,
          imageWidth: image.naturalWidth,
          imageHeight: image.naturalHeight,
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
      const toned = new Image();
      toned.onload = () => {
        if (!cancelled) setComposited(toned);
      };
      toned.src = canvas.toDataURL();
      return () => {
        cancelled = true;
      };
    }

    for (let y = 0; y < px.h; y++) {
      const my = Math.floor((y * mask.height) / px.h);
      for (let x = 0; x < px.w; x++) {
        const mx = Math.floor((x * mask.width) / px.w);
        const alpha = mask.data[my * mask.width + mx] / 255;
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

    const img = new Image();
    img.onload = () => {
      if (!cancelled) setComposited(img);
    };
    img.src = canvas.toDataURL();

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

  /** Apply a suggested correction from the compliance panel. */
  const onApplyFix = useCallback((fix: ipc.FixHint) => {
    switch (fix.kind) {
      case "set_head_height_mm":
        setHeadHeightMm(fix.value);
        break;
      case "set_rotation_deg":
        // The fix reports the measured tilt; straightening means applying it.
        setRotationOverride(fix.value);
        break;
      case "set_dpi":
        // Nothing to set directly: the resolution warning is informational,
        // so surface it rather than silently changing the output size.
        break;
    }
  }, []);

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
      if (!anchors || !image) return;
      const clampX = (v: number) => Math.max(0, Math.min(image.naturalWidth, v));
      const clampY = (v: number) => Math.max(0, Math.min(image.naturalHeight, v));
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
    [anchors, image],
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
      setStrokeCount(0);
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

  const onStroke = useCallback(
    async (points: Array<[number, number]>) => {
      if (!brushMode || !mask) return;
      try {
        // The brush radius is in screen terms; convert to mask pixels so a
        // stroke covers what the user saw under the cursor.
        const px = imagePixels.current;
        const scale = px ? mask.width / px.w : 1;
        const m = await ipc.addMaskStroke({
          mode: brushMode,
          radius: brushRadius * scale,
          feather: brushRadius * scale * 0.35,
          points,
        });
        setMask(m);
        setStrokeCount((n) => n + 1);
      } catch (e) {
        setError(formatError(e));
      }
    },
    [brushMode, brushRadius, mask],
  );

  const onThresholdChange = async (value: number) => {
    setMaskThreshold(value);
    try {
      setMask(await ipc.setMaskThreshold(value));
    } catch (e) {
      setError(formatError(e));
    }
  };

  /** Sample a colour from the photo, for matching an existing background. */
  const onPickColour = useCallback((x: number, y: number) => {
    const px = imagePixels.current;
    if (!px) return;
    const ix = Math.round(Math.max(0, Math.min(px.w - 1, x)));
    const iy = Math.round(Math.max(0, Math.min(px.h - 1, y)));
    const i = (iy * px.w + ix) * 4;
    setBgColour([px.data[i], px.data[i + 1], px.data[i + 2]]);
    setPickingColour(false);
  }, []);

  const onUndoStroke = async () => {
    try {
      setMask(await ipc.undoMaskStroke());
      setStrokeCount((n) => Math.max(0, n - 1));
    } catch (e) {
      setError(formatError(e));
    }
  };

  const onResetMask = async () => {
    try {
      setMask(await ipc.resetMaskEdits());
      setStrokeCount(0);
    } catch (e) {
      setError(formatError(e));
    }
  };

  const onPrintSquare = async (applyCalibration: boolean) => {
    if (!selectedPrinter) return;
    setStatus(null);
    setError(null);
    try {
      const jobId = await ipc.printCalibrationSquare(selectedPrinter, applyCalibration);
      setStatus(t("print.sent", { jobId }));
    } catch (e) {
      setError(formatError(e));
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

  const onPrintSheet = async () => {
    if (!selectedPrinter || !layout) return;
    setStatus(null);
    setError(null);
    try {
      // Print the photo when one is loaded and its crop is valid; otherwise
      // fall back to plain rectangles so the layout can still be checked.
      const px = imagePixels.current;
      const photo =
        px && crop
          ? {
              rgba: px.data,
              width: px.w,
              height: px.h,
              cropX: crop.rect.x,
              cropY: crop.rect.y,
              cropWidth: crop.rect.width,
              cropHeight: crop.rect.height,
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
            }
          : null;

      const jobId = await ipc.printSheet({
        printer: selectedPrinter,
        paperWidthMm: paper.widthMm,
        paperHeightMm: paper.heightMm,
        photoWidthMm,
        photoHeightMm,
        count,
        marginMm,
        gutterMm,
        alignTopLeft,
        photo,
      });
      setStatus(t("print.sent", { jobId }));
    } catch (e) {
      setError(formatError(e));
    }
  };

  const rotated = layout?.placements[0]?.rotated ?? false;

  return (
    <main className="app">
      <header className="app-header">
        <h1>{t("app.title")}</h1>
        <div className="header-actions">
          <button
            type="button"
            className="small"
            disabled={!canUndo(history)}
            onClick={onUndo}
            title={t("edit.undo_hint")}
          >
            ↶ {t("edit.undo")}
          </button>
          <button
            type="button"
            className="small"
            disabled={!canRedo(history)}
            onClick={onRedo}
            title={t("edit.undo_hint")}
          >
            ↷ {t("edit.redo")}
          </button>
        </div>
      </header>

      <div className="columns">
        <section className="panel">
          <h2>{t("step.format")}</h2>

          <label>
            {t("spec.label")}
            <select value={specId} onChange={(e) => onSelectSpec(e.target.value)}>
              <option value="">{t("spec.custom")}</option>
              {specs.map((s) => (
                <option key={s.id} value={s.id}>
                  {s.name}
                </option>
              ))}
            </select>
          </label>

          {specId === "" && <p className="hint">{t("spec.free_mode")}</p>}
          {specId !== "" && <p className="hint">{t("spec.chin_line_note")}</p>}

          <label>
            {t("format.width")}
            <input
              type="number"
              min={5}
              step={0.5}
              value={photoWidthMm}
              onChange={(e) => setPhotoWidthMm(Number(e.target.value))}
            />
          </label>

          <label>
            {t("format.height")}
            <input
              type="number"
              min={5}
              step={0.5}
              value={photoHeightMm}
              onChange={(e) => setPhotoHeightMm(Number(e.target.value))}
            />
          </label>

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

          <h2>{t("step.layout")}</h2>

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
        </section>

        <section className="panel preview-panel">
          <input
            ref={fileInputRef}
            type="file"
            accept="image/*"
            style={{ display: "none" }}
            onChange={(e) => {
              const f = e.target.files?.[0];
              if (f) void onPickImage(f);
              e.target.value = "";
            }}
          />

          {!image && (
            <button type="button" onClick={() => fileInputRef.current?.click()}>
              {t("face.load_image")}
            </button>
          )}

          {image && (
            <>
              <PhotoCanvas
                image={image}
                detection={detection}
                anchors={anchors}
                crop={crop}
                rotationDeg={rotationDeg}
                mask={mask}
                showMask={showMask}
                brush={brushMode ? { mode: brushMode, radius: brushRadius } : null}
                onStroke={onStroke}
                onPickColour={pickingColour ? onPickColour : null}
                onAnchorMove={onAnchorMove}
                onCropNudge={onCropNudge}
              />

              <div className="row">
                <button type="button" onClick={() => fileInputRef.current?.click()}>
                  {t("face.load_image")}
                </button>
                <button
                  type="button"
                  onClick={() => image && void runDetection(image)}
                  disabled={detecting}
                >
                  {detecting ? t("face.detecting") : t("face.detect")}
                </button>
              </div>

              {detection && (
                <div className="info">
                  <div>
                    {t("face.found", {
                      confidence: Math.round(detection.confidence * 100),
                    })}
                  </div>
                  <div>{t("face.roll", { deg: formatMm(detection.rollDeg, 1) })}</div>
                  <div>
                    {t("face.eye_distance", { px: Math.round(detection.eyeDistancePx) })}
                  </div>
                  {crop && (
                    <div>
                      {t("face.max_dpi", { dpi: Math.round(crop.maxLosslessDpi) })}
                    </div>
                  )}
                </div>
              )}

              {detection && (
                <>
                  <label>
                    <span className="label-row">
                      {t("face.head_height")}
                      {anchorOverride.chin || anchorOverride.crown ? null : (
                        <span className="badge">{t("face.auto")}</span>
                      )}
                    </span>
                    <input
                      type="number"
                      min={10}
                      max={60}
                      step={0.25}
                      value={headHeightMm}
                      onChange={(e) => setHeadHeightMm(Number(e.target.value))}
                    />
                  </label>

                  <label>
                    <span className="label-row">
                      {t("face.rotation")}
                      {rotationOverride === null && (
                        <span className="badge">{t("face.auto")}</span>
                      )}
                    </span>
                    <div className="row">
                      <input
                        type="range"
                        min={-15}
                        max={15}
                        step={0.1}
                        value={rotationDeg}
                        onChange={(e) => setRotationOverride(Number(e.target.value))}
                      />
                      <span className="numeric">{formatMm(rotationDeg, 1)}°</span>
                      <button
                        type="button"
                        className="small"
                        disabled={rotationOverride === null}
                        onClick={() => setRotationOverride(null)}
                      >
                        {t("face.reset")}
                      </button>
                    </div>
                  </label>

                  <p className="hint">{t("face.drag_hint")}</p>

                  {(["crown", "chin", "rightEye", "leftEye"] as const).map((name) => (
                    <div className="row" key={name}>
                      <span className="anchor-label">
                        {t(`face.${name}`)}
                        {!anchorOverride[name] && (
                          <span className="badge">{t("face.auto")}</span>
                        )}
                      </span>
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

                  <h2>{t("adjust.title")}</h2>
                  {(
                    [
                      ["adjust.exposure", exposureEv, setExposureEv, -3, 3, 0.1],
                      ["adjust.contrast", contrast, setContrast, -1, 1, 0.05],
                      ["adjust.temperature", temperature, setTemperature, -1, 1, 0.05],
                      ["adjust.tint", tint, setTint, -1, 1, 0.05],
                    ] as const
                  ).map(([key, value, setter, min, max, step]) => (
                    <label key={key}>
                      <span className="label-row">
                        {t(key)}
                        {value === 0 && <span className="badge">{t("face.auto")}</span>}
                      </span>
                      <div className="row">
                        <input
                          type="range"
                          min={min}
                          max={max}
                          step={step}
                          value={value}
                          onChange={(e) => setter(Number(e.target.value))}
                        />
                        <span className="numeric">{formatMm(value, 2)}</span>
                      </div>
                    </label>
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

                  <h2>{t("bg.title")}</h2>

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
                            className={`small ${pickingColour ? "active" : ""}`}
                            onClick={() => setPickingColour(!pickingColour)}
                            title={t("bg.eyedropper_hint")}
                          >
                            {t("bg.eyedropper")}
                          </button>
                          <button
                            type="button"
                            className="small"
                            onClick={() => setBgColour([235, 235, 235])}
                          >
                            {t("bg.colour_grey")}
                          </button>
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
                        <span className="label-row">
                          {t("bg.threshold")}
                          {maskThreshold === 128 && (
                            <span className="badge">{t("face.auto")}</span>
                          )}
                        </span>
                        <div className="row">
                          <input
                            type="range"
                            min={40}
                            max={220}
                            value={maskThreshold}
                            onChange={(e) => void onThresholdChange(Number(e.target.value))}
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
                        <p className="hint">{t("bg.threshold_hint")}</p>
                      </label>

                      <label>
                        {t("bg.brush")}
                        <div className="row">
                          <button
                            type="button"
                            className={`small ${brushMode === "keep" ? "active" : ""}`}
                            onClick={() =>
                              setBrushMode(brushMode === "keep" ? null : "keep")
                            }
                          >
                            {t("bg.brush_keep")}
                          </button>
                          <button
                            type="button"
                            className={`small ${brushMode === "erase" ? "active" : ""}`}
                            onClick={() =>
                              setBrushMode(brushMode === "erase" ? null : "erase")
                            }
                          >
                            {t("bg.brush_erase")}
                          </button>
                        </div>
                      </label>

                      {brushMode && (
                        <>
                          <label>
                            {t("bg.brush_size")}
                            <div className="row">
                              <input
                                type="range"
                                min={4}
                                max={80}
                                value={brushRadius}
                                onChange={(e) => setBrushRadius(Number(e.target.value))}
                              />
                              <span className="numeric">{brushRadius}</span>
                            </div>
                          </label>
                          <p className="hint">{t("bg.brush_hint")}</p>
                        </>
                      )}

                      <div className="row">
                        <button
                          type="button"
                          className="small"
                          disabled={strokeCount === 0}
                          onClick={() => void onUndoStroke()}
                        >
                          {t("bg.undo")}
                        </button>
                        <button
                          type="button"
                          className="small"
                          disabled={strokeCount === 0}
                          onClick={() => void onResetMask()}
                        >
                          {t("bg.reset")}
                        </button>
                      </div>
                    </>
                  )}
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
          {status && <div className="status">{status}</div>}
        </section>

        <section className="panel preview-panel">
          <h2>{t("preview.sheet")}</h2>

          <SheetPreview
            layout={layout}
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
            image={composited ?? image}
            crop={crop}
            rotationDeg={rotationDeg}
          />

          {layout && (
            <div className="info">
              <div>{t("layout.capacity", { capacity: layout.capacityPerSheet })}</div>
              <div>{t("layout.sheets_needed", { sheets: layout.sheetsNeeded })}</div>
              {rotated && <div className="hint">{t("layout.rotated")}</div>}
            </div>
          )}

          {!crop && image && (
            <p className="hint">{t("preview.no_crop")}</p>
          )}
          {!image && <p className="hint">{t("preview.no_image")}</p>}

          <CompliancePanel validation={validation} onApplyFix={onApplyFix} />
        </section>

        <section className="panel">
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
                {t("printer.margins", {
                  left: formatMm(caps.marginLeftMm, 2),
                  top: formatMm(caps.marginTopMm, 2),
                  right: formatMm(caps.marginRightMm, 2),
                  bottom: formatMm(caps.marginBottomMm, 2),
                })}
              </div>
            </div>
          )}

          <button
            type="button"
            className="primary"
            onClick={() => void onPrintSheet()}
            disabled={!selectedPrinter || !layout || layout.placements.length === 0}
          >
            {t("print.button")}
          </button>

          <h2>{t("calibration.title")}</h2>
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
            disabled={!selectedPrinter}
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
              disabled={!selectedPrinter}
            >
              {t("calibration.print_verify")}
            </button>
          )}
        </section>
      </div>

      <footer className="disclaimer">{t("disclaimer")}</footer>
    </main>
  );
}
