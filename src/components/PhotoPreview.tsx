import { useEffect, useRef } from "react";
import type { CropResult } from "../lib/ipc";
import { t } from "../lib/i18n";
import { formatMm } from "../lib/units";

interface Props {
  /**
   * The photo with tone and background already applied, or the original.
   *
   * A canvas as well as an image: the composited copy is produced on a canvas,
   * and handing it over directly avoids a PNG encode and decode per edit.
   */
  image: HTMLImageElement | HTMLCanvasElement | null;
  crop: CropResult | null;
  /** Straightening angle, in degrees. */
  rotationDeg: number;
  /** Print size in millimetres, which sets the preview's aspect ratio. */
  widthMm: number;
  heightMm: number;
  /** Longest edge of the drawn preview, in CSS pixels. */
  maxEdgePx?: number;
  /**
   * Tighten or loosen the crop by one step.
   *
   * Zooming in means a larger head on the same print, which is the same thing
   * as a smaller crop rectangle — so this adjusts the head height rather than
   * scaling anything on screen.
   */
  onZoom?: (direction: 1 | -1) => void;
  canZoomIn?: boolean;
  canZoomOut?: boolean;
}

/**
 * A single photo at its finished size, updating as the edits change.
 *
 * The editor canvas shows the whole photograph with guides over it, which is
 * right for adjusting but does not answer "what comes out of the printer".
 * This draws only the crop, straightened, at the target aspect ratio — the same
 * steps the renderer performs, so what is shown here is what gets printed.
 */
export function PhotoPreview({
  image,
  crop,
  rotationDeg,
  widthMm,
  heightMm,
  maxEdgePx = 190,
  onZoom,
  canZoomIn = true,
  canZoomOut = true,
}: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext("2d");
    if (!ctx) return;

    // Fit the target size inside a square of maxEdgePx, so portrait and
    // landscape formats both stay within the panel.
    const scale = maxEdgePx / Math.max(widthMm, heightMm);
    const cssW = widthMm * scale;
    const cssH = heightMm * scale;
    const dpr = window.devicePixelRatio || 1;

    canvas.width = Math.round(cssW * dpr);
    canvas.height = Math.round(cssH * dpr);
    canvas.style.width = `${cssW}px`;
    canvas.style.height = `${cssH}px`;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);

    ctx.clearRect(0, 0, cssW, cssH);
    // Placeholder shown before a crop exists. Read from the stylesheet so it
    // follows the theme; a literal grey would glare in the dark one.
    ctx.fillStyle =
      getComputedStyle(document.documentElement).getPropertyValue("--bg").trim() ||
      "#f0f1f3";
    ctx.fillRect(0, 0, cssW, cssH);

    if (!image || !crop) return;
    const c = crop.rect;

    ctx.save();
    ctx.beginPath();
    ctx.rect(0, 0, cssW, cssH);
    ctx.clip();

    // Same order as SheetPreview.drawPhoto and the Rust renderer: work from the
    // centre, straighten about the crop centre, then scale the crop to fill.
    ctx.translate(cssW / 2, cssH / 2);
    ctx.scale(cssW / c.width, cssH / c.height);
    ctx.rotate((-rotationDeg * Math.PI) / 180);
    ctx.translate(-(c.x + c.width / 2), -(c.y + c.height / 2));
    const srcW = image instanceof HTMLCanvasElement ? image.width : image.naturalWidth;
    const srcH = image instanceof HTMLCanvasElement ? image.height : image.naturalHeight;
    ctx.drawImage(image, 0, 0, srcW, srcH);
    ctx.restore();
  }, [image, crop, rotationDeg, widthMm, heightMm, maxEdgePx]);

  return (
    <div className="photo-preview">
      <div className="photo-preview-header">
        <span className="photo-preview-label">
          {t("preview.single", {
            width: formatMm(widthMm, 0),
            height: formatMm(heightMm, 0),
          })}
        </span>
      </div>

      <div className="photo-preview-viewport">
        <canvas ref={canvasRef} className="photo-preview-canvas" />
      </div>

      {!crop && <span className="hint">{t("preview.no_crop")}</span>}

      {/* Below the photo, where there is room for targets big enough to hit
          comfortably. These change the crop, not the display. */}
      {onZoom && (
        <div className="photo-preview-zoom">
          <button
            type="button"
            className="zoom-button zoom-button-out"
            disabled={!canZoomOut}
            onClick={() => onZoom(-1)}
            title={t("preview.zoom_out")}
            aria-label={t("preview.zoom_out")}
          >
            −
          </button>
          <button
            type="button"
            className="zoom-button zoom-button-in"
            disabled={!canZoomIn}
            onClick={() => onZoom(1)}
            title={t("preview.zoom_in")}
            aria-label={t("preview.zoom_in")}
          >
            +
          </button>
        </div>
      )}
    </div>
  );
}
