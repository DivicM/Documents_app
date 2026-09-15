/**
 * Minimal i18n layer. UI strings are keys from the start, so error messages
 * coming from Rust (which sends keys plus params, never prose) and strings
 * written in the UI resolve through the same table.
 */

type Params = Record<string, string | number>;

const hr: Record<string, string> = {
  "app.title": "Fotografije za dokumente",

  "step.print": "Ispis",

  "format.width": "Širina (mm)",
  "format.height": "Visina (mm)",
  "format.count": "Broj kopija",

  "paper.label": "Papir",
  "paper.10x15": "10 × 15 cm",
  "paper.13x18": "13 × 18 cm",
  "paper.a4": "A4 (210 × 297 mm)",

  "layout.margin": "Rub (mm)",
  "layout.gutter": "Razmak (mm)",
  "layout.capacity": "Stane na list: {capacity}",
  "layout.sheets_needed": "Potrebno listova: {sheets}",
  "layout.align_top_left": "Poravnaj u kut",
  "layout.cut_marks": "Okvir za rezanje",
  "layout.bottom_mark": "Crta samo ispod slike",
  "layout.cut_marks_hint":
    "Blijedi sivi okvir oko svake slike, ispisuje se na papir. Nalazi se izvan slike pa ga rezanjem uklonite.",
  "layout.rotated": "Slike su zakrenute za 90° radi boljeg iskorištenja papira",

  "printer.label": "Printer",
  "printer.none": "Nema instaliranih printera",
  "printer.dpi": "Rezolucija: {dpiX} × {dpiY} DPI",
  "printer.margins": "Hardverski rub: {left} / {top} / {right} / {bottom} mm",
  "printer.paper_from_driver": "Papir u printeru: {width} × {height} mm",
  "printer.paper_overridden":
    "Printer je namješten na {width} × {height} mm, pa se koristi ta veličina umjesto odabrane. Veličinu papira mijenjajte u postavkama printera u Windowsima.",
  "printer.refresh": "Osvježi",

  "calibration.never": "Nije kalibrirano",
  "calibration.done_at": "Kalibrirano: {date}",
  "calibration.print_square": "1. Ispiši kvadrat 50 × 50 mm",
  "calibration.print_verify": "Provjeri kalibraciju (ponovni ispis)",
  "calibration.measured_x": "Izmjerena širina (mm)",
  "calibration.measured_y": "Izmjerena visina (mm)",
  "calibration.save": "2. Spremi kalibraciju",
  "calibration.explain":
    "Ispišite kvadrat, izmjerite ga ravnalom i upišite stvarne mjere. Aplikacija će ispraviti razliku pri svakom sljedećem ispisu.",
  "calibration.correction": "Ispravak: {x} % vodoravno, {y} % okomito",
  "calibration.saved": "Kalibracija spremljena.",

  "print.button": "Ispiši",
  "print.sending": "Ispisujem…",
  "print.sent": "Poslano na printer (job {jobId})",

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

  "error.image.empty": "Slika je prazna.",
  "error.image.size_mismatch": "Neispravna veličina slike ({got} umjesto {expected} bajtova).",
  "error.image.decode_failed": "Nije moguće pročitati sliku.",
  "error.image.canvas_unavailable": "Grafički prikaz nije dostupan.",
  "error.image.not_raw": "Slika nije prenesena u ispravnom obliku.",
  "error.model.not_found": "Model za prepoznavanje lica nije pronađen.",
  "error.model.load_failed": "Nije moguće učitati model. ({detail})",
  "error.detect.failed": "Prepoznavanje lica nije uspjelo. ({detail})",
  "error.internal.lock": "Interna greška.",
  "error.crop.degenerate_head": "Brada i tjeme se poklapaju — pomaknite točke.",
  "error.segment.failed": "Uklanjanje pozadine nije uspjelo. ({detail})",
  "error.mask.unknown_mode": "Nepoznat način kista.",
  "error.mask.empty_stroke": "Potez je prazan.",
  "error.mask.not_segmented": "Prvo uklonite pozadinu.",
  "error.mask.size_mismatch": "Neispravna veličina maske.",

  "bg.remove": "Ukloni pozadinu",
  "bg.working": "Obrađujem…",
  "bg.enabled": "Zamijeni pozadinu",
  "bg.colour": "Boja pozadine",
  "bg.colour_white": "Bijela",
  "bg.threshold": "Prag maske",
  "bg.done": "Pozadina uklonjena ({backend}, {ratio} % osobe)",
  "bg.show_mask": "Prikaži masku",
  "error.crop.outside_image":
    "Izrez ne stane u sliku (nedostaje: lijevo {left}, gore {top}, desno {right}, dolje {bottom} px). Najmanja visina glave koja stane je {minHeadMm} mm.",

  "face.detecting": "Prepoznajem…",
  "face.none_found": "Lice nije pronađeno.",
  "face.reset": "Vrati",
  "face.chin": "Brada",
  "face.crown": "Tjeme",
  "face.rightEye": "Desno oko",
  "face.leftEye": "Lijevo oko",
  "face.drag_hint":
    "Povucite narančaste točke (brada, tjeme) ili plave (oči) ako automatika promaši. Pomicanje očiju mijenja i nagib.",
  "face.rotation": "Ispravljanje nagiba (°)",
  "face.apply_suggested": "Postavi visinu glave na {mm} mm",
  "face.reset_all": "Vrati sve na automatski",

  "error.spec.load_failed": "Nije moguće učitati specifikacije. ({detail})",
  "error.spec.not_found": "Nepoznata specifikacija: {id}.",

  "spec.free_mode": "Slobodan mod — bez provjera, samo upozorenje o rezoluciji.",

  "compliance.title": "Provjera",
  "compliance.checked": "Provjereno",
  "compliance.not_checked": "Ovo program ne mjeri",
  "compliance.all_ok": "Sve provjere prolaze.",
  "compliance.blocking": "Fotografija ne zadovoljava propis.",
  "compliance.fix": "Popravi",

  "rule.head_height.ok": "Visina glave: {actual} mm (traži se {min}–{max} mm)",
  "rule.head_height.too_small":
    "Glava je premala: {actual} mm, treba {min}–{max} mm",
  "rule.head_height.too_large":
    "Glava je prevelika: {actual} mm, treba {min}–{max} mm",
  "rule.eye_distance.ok": "Razmak očiju: {actual} mm (najmanje {min} mm)",
  "rule.eye_distance.too_small":
    "Razmak očiju je premali: {actual} mm, treba najmanje {min} mm",
  "rule.horizontal_center.ok": "Glava je centrirana (odstupanje {actual} mm)",
  "rule.horizontal_center.off":
    "Glava nije centrirana: odstupanje {actual} mm, dopušteno {tolerance} mm",
  "rule.pose_roll.ok": "Glava je uspravna ({actual}°)",
  "rule.pose_roll.tilted": "Glava je nagnuta {actual}°, dopušteno je {max}°",
  "rule.resolution.ok": "Rezolucija: {available} DPI (traži se {required})",
  "rule.resolution.too_low":
    "Premala rezolucija: {available} DPI, treba {required}. Ispis bi povećavao sliku.",
  "rule.background_uniform.ok": "Pozadina je ujednačena",
  "rule.background_uniform.uneven":
    "Pozadina nije ujednačena (odstupanje {actual}, dopušteno {max})",
  "rule.sharpness.ok": "Slika je oštra",
  "rule.sharpness.soft": "Slika je meka (oštrina {actual}, poželjno {min})",
  "rule.exposure.ok": "Ekspozicija je u redu",
  "rule.exposure.overexposed": "Presvijetlo: {highlights} % piksela je bijelo bez detalja",
  "rule.exposure.underexposed": "Pretamno: {shadows} % piksela je crno bez detalja",

  "rule.not_checked.head_width":
    "Širina glave se mjeri od uha do uha — detektor to ne vidi.",
  "rule.not_checked.pose_yaw": "Okretanje glave lijevo-desno se ne mjeri.",
  "rule.not_checked.pose_pitch": "Naginjanje glave gore-dolje se ne mjeri.",
  "rule.not_checked.eyes_open": "Jesu li oči otvorene — provjerite sami.",
  "rule.not_checked.mouth_closed": "Jesu li usta zatvorena — provjerite sami.",
  "rule.not_checked.background_shadows": "Sjene na pozadini se ne mjere.",
  "rule.not_checked.glasses_glare": "Odsjaj na naočalama se ne mjeri.",
  "rule.not_checked.red_eye": "Crvene oči se ne mjere.",
  "rule.not_checked.photo_age": "Starost fotografije program ne zna.",
  "rule.not_checked.background_uniform": "Prvo uklonite pozadinu da se izmjeri.",
  "rule.not_checked.sharpness": "Oštrina još nije izmjerena.",
  "rule.not_checked.exposure": "Ekspozicija još nije izmjerena.",
  "rule.not_checked.unknown": "Ovo pravilo program još ne provjerava.",

  "adjust.exposure": "Ekspozicija (EV)",
  "adjust.contrast": "Kontrast",
  "adjust.temperature": "Toplina",
  "adjust.tint": "Nijansa",
  "adjust.reset": "Vrati na neutralno",

  "edit.undo": "Poništi",
  "edit.redo": "Ponovi",
  "edit.undo_hint": "Ctrl+Z / Ctrl+Y",

  "preview.sheet": "Pregled ispisa",
  "preview.single": "Gotova slika ({width} × {height} mm)",
  "preview.zoom_in": "Uži izrez (veća glava)",
  "preview.zoom_out": "Širi izrez (manja glava)",
  "preview.no_image": "Učitajte sliku da vidite kako će list izgledati.",
  "preview.no_crop": "Izrez nije moguć — prilagodite visinu glave ili točke.",

  "error.preset.empty_name": "Predložak treba ime.",
  "error.preset.name_too_long": "Ime predloška ne smije biti dulje od {max} znakova.",
  "error.preset.not_found": "Predložak „{name}” ne postoji.",

  "preset.explain":
    "Spremite trenutne postavke (format, papir, rubove, pozadinu, svjetlinu) da ih ne morate ponovno namještati.",
  "preset.name": "Ime predloška",
  "preset.save": "Spremi postavke",
  "preset.load": "Učitaj",
  "preset.delete": "Obriši",
  "preset.none": "Nema spremljenih predložaka",
  "preset.saved": "Predložak „{name}” je spremljen.",
  "preset.loaded": "Predložak „{name}” je učitan.",
  "preset.deleted": "Predložak „{name}” je obrisan.",
  "preset.confirm_delete": "Obrisati predložak „{name}”?",
  "preset.overwrite_hint": "Ime koje već postoji zamijenit će stari predložak.",

  "mixed.enable": "Više formata na istom listu",
  "mixed.explain":
    "Stavite više različitih formata na isti papir — npr. 4 × osobna i 2 × putovnica.",
  "mixed.add": "Dodaj format",
  "mixed.remove": "Ukloni",
  "mixed.group": "Format {n}",
  "mixed.unplaced":
    "Ne stane sve na jedan list: {count} kom. formata {width} × {height} mm ostaje.",
  "mixed.all_placed": "Sve stane na jedan list ({count} kom.).",
  "mixed.empty": "Dodajte barem jedan format.",

  "settings.title": "Postavke",
  "settings.open": "Postavke",
  "settings.close": "Zatvori",
  "settings.saved": "Postavke su spremljene.",
  "settings.explain":
    "Namjesti se jednom i pamti se. Vrijedi za sve buduće ispise dok se ne promijeni.",
  "settings.tab_layout": "Raspored",
  "settings.tab_presets": "Predlošci",
  "settings.tab_calibration": "Kalibracija",
  "settings.tab_mixed": "Miješani list",
  "settings.save": "Spremi postavke",
  "settings.dark_mode": "Tamna tema",
  "settings.dark_mode_hint": "Ne utječe na ispis ni na boju pozadine fotografije.",
  "settings.start_maximized": "Pokreni preko cijelog zaslona",
  "settings.start_maximized_hint":
    "Vrijedi od sljedećeg pokretanja aplikacije.",
  "settings.font_scale": "Veličina teksta",
  "settings.font_scale_hint":
    "Povećava ili smanjuje cijelo sučelje. Ne utječe na ispis.",
  "settings.quarter_turn": "Vodoravni format (npr. 45 × 35 umjesto 35 × 45)",
  "settings.quarter_turn_hint":
    "Okreće oblik okvira na listu. Mijenja koliko slika stane na papir.",
  "settings.turn_photo": "Okreni sliku za 90° unutar okvira",
  "settings.turn_photo_hint":
    "Okreće sadržaj, ne okvir. Uz uključen vodoravni format lice će ležati na boku.",

  "wizard.step1": "Odabir slike",
  "wizard.step2": "Format",
  "wizard.step3": "Obrada",
  "wizard.step4": "Ispis",
  "wizard.next": "Dalje",
  "wizard.back": "Natrag",
  "wizard.start_over": "Nova fotografija",

  "drop.title": "Povucite fotografiju ovdje",
  "drop.or": "ili",
  "drop.browse": "Odaberi datoteku",
  "drop.hint": "JPEG ili PNG. Slika ostaje na vašem računalu.",
  "drop.active": "Pustite za učitavanje",
  "drop.rejected": "Nije slika. Odaberite JPEG ili PNG.",
  "drop.loaded": "Učitano: {name}",

  "picker.title": "Za koji dokument?",
  "picker.size": "{width} × {height} mm",
  "picker.group_hr": "Hrvatska",
  "picker.group_intl": "Međunarodno",
  "picker.group_other": "Ostalo",
  "picker.custom_title": "Vlastita veličina",
  "picker.custom_width": "Širina (mm)",
  "picker.custom_height": "Visina (mm)",
  "picker.custom_empty": "Upišite mjeru.",
  "picker.custom_not_a_number": "Upišite broj.",
  "picker.custom_not_positive": "Mjera mora biti veća od nule.",
  "picker.custom_out_of_range": "Dopušteno je {min}–{max} mm.",

  "editor.group_crop": "Kadriranje",
  "editor.group_tone": "Svjetlina i boja",
  "editor.group_background": "Pozadina",

  "print.done": "Ispisano. Možete započeti novu fotografiju.",

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
