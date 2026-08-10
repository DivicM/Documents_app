import { useMemo } from "react";
import { t } from "../lib/i18n";
import type { SpecSummary } from "../lib/ipc";
import { formatMm } from "../lib/units";

interface Props {
  specs: SpecSummary[];
  selectedId: string;
  onSelect: (id: string) => void;
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
export function FormatPicker({ specs, selectedId, onSelect }: Props) {
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
      <p className="hint">{t("picker.hint")}</p>

      {groups.map(([groupKey, items]) => (
        <section key={groupKey} className="format-group">
          <h3 className="format-group-title">{t(groupKey)}</h3>

          {items.map((spec) => {
            const selected = spec.id === selectedId;
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
                    width: formatMm(spec.widthMm, 0),
                    height: formatMm(spec.heightMm, 0),
                  })}
                </span>

                <span className="format-card-body">
                  <span className="format-card-name">{spec.name}</span>
                  <span className="format-card-detail">
                    {spec.headHeightMm !== null
                      ? t("picker.head", { mm: formatMm(spec.headHeightMm, 1) })
                      : t("spec.free_mode")}
                  </span>
                  {spec.confidence !== "verified" && !spec.freeMode && (
                    <span className="format-card-warning" title={t("picker.unverified_hint")}>
                      ⚠ {t("picker.unverified")}
                    </span>
                  )}
                </span>
              </button>
            );
          })}
        </section>
      ))}

      <p className="hint">{t("picker.unverified_hint")}</p>
    </div>
  );
}
