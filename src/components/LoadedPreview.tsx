import { useEffect, useRef } from "react";

interface Props {
  /** The reduced working copy, already decoded. */
  canvas: HTMLCanvasElement;
  name: string | null;
}

/**
 * The loaded photograph, shown before anything is done to it.
 *
 * Large enough to judge by: whether the right file was picked, whether the
 * subject's eyes are open, whether it is worth continuing at all. Drawn from
 * the working copy that detection already produced rather than decoding the
 * file a second time.
 */
export function LoadedPreview({ canvas, name }: Props) {
  const hostRef = useRef<HTMLCanvasElement>(null);

  useEffect(() => {
    const target = hostRef.current;
    if (!target) return;
    target.width = canvas.width;
    target.height = canvas.height;
    const ctx = target.getContext("2d");
    if (!ctx) return;
    ctx.drawImage(canvas, 0, 0);
  }, [canvas]);

  return (
    <figure className="loaded-preview">
      <canvas ref={hostRef} />
      {name && <figcaption>{name}</figcaption>}
    </figure>
  );
}
