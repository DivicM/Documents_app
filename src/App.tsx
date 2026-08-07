import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { PhotoCanvas, type Anchors, type HandleName } from "./components/PhotoCanvas";
import { SheetPreview } from "./components/SheetPreview";
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
  const fileInputRef = useRef<HTMLInputElement>(null);
  /** Pixels of the loaded photo, kept so printing need not decode it again. */
  const imagePixels = useRef<{ data: Uint8ClampedArray; w: number; h: number } | null>(null);

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
      <h1>{t("app.title")}</h1>

      <div className="columns">
        <section className="panel">
          <h2>{t("step.format")}</h2>

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
            image={image}
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
