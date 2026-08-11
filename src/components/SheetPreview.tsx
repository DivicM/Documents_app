import { useEffect, useRef, useState } from "react";
import type { CropResult, Layout, Placement } from "../lib/ipc";
import { fitScale, mm } from "../lib/units";

interface Props {
  /**
   * Placements to draw. A mixed sheet supplies them directly, since its copies
   * differ in size and so do not form a `Layout`.
   */
  layout: Layout | { placements: Placement[] } | null;
  paperWidthMm: number;
  paperHeightMm: number;
  /** Hardware margin drawn as a dashed guide, so the user sees the dead zone. */
  hardwareMarginMm?: { left: number; top: number; right: number; bottom: number };
  showCutMarks: boolean;
  /** The loaded photo. Without one the sheet shows empty frames. */
  /** A canvas as well as an image; the composited copy is produced on one. */
  image?: HTMLImageElement | HTMLCanvasElement | null;
  /** Region of the photo each copy shows. */
  crop?: CropResult | null;
  /** Straightening angle applied to the crop, in degrees. */
  rotationDeg?: number;
  /** Turn the picture inside its frame, matching RenderParams::turn_photo. */
  turnPhoto?: boolean;
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
  turnPhoto = false,
  maxHeightPx = 520,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const wrapRef = useRef<HTMLDivElement>(null);
  /**
   * Both axes are measured rather than assumed. A guessed width overflowed
   * whenever the column was narrower; a guessed height clipped the sheet from
   * the bottom, hiding whole rows of photographs.
   */
  const [avail, setAvail] = useState({ w: 320, h: maxHeightPx });

  useEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const observer = new ResizeObserver((entries) => {
      const r = entries[0]?.contentRect;
      if (r && r.width > 0 && r.height > 0) setAvail({ w: r.width, h: r.height });
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Fit within the space actually available on both axes, so the whole sheet
    // is visible however the panel is shaped.
    const scaleByWidth = fitScale(mm(paperWidthMm), avail.w);
    const scaleByHeight = fitScale(mm(paperHeightMm), avail.h);
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
        // Matches CUT_MARK_GREY and CUT_MARK_LENGTH_MM in render.rs, so the
        // preview shows the same faint guides that will be printed.
        ctx.strokeStyle = "#d2d2d2";
        ctx.lineWidth = 0.2;
        const len = 2.5;
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
        drawPhoto(ctx, image, crop, rotationDeg, p, turnPhoto);
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
    turnPhoto,
    avail,
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
  image: HTMLImageElement | HTMLCanvasElement,
  crop: CropResult,
  rotationDeg: number,
  p: { xMm: number; yMm: number; widthMm: number; heightMm: number; rotated: boolean },
  turnPhoto: boolean,
) {
  const c = crop.rect;

  ctx.save();
  // Clip so nothing spills into the gutter between copies.
  ctx.beginPath();
  ctx.rect(p.xMm, p.yMm, p.widthMm, p.heightMm);
  ctx.clip();

  // Work from the centre of the placement outwards.
  ctx.translate(p.xMm + p.widthMm / 2, p.yMm + p.heightMm / 2);

  // XOR, matching render.rs: the frame's own rotation and a requested turn of
  // the picture compose. Diverging here is the classic "right on screen, wrong
  // on paper" bug.
  const turned = p.rotated !== turnPhoto;
  if (turned) {
    ctx.rotate(Math.PI / 2);
  }
  // After the quarter turn the photo's own width and height swap.
  const drawW = turned ? p.heightMm : p.widthMm;
  const drawH = turned ? p.widthMm : p.heightMm;

  // Straightening: rotate the source about the crop centre.
  const theta = (-rotationDeg * Math.PI) / 180;
  const scaleX = drawW / c.width;
  const scaleY = drawH / c.height;

  ctx.scale(scaleX, scaleY);
  ctx.rotate(theta);
  ctx.translate(-(c.x + c.width / 2), -(c.y + c.height / 2));

  const srcW = image instanceof HTMLCanvasElement ? image.width : image.naturalWidth;
  const srcH = image instanceof HTMLCanvasElement ? image.height : image.naturalHeight;
  ctx.drawImage(image, 0, 0, srcW, srcH);
  ctx.restore();
}
