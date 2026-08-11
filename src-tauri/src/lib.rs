mod background;
mod commands;
mod face;
mod presets;
mod spec;

/// Whether the saved settings ask for a maximised window.
///
/// Read directly rather than through the IPC command because this runs before
/// the webview exists. A missing or unreadable config simply means the default
/// window size, which is why every failure returns false rather than stopping
/// startup.
fn wants_maximised() -> bool {
    platform::calibration::config_path()
        .and_then(|p| platform::presets::Config::load(&p).ok())
        .map(|cfg| cfg.sheet.start_maximized)
        .unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            use tauri::Manager;

            // Applied at startup, before the window is shown, so it does not
            // visibly resize itself after appearing.
            if wants_maximised() {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.maximize();
                }
            }

            // Build both ONNX sessions off the main thread while the user is
            // still choosing a photo. Together they cost roughly two seconds,
            // which was previously charged to the first detection and the
            // first background removal.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                face::preload(&handle.state::<face::DetectorState>());
                background::preload(&handle.state::<background::SegmentState>());
            });

            Ok(())
        })
        .manage(face::DetectorState::default())
        .manage(background::SegmentState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_printers,
            commands::printer_capabilities,
            commands::solve_layout,
            commands::get_calibration,
            commands::save_calibration,
            commands::print_calibration_square,
            commands::print_sheet,
            commands::solve_mixed_layout,
            commands::print_mixed_sheet,
            presets::get_sheet_settings,
            presets::save_sheet_settings,
            presets::list_presets,
            presets::save_preset,
            presets::load_preset,
            presets::delete_preset,
            face::detect_face,
            face::compute_crop,
            background::segment_background,
            background::add_mask_stroke,
            background::undo_mask_stroke,
            background::reset_mask_edits,
            background::set_mask_threshold,
            background::clear_mask,
            background::background_uniformity,
            spec::list_specs,
            spec::validate_photo,
            spec::analyse_image,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
