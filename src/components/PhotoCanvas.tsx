import { useCallback, useEffect, useRef, useState } from "react";
import type { CropResult, FaceDetection } from "../lib/ipc";

export interface Anchors {
  chin: { x: number; y: number };
  crown: { x: number; y: number };
  rightEye: { x: number; y: number };
  leftEye: { x: number; y: number };
}

/** Points the user can drag on the canvas. */
export type HandleName = keyof Anchors;

interface Props {
  image: HTMLImageElement | null;
  detection: FaceDetection | null;
  anchors: Anchors | null;
  crop: CropResult | null;
  /** Straightening angle in degrees, drawn so the effect is visible. */
  rotationDeg?: number;
  /** Background mask, drawn as a tint so its edges can be judged. */
  mask?: { width: number; height: number; data: Uint8Array } | null;
  /** Whether the mask tint is shown. */
  showMask?: boolean;
  /** Active brush; when set, dragging paints instead of moving the crop. */
  brush?: { mode: "keep" | "erase"; radius: number } | null;
  /** Called with a completed stroke, in mask coordinates. */
  onStroke?: (points: Array<[number, number]>) => void;
  /** When set, a click samples a colour instead of editing. */
  onPickColour?: ((x: number, y: number) => void) | null;
  /** Called while dragging, in source-image pixels. */
  onAnchorMove: (which: HandleName, x: number, y: number) => void;
  /** Called while dragging the crop itself, with the offset in source pixels. */
  onCropNudge?: (dx: number, dy: number) => void;
  maxWidthPx?: number;
}

type Handle = HandleName | null;

const HANDLE_RADIUS_PX = 7;

/**
 * The photo with the crop rectangle and draggable head anchors.
 *
 * Anchors are the two points a five-point detector cannot see directly, so
 * they are the ones most likely to need correcting by hand.
 */
