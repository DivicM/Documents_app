import { useEffect, useState, type ReactNode } from "react";
import { t } from "../lib/i18n";

const TABS = [
  "settings.tab_layout",
  "settings.tab_mixed",
  "settings.tab_presets",
  "settings.tab_calibration",
] as const;

interface Props {
  open: boolean;
  onClose: () => void;
  /** One pane per tab, in the order of TABS. */
  panes: [ReactNode, ReactNode, ReactNode, ReactNode];
  footer?: ReactNode;
}

/**
 * Settings that are configured once and then left alone.
 *
 * A dialog rather than a step in the wizard: none of this changes from photo to
 * photo, and having it inline made the print step long enough to scroll.
 */
export function SettingsDialog({ open, onClose, panes, footer }: Props) {
  const [tab, setTab] = useState(0);

  // Escape closes, which is what a dialog is expected to do.
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  return (
    <div
      className="modal-backdrop"
      // Only a click on the backdrop itself closes; one that started inside the
      // dialog and drifted out must not.
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div className="modal" role="dialog" aria-modal="true" aria-label={t("settings.title")}>
        <header className="modal-header">
          <h2>{t("settings.title")}</h2>
          <button type="button" className="small" onClick={onClose}>
            {t("settings.close")}
          </button>
        </header>

        <p className="hint modal-explain">{t("settings.explain")}</p>

        <nav className="modal-tabs">
          {TABS.map((key, i) => (
            <button
              key={key}
              type="button"
              className={`modal-tab ${i === tab ? "modal-tab-active" : ""}`}
              aria-selected={i === tab}
              onClick={() => setTab(i)}
            >
              {t(key)}
            </button>
          ))}
        </nav>

        <div className="modal-body">{panes[tab]}</div>

        {footer && <footer className="modal-footer">{footer}</footer>}
      </div>
    </div>
  );
}
