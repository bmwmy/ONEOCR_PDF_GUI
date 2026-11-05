// src/gui.rs
use fltk::{
    app,
    button::{Button, RadioRoundButton},
    dialog::{NativeFileChooser, NativeFileChooserType},
    frame::Frame,
    group::{Group, Pack, PackType},
    image::IcoImage,
    input::Input,
    misc::Progress,
    prelude::*,
    text::{TextBuffer, TextDisplay},
    window::DoubleWindow,
};
use std::path::Path;
use std::thread;
use crate::WorkerEvent;

// Embed icon bytes into the binary at compile time
const APP_ICON_BYTES: &[u8] = include_bytes!("../assets/app.ico");

pub struct OcrAppUi {
    pub window: DoubleWindow,
    pub input_path_input: Input,
    pub output_path_input: Input,
    pub radio_visible: RadioRoundButton,
    pub radio_invisible: RadioRoundButton,
    pub progress_bar: Progress,
    pub run_button: Button,
    pub log_display: TextDisplay,
    pub log_buffer: TextBuffer,
    sender: app::Sender<WorkerEvent>,
}

impl OcrAppUi {
    pub fn new(sender: app::Sender<WorkerEvent>) -> Self {
        let mut window = DoubleWindow::default()
            .with_size(680, 600)
            .with_label("OneOCR Searchable PDF Studio (Arabic)");
        window.set_color(fltk::enums::Color::from_rgb(241, 245, 249)); // Slate 100

        // ==========================================
        // SET ICON HERE (BEFORE WINDOW END & SHOW)
        // ==========================================
        if let Ok(icon) = IcoImage::from_data(APP_ICON_BYTES) {
            window.set_icon(Some(icon));
        }

        let mut main_pack = Pack::new(24, 18, 632, 564, "");
        main_pack.set_type(PackType::Vertical);
        main_pack.set_spacing(10);

        // Header Title Card
        let mut header_group = Group::new(0, 0, 632, 40, "");
        header_group.set_frame(fltk::enums::FrameType::RFlatBox);
        header_group.set_color(fltk::enums::Color::from_rgb(30, 41, 59)); // Slate 800
        
        let mut title_lbl = Frame::new(12, 6, 608, 28, "OneOCR Searchable PDF - تحويل ملفات PDF إلى نصوص قابلة للبحث");
        title_lbl.set_label_color(fltk::enums::Color::White);
        title_lbl.set_label_font(fltk::enums::Font::HelveticaBold);
        title_lbl.set_label_size(13);
        header_group.end();

        // 1. Input PDF Path Card
        let mut in_group = Group::new(0, 0, 632, 58, "");
        in_group.set_frame(fltk::enums::FrameType::RFlatBox);
        in_group.set_color(fltk::enums::Color::White);

        let mut lbl_in = Frame::new(12, 6, 400, 18, "ملف المدخل PDF (Input PDF):");
        lbl_in.set_align(fltk::enums::Align::Left | fltk::enums::Align::Inside);
        lbl_in.set_label_font(fltk::enums::Font::HelveticaBold);
        lbl_in.set_label_size(12);

        let mut input_path_input = Input::new(12, 26, 490, 26, "");
        input_path_input.set_frame(fltk::enums::FrameType::FlatBox);
        input_path_input.set_color(fltk::enums::Color::from_rgb(248, 250, 252));

        let mut btn_browse_in = Button::new(510, 26, 110, 26, "استعراض...");
        btn_browse_in.set_frame(fltk::enums::FrameType::RFlatBox);
        btn_browse_in.set_color(fltk::enums::Color::from_rgb(226, 232, 240));
        btn_browse_in.set_label_size(12);
        in_group.end();

        // 2. Output PDF Path Card
        let mut out_group = Group::new(0, 0, 632, 58, "");
        out_group.set_frame(fltk::enums::FrameType::RFlatBox);
        out_group.set_color(fltk::enums::Color::White);

        let mut lbl_out = Frame::new(12, 6, 400, 18, "مسار حفظ الملف الناتج (Output PDF):");
        lbl_out.set_align(fltk::enums::Align::Left | fltk::enums::Align::Inside);
        lbl_out.set_label_font(fltk::enums::Font::HelveticaBold);
        lbl_out.set_label_size(12);

        let mut output_path_input = Input::new(12, 26, 490, 26, "");
        output_path_input.set_frame(fltk::enums::FrameType::FlatBox);
        output_path_input.set_color(fltk::enums::Color::from_rgb(248, 250, 252));

        let mut btn_browse_out = Button::new(510, 26, 110, 26, "تحديد المسار...");
        btn_browse_out.set_frame(fltk::enums::FrameType::RFlatBox);
        btn_browse_out.set_color(fltk::enums::Color::from_rgb(226, 232, 240));
        btn_browse_out.set_label_size(12);
        out_group.end();

        // 3. Mode Selection Card
        let mut mode_group = Group::new(0, 0, 632, 64, "");
        mode_group.set_frame(fltk::enums::FrameType::RFlatBox);
        mode_group.set_color(fltk::enums::Color::White);

        let mut radio_visible = RadioRoundButton::new(16, 8, 600, 24, "الخيار 1: دمج طبقة نصوص شفافة مع صورة الصفحة الأصلية (Searchable PDF)");
        radio_visible.set_value(true);
        radio_visible.set_label_size(12);
        radio_visible.set_label_font(fltk::enums::Font::HelveticaBold);
        
        let mut radio_invisible = RadioRoundButton::new(16, 34, 600, 24, "الخيار 2: طبقة نصوص شفافة فقط بدون صور للدمج اللاحق (Text Layer Only)");
        radio_invisible.set_label_size(12);
        mode_group.end();

        // 4. Progress Bar
        let mut progress_bar = Progress::new(0, 0, 632, 22, "");
        progress_bar.set_minimum(0.0);
        progress_bar.set_maximum(100.0);
        progress_bar.set_value(0.0);
        progress_bar.set_frame(fltk::enums::FrameType::RFlatBox);
        progress_bar.set_color(fltk::enums::Color::from_rgb(226, 232, 240));
        progress_bar.set_selection_color(fltk::enums::Color::from_rgb(37, 99, 235)); // Blue 600

        // 5. Run Button
        let mut run_button = Button::new(0, 0, 632, 42, "▶ بدء المعالجة والتوليد | Start OCR & PDF Generation");
        run_button.set_frame(fltk::enums::FrameType::RFlatBox);
        run_button.set_color(fltk::enums::Color::from_rgb(37, 99, 235));
        run_button.set_label_color(fltk::enums::Color::White);
        run_button.set_label_font(fltk::enums::Font::HelveticaBold);
        run_button.set_label_size(14);

        // 6. Log Console
        let log_buffer = TextBuffer::default();
        let mut log_display = TextDisplay::new(0, 0, 632, 190, "");
        log_display.set_frame(fltk::enums::FrameType::FlatBox);
        log_display.set_buffer(log_buffer.clone());
        log_display.set_text_font(fltk::enums::Font::by_name("Consolas"));
        log_display.set_text_size(12);
        log_display.set_color(fltk::enums::Color::from_rgb(15, 23, 42)); // Slate 900
        log_display.set_text_color(fltk::enums::Color::from_rgb(226, 232, 240));

        main_pack.end();
        window.end();

        // Setup File Browser Callbacks
        let mut in_clone = input_path_input.clone();
        let mut out_clone = output_path_input.clone();
        let rad_vis = radio_visible.clone();
        btn_browse_in.set_callback(move |_| {
            let mut dialog = NativeFileChooser::new(NativeFileChooserType::BrowseFile);
            dialog.set_filter("PDF Documents\t*.pdf");
            dialog.show();
            let chosen = dialog.filename();
            if !chosen.as_os_str().is_empty() {
                in_clone.set_value(&chosen.to_string_lossy());
                
                // Automatically generate suggested output path
                let stem = chosen.file_stem().and_then(|s| s.to_str()).unwrap_or("document");
                let ext = chosen.extension().and_then(|e| e.to_str()).unwrap_or("pdf");
                let suffix = if rad_vis.value() { "_searchable" } else { "_text_layer" };
                let mut out_path = chosen.clone();
                out_path.set_file_name(format!("{}{}.{}", stem, suffix, ext));
                out_clone.set_value(&out_path.to_string_lossy());
            }
        });

        let mut out_clone2 = output_path_input.clone();
        btn_browse_out.set_callback(move |_| {
            let mut dialog = NativeFileChooser::new(NativeFileChooserType::BrowseSaveFile);
            dialog.set_filter("PDF Documents\t*.pdf");
            dialog.show();
            let chosen = dialog.filename();
            if !chosen.as_os_str().is_empty() {
                out_clone2.set_value(&chosen.to_string_lossy());
            }
        });

        // Trigger processing in background worker thread
        let in_ref = input_path_input.clone();
        let out_ref = output_path_input.clone();
        let rad_vis_ref = radio_visible.clone();
        let mut run_btn_ref = run_button.clone();
        let worker_tx = sender.clone();

        run_button.set_callback(move |_| {
            let in_path = in_ref.value().trim().to_string();
            let out_path = out_ref.value().trim().to_string();
            let is_visible = rad_vis_ref.value();

            if in_path.is_empty() || !Path::new(&in_path).exists() {
                fltk::dialog::alert_default("يرجى اختيار ملف PDF المدخل أولاً.\nPlease select input PDF file first.");
                return;
            }
            if out_path.is_empty() {
                fltk::dialog::alert_default("يرجى تحديد مسار حفظ الملف الناتج.\nPlease select output file path.");
                return;
            }

            run_btn_ref.deactivate();
            let tx = worker_tx.clone();

            thread::spawn(move || {
                let res = crate::pdf_builder::process_pdf(
                    &in_path,
                    &out_path,
                    is_visible,
                    tx.clone(),
                );
                match res {
                    Ok(_) => {
                        tx.send(WorkerEvent::Complete(out_path));
                        fltk::app::awake();
                    }
                    Err(e) => {
                        tx.send(WorkerEvent::Error(e.to_string()));
                        fltk::app::awake();
                    }
                }
            });
        });

        Self {
            window,
            input_path_input,
            output_path_input,
            radio_visible,
            radio_invisible,
            progress_bar,
            run_button,
            log_display,
            log_buffer,
            sender,
        }
    }

    pub fn show(&mut self) {
        self.window.show();
    }

    pub fn append_log(&mut self, text: &str) {
        self.log_buffer.append(text);
        self.log_buffer.append("\n");
        self.log_display.scroll(self.log_buffer.length(), 0);
        self.log_display.redraw();
        fltk::app::flush();
    }

    pub fn set_progress(&mut self, val: f32) {
        self.progress_bar.set_value(val as f64);
        self.progress_bar.redraw();
        fltk::app::redraw();
        fltk::app::flush();
    }

    pub fn set_processing_state(&mut self, is_running: bool) {
        if is_running {
            self.run_button.deactivate();
        } else {
            self.run_button.activate();
        }
        fltk::app::flush();
    }
}