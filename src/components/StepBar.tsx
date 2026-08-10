import { t } from "../lib/i18n";

export const STEPS = ["wizard.step1", "wizard.step2", "wizard.step3", "wizard.step4"] as const;

interface Props {
  /** Zero-based index of the active step. */
  current: number;
  /** Jump back to an already-completed step. Forward moves use Next. */
  onGoTo: (step: number) => void;
}

/** Progress across the four steps, with completed ones clickable. */
export function StepBar({ current, onGoTo }: Props) {
  return (
    <nav className="stepbar" aria-label={t("app.title")}>
      {STEPS.map((key, i) => {
        const done = i < current;
        const active = i === current;
        return (
          <button
            key={key}
            type="button"
            className={`step ${active ? "step-active" : ""} ${done ? "step-done" : ""}`}
            aria-current={active ? "step" : undefined}
            // Going forward has preconditions the bar cannot know about, so
            // only backward jumps are offered here.
            disabled={!done}
            onClick={() => done && onGoTo(i)}
          >
            <span className="step-number">{done ? "✓" : i + 1}</span>
            <span className="step-label">{t(key)}</span>
          </button>
        );
      })}
    </nav>
  );
}
