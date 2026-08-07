mod commands;
mod face;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(face::DetectorState::default())
        .invoke_handler(tauri::generate_handler![
            commands::list_printers,
            commands::printer_capabilities,
            commands::solve_layout,
            commands::check_resolution,
            commands::get_calibration,
            commands::save_calibration,
            commands::print_calibration_square,
            commands::print_sheet,
            face::detect_face,
            face::compute_crop,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
