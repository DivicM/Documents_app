import { useEffect, useRef } from "react";
import type { Layout } from "../lib/ipc";
import { fitScale, mm } from "../lib/units";

interface Props {
  layout: Layout | null;
  paperWidthMm: number;
  paperHeightMm: number;
  /** Hardware margin drawn as a dashed guide, so the user sees the dead zone. */
  hardwareMarginMm?: { left: number; top: number; right: number; bottom: number };
  showCutMarks: boolean;
  maxWidthPx?: number;
}

/**
 * Draws the sheet exactly as the layout describes it, in millimetres scaled to
 * fit the canvas. The printer renders the same millimetres at device DPI, so
 * anything visible here is what lands on paper.
 */
export function SheetPreview({
  layout,
  paperWidthMm,
  paperHeightMm,
  hardwareMarginMm,
  showCutMarks,
  maxWidthPx = 420,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Scale so the paper fits the available width, capped so tall paper stays
    // on screen too.
    const scaleByWidth = fitScale(mm(paperWidthMm), maxWidthPx);
    const scaleByHeight = fitScale(mm(paperHeightMm), maxWidthPx * 1.5);
    const scale = Math.min(scaleByWidth, scaleByHeight);

    const dpr = window.devicePixelRatio || 1;
    const cssWidth = paperWidthMm * scale;
    const cssHeight = paperHeightMm * scale;

    canvas.width = Math.round(cssWidth * dpr);
    canvas.height = Math.round(cssHeight * dpr);
    canvas.style.width = `${cssWidth}px`;
    canvas.style.height = `${cssHeight}px`;
    ctx.setTransform(dpr * scale, 0, 0, dpr * scale, 0, 0);

    // Everything below is in millimetres.
    ctx.clearRect(0, 0, paperWidthMm, paperHeightMm);
    ctx.fillStyle = "#ffffff";
    ctx.fillRect(0, 0, paperWidthMm, paperHeightMm);

    ctx.lineWidth = 0.3;
    ctx.strokeStyle = "#c9ccd1";
    ctx.strokeRect(0, 0, paperWidthMm, paperHeightMm);

    if (hardwareMarginMm) {
      const { left, top, right, bottom } = hardwareMarginMm;
      ctx.save();
      ctx.setLineDash([1.5, 1.5]);
      ctx.strokeStyle = "#e08a8a";
      ctx.lineWidth = 0.25;
      ctx.strokeRect(
        left,
        top,
        Math.max(0, paperWidthMm - left - right),
        Math.max(0, paperHeightMm - top - bottom),
      );
      ctx.restore();
    }

    if (!layout) return;

    for (const p of layout.placements) {
      if (showCutMarks) {
        ctx.save();
        ctx.strokeStyle = "#b9bec5";
        ctx.lineWidth = 0.2;
        const len = 3;
        // Corner marks sit outside the photo so they can be cut away.
        const corners: Array<[number, number, number, number]> = [
          [p.xMm - len, p.yMm, p.xMm, p.yMm],
          [p.xMm, p.yMm - len, p.xMm, p.yMm],
          [p.xMm + p.widthMm, p.yMm, p.xMm + p.widthMm + len, p.yMm],
          [p.xMm + p.widthMm, p.yMm - len, p.xMm + p.widthMm, p.yMm],
          [p.xMm - len, p.yMm + p.heightMm, p.xMm, p.yMm + p.heightMm],
          [p.xMm, p.yMm + p.heightMm, p.xMm, p.yMm + p.heightMm + len],
          [
            p.xMm + p.widthMm,
            p.yMm + p.heightMm,
            p.xMm + p.widthMm + len,
            p.yMm + p.heightMm,
          ],
          [
            p.xMm + p.widthMm,
            p.yMm + p.heightMm,
            p.xMm + p.widthMm,
            p.yMm + p.heightMm + len,
          ],
        ];
        ctx.beginPath();
        for (const [x1, y1, x2, y2] of corners) {
          ctx.moveTo(x1, y1);
          ctx.lineTo(x2, y2);
        }
        ctx.stroke();
        ctx.restore();
      }

      ctx.fillStyle = "#dfe4ea";
      ctx.fillRect(p.xMm, p.yMm, p.widthMm, p.heightMm);
      ctx.strokeStyle = "#8e959e";
      ctx.lineWidth = 0.25;
      ctx.strokeRect(p.xMm, p.yMm, p.widthMm, p.heightMm);
    }
  }, [layout, paperWidthMm, paperHeightMm, hardwareMarginMm, showCutMarks, maxWidthPx]);

  return <canvas ref={canvasRef} className="sheet-preview" />;
}
