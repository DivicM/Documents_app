import { useState, type ReactNode } from "react";

interface Props {
  title: string;
  /** Open on first render. Later state is the user's. */
  defaultOpen?: boolean;
  /** Short status shown in the header, e.g. how many edits are active. */
  badge?: string | null;
  children: ReactNode;
}

/**
 * A collapsible group of editing controls.
 *
 * The editor has enough controls that a flat list buries the ones in use.
 * Grouping them keeps each concern — framing, tone, background — closed until
 * it is wanted.
 */
export function Section({ title, defaultOpen = false, badge, children }: Props) {
  const [open, setOpen] = useState(defaultOpen);

  return (
    <section className={`edit-section ${open ? "edit-section-open" : ""}`}>
      <button
        type="button"
        className="edit-section-header"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        <span className="edit-section-chevron" aria-hidden="true">
          {open ? "▾" : "▸"}
        </span>
        <span className="edit-section-title">{title}</span>
        {badge && <span className="badge">{badge}</span>}
      </button>

      {open && <div className="edit-section-body">{children}</div>}
    </section>
  );
}
