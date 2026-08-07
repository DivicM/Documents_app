//! Runs face detection on a photo and prints what it found.
//!
//!   cargo run -p vision --example detect_photo -- test-data/5677.png
//!
//! Reads the file from disk and runs entirely locally; nothing leaves the
//! machine.

use domain::geometry::estimate_head_anchors;
use domain::geometry::{solve_crop, CropTarget, Rect};
use vision::decode::primary_face;
use vision::detect::FaceDetector;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: detect_photo <image.png>");
        std::process::exit(1);
    });

    // ort with `load-dynamic` needs to be told where the runtime library is.
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

    let model = std::path::Path::new("models/face_detection_yunet_2023mar.onnx");
    let mut detector = match FaceDetector::from_path(model) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("could not load model: {e}");
            std::process::exit(1);
        }
    };

    let started = std::time::Instant::now();
    let faces = match detector.detect(&img) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("detection failed: {e}");
            std::process::exit(1);
        }
    };
    let elapsed = started.elapsed();

    println!("detected {} face(s) in {:?}", faces.len(), elapsed);
    if faces.is_empty() {
        println!("\nNo face found. Either the photo has none, or decoding is wrong.");
        return;
    }

    for (i, f) in faces.iter().enumerate() {
        println!(
            "\nface {i}: confidence {:.3}\n  box    {:.0},{:.0}  {:.0}x{:.0}",
            f.confidence, f.bbox.x, f.bbox.y, f.bbox.width, f.bbox.height
        );
        println!(
            "  eyes   R({:.0},{:.0})  L({:.0},{:.0})  distance {:.0}px",
            f.landmarks.right_eye.x,
            f.landmarks.right_eye.y,
            f.landmarks.left_eye.x,
            f.landmarks.left_eye.y,
            f.landmarks.eye_distance()
        );
        println!("  roll   {:.1} deg", f.landmarks.roll_degrees());
    }

    let Some(face) = primary_face(&faces) else { return };
    let anchors = estimate_head_anchors(face);
    println!(
        "\nestimated chin ({:.0},{:.0}), crown ({:.0},{:.0}) -> head {:.0}px",
        anchors.chin.x,
        anchors.chin.y,
        anchors.crown.x,
        anchors.crown.y,
        anchors.chin.y - anchors.crown.y
    );

    // Croatian passport: 35x45mm with an adult head of 31.5-36mm.
    let target = CropTarget {
        photo_width_mm: 35.0,
        photo_height_mm: 45.0,
        head_height_mm: 33.75,
        chin_from_bottom_mm: None,
    };
    let bounds = Rect::new(0.0, 0.0, img.width() as f64, img.height() as f64);

    match solve_crop(&anchors, &target, &bounds) {
        Ok(c) => {
            println!(
                "crop  {:.0},{:.0}  {:.0}x{:.0} px",
                c.x, c.y, c.width, c.height
            );
            let dpi_w = c.width * 25.4 / 35.0;
            let dpi_h = c.height * 25.4 / 45.0;
            println!("max lossless dpi: {:.0}", dpi_w.min(dpi_h));
            if dpi_w.min(dpi_h) < 300.0 {
                println!("  below 300 dpi: printing would upscale");
            }
        }
        Err(domain::geometry::CropError::CropOutsideImage {
            overflow_left_px,
            overflow_top_px,
            overflow_right_px,
            overflow_bottom_px,
            min_head_height_mm,
        }) => {
            println!("crop does not fit the image:");
            for (edge, px) in [
                ("left", overflow_left_px),
                ("top", overflow_top_px),
                ("right", overflow_right_px),
                ("bottom", overflow_bottom_px),
            ] {
                if px > 0.0 {
                    println!("  {edge} edge short by {px:.0}px");
                }
            }
            println!("  smallest head height that fits: {min_head_height_mm:.1}mm");
            if min_head_height_mm > 36.0 {
                println!(
                    "  Croatian passport allows 31.5-36mm, so this photo cannot\n  \
                     satisfy it: the subject is framed too close."
                );
            }
        }
        Err(e) => println!("crop not possible: {e:?}"),
    }
}
