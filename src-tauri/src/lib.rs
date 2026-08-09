mod background;
mod commands;
mod face;
mod presets;
mod spec;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
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