export function PhotoCanvas({
  image,
  detection,
  anchors,
  crop,
  rotationDeg = 0,
  mask,
  showMask = false,
  brush = null,
  onStroke,
  onPickColour = null,
  onAnchorMove,
  onCropNudge,
  maxWidthPx = 520,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const [dragging, setDragging] = useState<Handle>(null);
  /** Set while dragging the crop body; holds the last pointer position. */
  const cropDrag = useRef<{ x: number; y: number } | null>(null);
  const [cursor, setCursor] = useState("crosshair");
  /** Points of the stroke currently being drawn, in mask coordinates. */
  const strokePoints = useRef<Array<[number, number]> | null>(null);
  /** Mask rendered once into an offscreen canvas, rather than per frame. */
  const maskCanvas = useRef<HTMLCanvasElement | null>(null);

  // Rebuild the tint only when the mask itself changes.
  useEffect(() => {
    if (!mask) {
      maskCanvas.current = null;
      return;
    }
    const off = document.createElement("canvas");
    off.width = mask.width;
    off.height = mask.height;
    const octx = off.getContext("2d");
    if (!octx) return;

    const img = octx.createImageData(mask.width, mask.height);
    for (let i = 0; i < mask.data.length; i++) {
      // Background is tinted red; the subject stays clear so the face is
      // visible while brushing.
      const background = 255 - mask.data[i];
      img.data[i * 4] = 220;
      img.data[i * 4 + 1] = 60;
      img.data[i * 4 + 2] = 60;
      img.data[i * 4 + 3] = Math.round(background * 0.45);
    }
    octx.putImageData(img, 0, 0);
    maskCanvas.current = off;
  }, [mask]);

  // Scale from source pixels to canvas pixels.
  const scale = image ? Math.min(maxWidthPx / image.width, 1) : 1;

  const toCanvas = useCallback((v: number) => v * scale, [scale]);
  const toSource = useCallback((v: number) => v / scale, [scale]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas || !image) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    const w = image.width * scale;
    const h = image.height * scale;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(h * dpr);
    canvas.style.width = `${w}px`;
    canvas.style.height = `${h}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    ctx.clearRect(0, 0, w, h);
    ctx.drawImage(image, 0, 0, w, h);

    // The mask is stretched over the whole photo, matching how it is applied
    // when the background is replaced.
    if (showMask && maskCanvas.current) {
      ctx.drawImage(maskCanvas.current, 0, 0, w, h);
    }

    // Everything outside the crop is dimmed, so the result is obvious. The
    // rectangle is drawn rotated because that is the region the printer will
    // actually sample once straightening is applied.
    if (crop) {
      const c = crop.rect;
      const cx = toCanvas(c.x + c.width / 2);
      const cy = toCanvas(c.y + c.height / 2);
      const cw = toCanvas(c.width);
      const ch = toCanvas(c.height);
      const theta = (rotationDeg * Math.PI) / 180;

      // Dim everything outside the rotated crop, using an even-odd fill of the
      // whole canvas minus the crop rectangle.
      ctx.save();
      ctx.fillStyle = "rgba(0, 0, 0, 0.45)";
      ctx.beginPath();
      ctx.rect(0, 0, w, h);
      ctx.translate(cx, cy);
      ctx.rotate(theta);
      ctx.rect(-cw / 2, -ch / 2, cw, ch);
      ctx.fill("evenodd");
      ctx.restore();

      ctx.save();
      ctx.translate(cx, cy);
      ctx.rotate(theta);
      ctx.strokeStyle = "#2f6f4f";
      ctx.lineWidth = 2;
      ctx.strokeRect(-cw / 2, -ch / 2, cw, ch);
      ctx.restore();
    }

    if (detection) {
      ctx.strokeStyle = "rgba(70, 130, 200, 0.9)";
      ctx.lineWidth = 1.5;
      const b = detection.faceBox;
      ctx.strokeRect(toCanvas(b.x), toCanvas(b.y), toCanvas(b.width), toCanvas(b.height));
    }

    if (anchors) {
      // Eye line: the measurement the crown estimate and the tilt come from.
      ctx.strokeStyle = "rgba(70, 130, 200, 0.8)";
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.moveTo(toCanvas(anchors.rightEye.x), toCanvas(anchors.rightEye.y));
      ctx.lineTo(toCanvas(anchors.leftEye.x), toCanvas(anchors.leftEye.y));
      ctx.stroke();

      // Horizontal guides make it obvious where the head is measured from.
      ctx.setLineDash([4, 4]);
      ctx.strokeStyle = "rgba(230, 160, 60, 0.9)";
      ctx.lineWidth = 1;
      for (const p of [anchors.chin, anchors.crown]) {
        ctx.beginPath();
        ctx.moveTo(0, toCanvas(p.y));
        ctx.lineTo(w, toCanvas(p.y));
        ctx.stroke();
      }
      ctx.setLineDash([]);

      // Eyes are drawn smaller: they are usually right, and oversized handles
      // over the face would obscure what the user is judging.
      for (const [name, p, radius, colour] of [
        ["crown", anchors.crown, HANDLE_RADIUS_PX, "#e6a03c"],
        ["chin", anchors.chin, HANDLE_RADIUS_PX, "#e6a03c"],
        ["rightEye", anchors.rightEye, HANDLE_RADIUS_PX * 0.7, "#4682c8"],
        ["leftEye", anchors.leftEye, HANDLE_RADIUS_PX * 0.7, "#4682c8"],
      ] as const) {
        ctx.fillStyle = dragging === name ? "#e08a30" : colour;
        ctx.strokeStyle = "#ffffff";
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(toCanvas(p.x), toCanvas(p.y), radius, 0, Math.PI * 2);
        ctx.fill();
        ctx.stroke();
      }
    }
  }, [image, detection, anchors, crop, rotationDeg, showMask, mask, scale, dragging, toCanvas]);

  const hitTest = (mx: number, my: number): Handle => {
    if (!anchors) return null;
    // A generous radius: these are small targets on a large photo. Nearest
    // wins, so overlapping handles stay individually selectable.
    const grab = HANDLE_RADIUS_PX * 2.5;
    let best: Handle = null;
    let bestDist = Infinity;
    for (const name of ["chin", "crown", "rightEye", "leftEye"] as const) {
      const p = anchors[name];
      const dist = Math.hypot(mx - toCanvas(p.x), my - toCanvas(p.y));
      if (dist <= grab && dist < bestDist) {
        best = name;
        bestDist = dist;
      }
    }
    return best;
  };

  const pointerPos = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const rect = e.currentTarget.getBoundingClientRect();
    return { x: e.clientX - rect.left, y: e.clientY - rect.top };
  };

  /** Whether a point lies inside the crop rectangle, ignoring rotation. */
  const insideCrop = (mx: number, my: number) => {
    if (!crop) return false;
    const c = crop.rect;
    return (
      mx >= toCanvas(c.x) &&
      mx <= toCanvas(c.x + c.width) &&
      my >= toCanvas(c.y) &&
      my <= toCanvas(c.y + c.height)
    );
  };

  /** Canvas position to mask coordinates. */
  const toMask = (x: number, y: number): [number, number] | null => {
    if (!image || !mask) return null;
    return [
      (toSource(x) / image.width) * mask.width,
      (toSource(y) / image.height) * mask.height,
    ];
  };

  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    const { x, y } = pointerPos(e);

    // The eyedropper consumes the click outright, so a stray drag cannot move
    // an anchor while the user is only sampling a colour.
    if (onPickColour) {
      onPickColour(toSource(x), toSource(y));
      return;
    }

    // Painting takes priority: with a brush selected the user is editing the
    // mask, not repositioning the crop.
    if (brush && mask && onStroke) {
      const p = toMask(x, y);
      if (p) {
        strokePoints.current = [p];
        e.currentTarget.setPointerCapture(e.pointerId);
      }
      return;
    }

    const hit = hitTest(x, y);
    if (hit) {
      setDragging(hit);
      e.currentTarget.setPointerCapture(e.pointerId);
      return;
    }
    // Dragging inside the crop moves the whole thing, which is the quickest
    // way to correct framing without touching individual points.
    if (onCropNudge && insideCrop(x, y)) {
      cropDrag.current = { x, y };
      e.currentTarget.setPointerCapture(e.pointerId);
    }
  };

  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (!image) return;
    const { x, y } = pointerPos(e);

    if (strokePoints.current) {
      const p = toMask(x, y);
      if (p) strokePoints.current.push(p);
      return;
    }

    if (dragging) {
      // Clamp to the image: an anchor outside it has no meaning.
      const sx = Math.max(0, Math.min(image.width, toSource(x)));
      const sy = Math.max(0, Math.min(image.height, toSource(y)));
      onAnchorMove(dragging, sx, sy);
      return;
    }

    if (cropDrag.current && onCropNudge) {
      const dx = toSource(x - cropDrag.current.x);
      const dy = toSource(y - cropDrag.current.y);
      cropDrag.current = { x, y };
      onCropNudge(dx, dy);
      return;
    }

    // Hover feedback, so the draggable regions are discoverable.
    if (onPickColour) setCursor("copy");
    else if (brush) setCursor("cell");
    else if (hitTest(x, y)) setCursor("grab");
    else if (onCropNudge && insideCrop(x, y)) setCursor("move");
    else setCursor("crosshair");
  };

  const endDrag = (e: React.PointerEvent<HTMLCanvasElement>) => {
    // A stroke is sent once on release, not per pointer event: each one costs
    // a full mask recomputation.
    if (strokePoints.current) {
      const points = strokePoints.current;
      strokePoints.current = null;
      e.currentTarget.releasePointerCapture(e.pointerId);
      if (points.length > 0) onStroke?.(points);
      return;
    }
    if (dragging || cropDrag.current) {
      e.currentTarget.releasePointerCapture(e.pointerId);
      setDragging(null);
      cropDrag.current = null;
    }
  };

  if (!image) return null;

  return (
    <canvas
      ref={canvasRef}
      className="photo-canvas"
      style={{ cursor: dragging || cropDrag.current ? "grabbing" : cursor }}
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
    />
  );
}
