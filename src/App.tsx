import { useCallback, useEffect, useMemo, useState } from "react";
import { SheetPreview } from "./components/SheetPreview";
import { formatError, t } from "./lib/i18n";
import * as ipc from "./lib/ipc";
import type { Calibration, Layout, Printer, PrinterCapabilities } from "./lib/ipc";
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
          />

          {layout && (
            <div className="info">
              <div>{t("layout.capacity", { capacity: layout.capacityPerSheet })}</div>
              <div>{t("layout.sheets_needed", { sheets: layout.sheetsNeeded })}</div>
              {rotated && <div className="hint">{t("layout.rotated")}</div>}
            </div>
          )}

          {error && <div className="error">{error}</div>}
          {status && <div className="status">{status}</div>}
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
