//! Runs background segmentation on a photo and writes the mask out.
//!
//!   cargo run -p vision --example segment_photo -- test-data/5677.png
//!
//! Writes <input>.mask.png and <input>.cutout.png next to the input so the
//! result can be inspected. Runs entirely locally.

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: segment_photo <image.png>");
        std::process::exit(1);
    });

    let dll = std::path::Path::new("runtime/onnxruntime.dll");
    if dll.exists() {
        // SAFETY: single-threaded startup, before any ort call.
        unsafe { std::env::set_var("ORT_DYLIB_PATH", dll.canonicalize().unwrap()) };
    }

    let img = match image::open(&path) {
        Ok(i) => i.to_rgb8(),
        Err(e) => {
            eprintln!("could not read {path}: {e}");
            std::process::exit(1);
        }
    };
    println!("image: {} x {} px", img.width(), img.height());

    let model = std::path::Path::new("models/u2netp.onnx");
    // `--cpu` forces the CPU provider, so the two backends can be compared on
    // the same machine and image.
    let force_cpu = std::env::args().any(|a| a == "--cpu");
    let mut seg = match if force_cpu {
        vision::segment::BackgroundSegmenter::cpu_only(model)
    } else {
        vision::segment::BackgroundSegmenter::from_path(model)
    } {
        Ok(s) => s,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    println!("backend: {:?}", seg.backend());

    let started = std::time::Instant::now();
    let mask = match seg.segment(&img) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let first = started.elapsed();
    println!("mask {} x {} in {first:?} (first run)", mask.width, mask.height);

    // Several warm runs. `--pause` waits between them, which distinguishes a
    // session that degrades from a GPU that drops to a low-power state while
    // idle: the first tells us to fix the code, the second tells us not to.
    let pause = std::env::args().any(|a| a == "--pause");
    for i in 2..=5 {
        if pause {
            std::thread::sleep(std::time::Duration::from_secs(5));
        }
        let again = std::time::Instant::now();
        if seg.segment(&img).is_ok() {
            println!("run {i}: {:?}", again.elapsed());
        }
    }

    // Split the warm cost into its parts, so optimisation targets the part that
    // actually dominates rather than the one that looks suspicious.
    let t = std::time::Instant::now();
    let pre = vision::segment::preprocess_for_bench(&img);
    println!("  preprocess only: {:?} ({} floats)", t.elapsed(), pre.len());

    // A quick sanity read: a portrait should be part subject, part background,
    // not uniformly one or the other.
    let total = mask.data.len() as f64;
    let subject = mask.data.iter().filter(|&&v| v > 128).count() as f64;
    println!("subject coverage: {:.1}%", subject / total * 100.0);
    if subject / total > 0.98 {
        println!("  warning: almost everything is subject, the mask may be wrong");
    } else if subject / total < 0.02 {
        println!("  warning: almost nothing is subject, the mask may be wrong");
    }

    // Write the mask as greyscale.
    let mask_img = image::GrayImage::from_raw(mask.width, mask.height, mask.data.clone())
        .expect("mask dimensions");
    let mask_path = format!("{path}.mask.png");
    if let Err(e) = mask_img.save(&mask_path) {
        eprintln!("could not write {mask_path}: {e}");
    } else {
        println!("wrote {mask_path}");
    }

    // And the subject on the light grey the Croatian guidance prefers.
    let mut rgba: Vec<u8> = img
        .pixels()
        .flat_map(|p| [p[0], p[1], p[2], 255])
        .collect();
    domain::mask::composite_background(
        &mut rgba,
        img.width(),
        img.height(),
        &mask,
        [235, 235, 235],
    );
    let cutout = image::RgbaImage::from_raw(img.width(), img.height(), rgba)
        .expect("cutout dimensions");
    let cutout_path = format!("{path}.cutout.png");
    if let Err(e) = cutout.save(&cutout_path) {
        eprintln!("could not write {cutout_path}: {e}");
    } else {
        println!("wrote {cutout_path}");
    }
}
