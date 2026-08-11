import { type ReactNode } from "react";

interface Props {
  title: string;
  /** Short status shown in the header, e.g. whether edits are active. */
  badge?: string | null;
  children: ReactNode;
}

/**
 * A titled group of editing controls.
 *
 * These were collapsible, but every group is worth seeing at a glance and
 * collapsing only added a click between the user and the control they wanted.
 * The heading now just labels the group.
 */
export function Section({ title, badge, children }: Props) {
  return (
    <section className="edit-section">
      <h3 className="edit-section-header">
        <span className="edit-section-title">{title}</span>
        {badge && <span className="badge">{badge}</span>}
      </h3>

      <div className="edit-section-body">{children}</div>
    </section>
  );
}
