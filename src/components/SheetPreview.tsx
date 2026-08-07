import { useEffect, useRef, useState } from "react";
import type { CropResult, Layout } from "../lib/ipc";
import { fitScale, mm } from "../lib/units";

interface Props {
  layout: Layout | null;
  paperWidthMm: number;
  paperHeightMm: number;
  /** Hardware margin drawn as a dashed guide, so the user sees the dead zone. */
  hardwareMarginMm?: { left: number; top: number; right: number; bottom: number };
  showCutMarks: boolean;
  /** The loaded photo. Without one the sheet shows empty frames. */
  image?: HTMLImageElement | null;
  /** Region of the photo each copy shows. */
  crop?: CropResult | null;
  /** Straightening angle applied to the crop, in degrees. */
  rotationDeg?: number;
  /** Cap on the drawn height, so tall paper does not dominate the panel. */
  maxHeightPx?: number;
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
  image,
  crop,
  rotationDeg = 0,
  maxHeightPx = 520,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  // Measured rather than assumed: a fixed width overflows whenever the column
  // is narrower than the guess, which is what pushed the sheet off screen.
  const [availableWidth, setAvailableWidth] = useState(320);

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width;
      if (w && w > 0) setAvailableWidth(w);
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Fit within both the measured width and a height cap, so a tall sheet
    // shrinks instead of running off the panel.
    const scaleByWidth = fitScale(mm(paperWidthMm), availableWidth);
    const scaleByHeight = fitScale(mm(paperHeightMm), maxHeightPx);
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

      if (image && crop) {
        drawPhoto(ctx, image, crop, rotationDeg, p);
      } else {
        ctx.fillStyle = "#dfe4ea";
        ctx.fillRect(p.xMm, p.yMm, p.widthMm, p.heightMm);
      }

      ctx.strokeStyle = "#8e959e";
      ctx.lineWidth = 0.25;
      ctx.strokeRect(p.xMm, p.yMm, p.widthMm, p.heightMm);
    }
  }, [
    layout,
    paperWidthMm,
    paperHeightMm,
    hardwareMarginMm,
    showCutMarks,
    image,
    crop,
    rotationDeg,
    availableWidth,
    maxHeightPx,
  ]);

  return (
    <div ref={wrapRef} className="sheet-preview-wrap">
      <canvas ref={canvasRef} className="sheet-preview" />
    </div>
  );
}

/**
 * Draw the cropped, straightened photo into one placement.
 *
 * Mirrors what the Rust renderer does for the printer: the same crop region,
 * the same straightening angle, and the same 90 degree turn when the layout
 * chose landscape. Any divergence here is the classic "right on screen, wrong
 * on paper" bug, so the steps are kept deliberately literal.
 *
 * The canvas is already scaled to millimetres, so the destination rectangle is
 * in millimetres while the source is in image pixels.
 */
function drawPhoto(
  ctx: CanvasRenderingContext2D,
  image: HTMLImageElement,
  crop: CropResult,
  rotationDeg: number,
  p: { xMm: number; yMm: number; widthMm: number; heightMm: number; rotated: boolean },
) {
  const c = crop.rect;

  ctx.save();
  // Clip so nothing spills into the gutter between copies.
  ctx.beginPath();
  ctx.rect(p.xMm, p.yMm, p.widthMm, p.heightMm);
  ctx.clip();

  // Work from the centre of the placement outwards.
  ctx.translate(p.xMm + p.widthMm / 2, p.yMm + p.heightMm / 2);

  // A rotated layout turns the photo rather than stretching it, matching the
  // renderer; without this, faces would come out squashed on rotated sheets.
  if (p.rotated) {
    ctx.rotate(Math.PI / 2);
  }
  // After the quarter turn the photo's own width and height swap.
  const drawW = p.rotated ? p.heightMm : p.widthMm;
  const drawH = p.rotated ? p.widthMm : p.heightMm;

  // Straightening: rotate the source about the crop centre.
  const theta = (-rotationDeg * Math.PI) / 180;
  const scaleX = drawW / c.width;
  const scaleY = drawH / c.height;

  ctx.scale(scaleX, scaleY);
  ctx.rotate(theta);
  ctx.translate(-(c.x + c.width / 2), -(c.y + c.height / 2));

  ctx.drawImage(image, 0, 0, image.naturalWidth, image.naturalHeight);
  ctx.restore();
}
