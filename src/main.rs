//! OneOCR Searchable PDF Desktop Tool
//! High-performance native Rust port using fltk-rs

#![windows_subsystem = "windows"] // Prevents CMD prompt on Windows launch

mod gui;
mod ocr_ffi;
mod layout;
mod arabic;
mod pdf_builder;

use fltk::app;

/// Inter-thread communication events from background processor to FLTK GUI
#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Log(String),
    Progress(f32),
    Complete(String),
    Error(String),
}

fn main() {
    let fltk_app = app::App::default().with_scheme(app::Scheme::Gleam);
    app::set_visible_focus(false); // Removes retro dotted focus outlines

    // Use Windows ClearType font (Segoe UI / Tahoma) for beautiful, smooth Arabic typography
    let font = fltk::enums::Font::by_name("Segoe UI");
    app::set_font(font);
    app::set_font_size(13);
    
    // Create crossbeam/mpsc channel for safe UI updates from worker threads
    let (tx, rx) = app::channel::<WorkerEvent>();

    // Build the native FLTK user interface
    let mut ui = gui::OcrAppUi::new(tx.clone());
    ui.show();

    // Event listener on main GUI loop - processes all queued events on each wakeup
    while fltk_app.wait() {
        while let Some(event) = rx.recv() {
            match event {
                WorkerEvent::Log(msg) => {
                    ui.append_log(&msg);
                }
                WorkerEvent::Progress(percentage) => {
                    ui.set_progress(percentage);
                }
                WorkerEvent::Complete(output_path) => {
                    ui.append_log(&format!("\n[SUCCESS] Document saved successfully at:\n{}", output_path));
                    ui.set_processing_state(false);
                    fltk::dialog::message_default(&format!("Saved successfully to:\n{}", output_path));
                }
                WorkerEvent::Error(err) => {
                    ui.append_log(&format!("\n[FATAL ERROR]: {}", err));
                    ui.set_processing_state(false);
                    fltk::dialog::alert_default(&format!("Error during processing:\n{}", err));
                }
            }
        }
        app::flush();
    }
}
