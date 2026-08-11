import { t } from "../lib/i18n";

interface Props {
  label: string;
  value: number;
  /** One press of a button changes the value by this much. */
  step: number;
  min: number;
  max: number;
  onChange: (value: number) => void;
  /** Decimal places shown. */
  decimals?: number;
  /** Appended to the number, e.g. "°". */
  unit?: string;
  /** Whether the value is still the automatic one, which disables Reset. */
  isAuto?: boolean;
  /** Reset to automatic. Omitted when there is nothing to reset to. */
  onReset?: () => void;
}

/**
 * A label with a value between two buttons.
 *
 * Replaces the sliders these controls used to be: a slider needs a careful drag
 * to land on a value, while these are one press per step and read clearly at a
 * glance. The value is clamped here so callers cannot go out of range.
 */
export function Stepper({
  label,
  value,
  step,
  min,
  max,
  onChange,
  decimals = 2,
  unit = "",
  isAuto = false,
  onReset,
}: Props) {
  const clamp = (v: number) => Math.min(max, Math.max(min, v));
  // Rounded before comparing, so floating-point drift does not disable a
  // button one step early.
  const at = (v: number) => Math.abs(clamp(v) - value) < 1e-9;

  return (
    <div className="stepper">
      <span className="stepper-label">{label}</span>

      <div className="stepper-controls">
        <button
          type="button"
          className="stepper-button stepper-button-down"
          disabled={at(value - step)}
          onClick={() => onChange(clamp(value - step))}
          aria-label={`${label} −`}
        >
          −
        </button>

        <span className="stepper-value">
          {value.toFixed(decimals).replace(".", ",")}
          {unit}
        </span>

        <button
          type="button"
          className="stepper-button stepper-button-up"
          disabled={at(value + step)}
          onClick={() => onChange(clamp(value + step))}
          aria-label={`${label} +`}
        >
          +
        </button>

        {onReset && (
          <button
            type="button"
            className="small stepper-reset"
            disabled={isAuto}
            onClick={onReset}
          >
            {t("face.reset")}
          </button>
        )}
      </div>
    </div>
  );
}
