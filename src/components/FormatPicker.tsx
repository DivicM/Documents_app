import { useMemo } from "react";
import { t } from "../lib/i18n";
import type { SpecSummary } from "../lib/ipc";
import { formatMm } from "../lib/units";

/// The spec whose size the user types in themselves.
const CUSTOM_ID = "free-custom";

interface Props {
  specs: SpecSummary[];
  selectedId: string;
  onSelect: (id: string) => void;
  /** Current custom size, so its card shows what is being typed elsewhere. */
  customWidthMm: number;
  customHeightMm: number;
}

/** Which heading a format sits under. */
function groupOf(spec: SpecSummary): string {
  if (spec.country === "HR") return "picker.group_hr";
  if (spec.country) return "picker.group_intl";
  return "picker.group_other";
}

/**
 * Second step: pick the document the photo is for.
 *
 * Each entry states its size and the head height the rules require, so the
 * choice is made on the numbers rather than on a name alone. Formats without a
 * verified source say so — the brief forbids presenting a guessed figure as if
 * it were regulation.
 */
export function FormatPicker({
  specs,
  selectedId,
  onSelect,
  customWidthMm,
  customHeightMm,
}: Props) {
  const groups = useMemo(() => {
    const order = ["picker.group_hr", "picker.group_intl", "picker.group_other"];
    const byGroup = new Map<string, SpecSummary[]>();
    for (const spec of specs) {
      const key = groupOf(spec);
      const list = byGroup.get(key);
      if (list) list.push(spec);
      else byGroup.set(key, [spec]);
    }
    return order.filter((k) => byGroup.has(k)).map((k) => [k, byGroup.get(k)!] as const);
  }, [specs]);

  return (
    <div className="format-picker">
      <h2>{t("picker.title")}</h2>

      {groups.map(([groupKey, items]) => (
        <section key={groupKey} className="format-group">
          <h3 className="format-group-title">{t(groupKey)}</h3>

          {items.map((spec) => {
            const selected = spec.id === selectedId;
            const custom = spec.id === CUSTOM_ID;
            return (
              <button
                key={spec.id}
                type="button"
                className={`format-card ${selected ? "format-card-selected" : ""}`}
                aria-pressed={selected}
                onClick={() => onSelect(spec.id)}
              >
                <span className="format-card-size">
                  {t("picker.size", {
                    // The custom format's size is the one being typed, not the
                    // placeholder the spec file carries.
                    width: formatMm(custom ? customWidthMm : spec.widthMm, 0),
                    height: formatMm(custom ? customHeightMm : spec.heightMm, 0),
                  })}
                </span>

                <span className="format-card-body">
                  <span className="format-card-name">{spec.name}</span>
                  {spec.freeMode && (
                    <span className="format-card-detail">{t("spec.free_mode")}</span>
                  )}
                </span>
              </button>
            );
          })}
        </section>
      ))}
    </div>
  );
}
