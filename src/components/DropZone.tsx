import { useCallback, useRef, useState } from "react";
import { t } from "../lib/i18n";

interface Props {
  onPick: (file: File) => void;
  /** Name of the image already loaded, if any. */
  loadedName?: string | null;
}

/**
 * First step: choose a photo, by dropping it or through the file dialog.
 *
 * Both routes exist because they suit different habits — dragging from an
 * already-open folder, or browsing when the file's location is not in mind.
 */
export function DropZone({ onPick, loadedName }: Props) {
  const [dragging, setDragging] = useState(false);
  const [rejected, setRejected] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);
  /**
   * Drag events fire for every child element, so a plain boolean flickers as
   * the pointer crosses the inner text. Counting enter/leave pairs is stable.
   */
  const depth = useRef(0);

  const accept = useCallback(
    (file: File | undefined) => {
      if (!file) return;
      if (!file.type.startsWith("image/")) {
        setRejected(true);
        return;
      }
      setRejected(false);
      onPick(file);
    },
    [onPick],
  );

  return (
    <div
      className={`dropzone ${dragging ? "dropzone-active" : ""}`}
      onDragEnter={(e) => {
        e.preventDefault();
        depth.current += 1;
        setDragging(true);
      }}
      onDragOver={(e) => e.preventDefault()}
      onDragLeave={(e) => {
        e.preventDefault();
        depth.current -= 1;
        if (depth.current <= 0) {
          depth.current = 0;
          setDragging(false);
        }
      }}
      onDrop={(e) => {
        e.preventDefault();
        depth.current = 0;
        setDragging(false);
        accept(e.dataTransfer.files?.[0]);
      }}
    >
      <input
        ref={inputRef}
        type="file"
        accept="image/*"
        style={{ display: "none" }}
        onChange={(e) => {
          accept(e.target.files?.[0]);
          // Clear, so picking the same file twice still fires a change.
          e.target.value = "";
        }}
      />

      <div className="dropzone-icon" aria-hidden="true">
        {dragging ? "⤓" : "🖼"}
      </div>

      <p className="dropzone-title">
        {dragging ? t("drop.active") : t("drop.title")}
      </p>

      {!dragging && (
        <>
          <p className="dropzone-or">{t("drop.or")}</p>
          <button
            type="button"
            className="primary"
            onClick={() => inputRef.current?.click()}
            title={t("drop.hint")}
          >
            {t("drop.browse")}
          </button>
        </>
      )}

      {rejected && <p className="error dropzone-error">{t("drop.rejected")}</p>}
      {loadedName && !rejected && (
        <p className="status dropzone-loaded">{t("drop.loaded", { name: loadedName })}</p>
      )}
    </div>
  );
}
