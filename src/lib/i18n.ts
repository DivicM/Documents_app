/**
 * Minimal i18n layer. UI strings are keys from the start, so error messages
 * coming from Rust (which sends keys plus params, never prose) and strings
 * written in the UI resolve through the same table.
 */

type Params = Record<string, string | number>;

const hr: Record<string, string> = {
  "app.title": "Fotografije za dokumente",

  "step.source": "Izvor",
  "step.format": "Format",
  "step.layout": "Raspored",
  "step.print": "Ispis",

  "format.custom": "Prilagođeno",
  "format.width": "Širina (mm)",
  "format.height": "Visina (mm)",
  "format.count": "Broj kopija",
  "format.dpi": "DPI",

  "paper.label": "Papir",
  "paper.10x15": "10 × 15 cm",
  "paper.13x18": "13 × 18 cm",
  "paper.a4": "A4 (210 × 297 mm)",

  "layout.margin": "Rub (mm)",
  "layout.gutter": "Razmak (mm)",
  "layout.capacity": "Stane na list: {capacity}",
  "layout.sheets_needed": "Potrebno listova: {sheets}",
  "layout.align_top_left": "Poravnaj u kut",
  "layout.cut_marks": "Linije za rezanje",
  "layout.rotated": "Slike su zakrenute za 90° radi boljeg iskorištenja papira",

  "printer.label": "Printer",
  "printer.none": "Nema instaliranih printera",
  "printer.dpi": "Rezolucija: {dpiX} × {dpiY} DPI",
  "printer.margins": "Hardverski rub: {left} / {top} / {right} / {bottom} mm",
  "printer.refresh": "Osvježi",

  "calibration.title": "Kalibracija",
  "calibration.never": "Nije kalibrirano",
  "calibration.done_at": "Kalibrirano: {date}",
  "calibration.print_square": "1. Ispiši kvadrat 50 × 50 mm",
  "calibration.print_verify": "Provjeri kalibraciju (ponovni ispis)",
  "calibration.measured_x": "Izmjerena širina (mm)",
  "calibration.measured_y": "Izmjerena visina (mm)",
  "calibration.save": "2. Spremi kalibraciju",
  "calibration.recalibrate": "Rekalibriraj",
  "calibration.explain":
    "Ispišite kvadrat, izmjerite ga ravnalom i upišite stvarne mjere. Aplikacija će ispraviti razliku pri svakom sljedećem ispisu.",
  "calibration.correction": "Ispravak: {x} % vodoravno, {y} % okomito",
  "calibration.applies_to": "Vrijedi za: {printer}, {paper}",
  "calibration.saved": "Kalibracija spremljena.",

  "print.button": "Ispiši",
  "print.sent": "Poslano na printer (job {jobId})",

  "resolution.insufficient":
    "Za {targetW} × {targetH} mm na {dpi} DPI treba {reqW} × {reqH} px, a slika ima {srcW} × {srcH} px.",
  "resolution.max_dpi": "Bez povećavanja moguće je najviše {dpi} DPI.",

  "error.printer.list_failed": "Nije moguće dohvatiti popis printera. ({detail})",
  "error.printer.dpi_failed": "Nije moguće pročitati rezoluciju printera. ({detail})",
  "error.printer.margins_failed": "Nije moguće pročitati rubove printera. ({detail})",
  "error.layout.photo_too_large":
    "Slika {photoWidth} × {photoHeight} mm ne stane na papir {paperWidth} × {paperHeight} mm.",
  "error.layout.no_usable_area": "Rubovi su preveliki — na papiru ne ostaje mjesta.",
  "error.layout.invalid_dimensions": "Neispravne dimenzije.",
  "error.calibration.implausible":
    "Izmjerena vrijednost odstupa više od 10 % (x {scaleX}, y {scaleY}). Provjerite mjeru i veličinu papira.",
  "error.calibration.invalid": "Neispravna mjera. ({detail})",
  "error.config.no_path": "Nije moguće odrediti mjesto za postavke.",
  "error.config.load_failed": "Nije moguće učitati postavke. ({detail})",
  "error.config.save_failed": "Nije moguće spremiti postavke. ({detail})",
  "error.print.failed": "Ispis nije uspio. ({detail})",
  "error.print.nothing_to_print": "Nema ničega za ispis.",
  "error.platform.unsupported": "Ova značajka nije podržana na ovoj platformi.",
  "error.unknown": "Nepoznata greška.",

  "disclaimer":
    "Provjeravaju se samo mjerljivi kriteriji. Službenik može odbiti fotografiju i iz razloga koje program ne mjeri.",
};

/** Replace {placeholders} with values. Missing keys surface as the key itself. */
export function t(key: string, params?: Params): string {
  const template = hr[key];
  if (template === undefined) return key;
  if (!params) return template;

  return template.replace(/\{(\w+)\}/g, (match, name: string) => {
    const value = params[name];
    return value === undefined ? match : String(value);
  });
}

/** Error shape crossing the IPC boundary. */
export interface UiError {
  key: string;
  params?: Record<string, unknown>;
}

/** Turn an error from Rust, or any thrown value, into a readable message. */
export function formatError(err: unknown): string {
  if (err && typeof err === "object" && "key" in err) {
    const e = err as UiError;
    return t(e.key, (e.params ?? {}) as Params);
  }
  if (typeof err === "string") return err;
  if (err instanceof Error) return err.message;
  return t("error.unknown");
}
