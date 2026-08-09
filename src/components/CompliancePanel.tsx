import { t } from "../lib/i18n";
import type { FixHint, RuleResult, Validation } from "../lib/ipc";

interface Props {
  validation: Validation | null;
  onApplyFix: (fix: FixHint) => void;
}

const ICONS: Record<string, string> = {
  pass: "✅",
  warn: "⚠️",
  fail: "❌",
};

/**
 * Shows which regulation criteria the photo meets.
 *
 * Split in two deliberately. The first section is what the program actually
 * measured; the second lists criteria the spec contains but no code here can
 * check. Showing the second as passing would promise a guarantee the app
 * cannot give, which the brief forbids and the disclaimer denies.
 */
export function CompliancePanel({ validation, onApplyFix }: Props) {
  if (!validation || validation.results.length === 0) return null;

  const checked = validation.results.filter((r) => r.status !== "not_checked");
  const unchecked = validation.results.filter((r) => r.status === "not_checked");

  // Failures first: they are what the user has to act on.
  const order: Record<string, number> = { fail: 0, warn: 1, pass: 2 };
  const sorted = [...checked].sort(
    (a, b) => (order[a.status] ?? 3) - (order[b.status] ?? 3),
  );

  return (
    <section className="panel compliance">
      <h2>{t("compliance.title")}</h2>

      {validation.blocking ? (
        <div className="error">{t("compliance.blocking")}</div>
      ) : (
        checked.every((r) => r.status === "pass") && (
          <div className="status">{t("compliance.all_ok")}</div>
        )
      )}

      {sorted.length > 0 && (
        <>
          <h3>{t("compliance.checked")}</h3>
          <ul className="rule-list">
            {sorted.map((r) => (
              <RuleRow key={r.ruleId} rule={r} onApplyFix={onApplyFix} />
            ))}
          </ul>
        </>
      )}

      {unchecked.length > 0 && (
        <>
          <h3>{t("compliance.not_checked")}</h3>
          <ul className="rule-list muted">
            {unchecked.map((r) => (
              <li key={r.ruleId} className="rule-row">
                <span className="rule-icon">—</span>
                <span className="rule-text">{t(r.messageKey, r.params as never)}</span>
              </li>
            ))}
          </ul>
        </>
      )}

      <p className="disclaimer-inline">{t("disclaimer")}</p>
    </section>
  );
}

function RuleRow({
  rule,
  onApplyFix,
}: {
  rule: RuleResult;
  onApplyFix: (fix: FixHint) => void;
}) {
  return (
    <li className={`rule-row rule-${rule.status}`}>
      <span className="rule-icon">{ICONS[rule.status] ?? "—"}</span>
      <span className="rule-text">
        {t(rule.messageKey, rule.params as never)}
        {rule.fixHint && (
          <button
            type="button"
            className="small fix-button"
            onClick={() => rule.fixHint && onApplyFix(rule.fixHint)}
          >
            {t("compliance.fix")}
          </button>
        )}
      </span>
    </li>
  );
}
