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

    let model = std::path::Path::new("models/birefnet_lite_fp16.onnx");
    let mut seg = match vision::segment::BackgroundSegmenter::from_path(model) {
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

    // A second run reuses the warmed session, which is what the user actually
    // experiences after the first photo.
    let again = std::time::Instant::now();
    if seg.segment(&img).is_ok() {
        println!("second run: {:?}", again.elapsed());
    }

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
