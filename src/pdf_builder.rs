// src/pdf_builder.rs
//! High-performance Searchable PDF Generator using pdf-writer
//! Generates valid Type 0 / CIDFont with /ToUnicode CMap, JPEG DCT compression, and logical text ordering.

use crate::arabic::ArabicProcessor;
use crate::layout::LayoutAnalyzer;
use crate::ocr_ffi::OneOcrEngine;
use crate::WorkerEvent;
use fltk::app::{self, Sender};
use pdfium_render::prelude::*;
use pdf_writer::{
    types::{CidFontType, FontFlags, SystemInfo, TextRenderingMode, UnicodeCmap},
    Content, Filter, Finish, Name, Pdf, Rect, Ref, Str,
};
use std::collections::HashSet;
use std::fs;
use std::io::Cursor;

const SYSTEM_INFO: SystemInfo = SystemInfo {
    registry: Str(b"Adobe"),
    ordering: Str(b"Identity"),
    supplement: 0,
};

/// Safely sends an event to the FLTK GUI thread and wakes up the event loop immediately
fn notify_gui(tx: &Sender<WorkerEvent>, event: WorkerEvent) {
    tx.send(event);
    app::awake();
}

struct ProcessedPage {
    width_pts: f32,
    height_pts: f32,
    width_px: u32,
    height_px: u32,
    jpeg_bytes: Vec<u8>,
    ordered_word_groups: Vec<Vec<crate::ocr_ffi::OcrWord>>,
}

pub fn process_pdf(
    input_path: &str,
    output_path: &str,
    is_visible: bool,
    tx: Sender<WorkerEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    notify_gui(&tx, WorkerEvent::Log("Initializing Microsoft OneOCR Engine...".into()));
    let ocr_engine = OneOcrEngine::new()?;

    notify_gui(&tx, WorkerEvent::Log("Loading PDFium rendering engine...".into()));
    
    // Check beside the built binary FIRST for pdfium.dll, then current dir, then ~/.config/oneocr
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|p| p.to_path_buf()));
    let pdfium_candidates = [
        exe_dir.as_ref().map(|d| d.join("pdfium.dll")),
        Some(std::path::PathBuf::from("pdfium.dll")),
        dirs::config_dir().map(|d| d.join("oneocr").join("pdfium.dll")),
    ];

    let mut pdfium_binding = None;
    for cand in pdfium_candidates.into_iter().flatten() {
        if cand.exists() {
            if let Ok(b) = Pdfium::bind_to_library(&cand) {
                notify_gui(&tx, WorkerEvent::Log(format!("Found pdfium.dll: {:?}", cand)));
                pdfium_binding = Some(b);
                break;
            }
        }
    }

    let pdfium = Pdfium::new(
        match pdfium_binding {
            Some(b) => b,
            None => Pdfium::bind_to_system_library()
                .map_err(|e| format!("Could not find pdfium.dll beside binary, in current folder, or system: {}", e))?,
        }
    );

    let doc = pdfium.load_pdf_from_file(input_path, None)?;
    let total_pages = doc.pages().len();
    notify_gui(&tx, WorkerEvent::Log(format!("Document opened successfully: {} pages total", total_pages)));
    notify_gui(&tx, WorkerEvent::Progress(5.0));

    let mut processed_pages = Vec::with_capacity(total_pages as usize);
    let mut unique_chars = HashSet::new();

    // 1. Process and rasterize each page, extract OCR text, and compress images
    for page_idx in 0..total_pages {
        let current_page = page_idx + 1;
        let base_pct = 5.0 + (page_idx as f32 / total_pages as f32) * 80.0;
        
        notify_gui(&tx, WorkerEvent::Log(format!("[Page {}/{}] Rendering at 300 DPI...", current_page, total_pages)));
        notify_gui(&tx, WorkerEvent::Progress(base_pct));

        let page = doc.pages().get(page_idx)?;
        let width_pts = page.width().value;
        let height_pts = page.height().value;

        // Render page at 300 DPI for pristine OCR accuracy
        let target_dpi = 300;
        let render_config = PdfRenderConfig::new()
            .set_target_width((width_pts * (target_dpi as f32 / 72.0)) as i32)
            .set_target_height((height_pts * (target_dpi as f32 / 72.0)) as i32);

        let bitmap = page.render_with_config(&render_config)?;
        let dynamic_img = bitmap.as_image();
        let rgba = dynamic_img.to_rgba8();
        let rgb = dynamic_img.to_rgb8();
        let (width_px, height_px) = rgba.dimensions();

        // High-efficiency JPEG compression: reduces page from 21 MB uncompressed to ~250 KB!
        let mut jpeg_bytes = Vec::new();
        let mut cursor = Cursor::new(&mut jpeg_bytes);
        let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut cursor, 85);
        encoder.encode(&rgb, width_px, height_px, image::ExtendedColorType::Rgb8)?;

        notify_gui(&tx, WorkerEvent::Log(format!("[Page {}/{}] Running Microsoft OneOCR recognition...", current_page, total_pages)));
        notify_gui(&tx, WorkerEvent::Progress(base_pct + (40.0 / total_pages as f32)));

        // Run OCR recognition
        let ocr_result = ocr_engine.recognize(&rgba)?;

        // Layout Analysis: detect multi-column gutters and headlines
        let gutters = LayoutAnalyzer::find_column_gutters(&ocr_result.lines, width_px as usize);
        let ordered_word_groups = LayoutAnalyzer::partition_into_reading_order(&ocr_result.lines, &gutters);

        // Track all unique characters for the /ToUnicode CMap
        for group in &ordered_word_groups {
            let mut sorted_words = group.clone();
            sorted_words.sort_by(|a, b| {
                let cx_a = (a.rect.x1 + a.rect.x2 + a.rect.x3 + a.rect.x4) / 4.0;
                let cx_b = (b.rect.x1 + b.rect.x2 + b.rect.x3 + b.rect.x4) / 4.0;
                cx_b.partial_cmp(&cx_a).unwrap_or(std::cmp::Ordering::Equal)
            });
            let mut logical_words = Vec::new();
            for word in sorted_words {
                let cleaned = ArabicProcessor::clean_and_normalize(&word.text);
                if !cleaned.trim().is_empty() {
                    logical_words.push(cleaned);
                }
            }
            if !logical_words.is_empty() {
                let line_str = logical_words.join(" ");
                let rev_str = ArabicProcessor::reverse_text_preserving_numbers_and_latin(&line_str);
                for c in rev_str.chars() {
                    unique_chars.insert(c);
                }
                for c in line_str.chars() {
                    unique_chars.insert(c);
                }
            }
        }

        processed_pages.push(ProcessedPage {
            width_pts,
            height_pts,
            width_px,
            height_px,
            jpeg_bytes,
            ordered_word_groups,
        });

        notify_gui(&tx, WorkerEvent::Progress(base_pct + (80.0 / total_pages as f32)));
    }

    notify_gui(&tx, WorkerEvent::Log("Building Searchable PDF with Type 0 /ToUnicode CMap...".into()));
    notify_gui(&tx, WorkerEvent::Progress(88.0));

    // 2. Build PDF Document
    let mut pdf = Pdf::new();
    let mut alloc = Ref::new(1);
    let catalog_ref = alloc.bump();
    let page_tree_ref = alloc.bump();
    let type0_font_ref = alloc.bump();
    let cid_ref = alloc.bump();
    let desc_ref = alloc.bump();
    let cmap_ref = alloc.bump();

    // 3. Construct /ToUnicode CMap (Solves Mojibake / Latin-1 encoding distortion)
    let mut unicode_cmap = UnicodeCmap::<u16>::new(Name(b"OneOcr-Identity-Unicode"), SYSTEM_INFO);

    // Map all characters encountered in document to their exact Unicode codepoints
    for &c in &unique_chars {
        unicode_cmap.pair(c as u16, c);
    }
    // Also include standard printable ASCII range
    for c in 32u8..=126u8 {
        unicode_cmap.pair(c as u16, c as char);
    }
    // Also include core Arabic blocks (U+0600 - U+06FF, Presentation Forms A & B)
    for cp in 0x0600u32..=0x06FFu32 {
        if let Some(c) = char::from_u32(cp) {
            unicode_cmap.pair(cp as u16, c);
        }
    }
    for cp in 0xFB50u32..=0xFDFFu32 {
        if let Some(c) = char::from_u32(cp) {
            unicode_cmap.pair(cp as u16, c);
        }
    }
    for cp in 0xFE70u32..=0xFEFFu32 {
        if let Some(c) = char::from_u32(cp) {
            unicode_cmap.pair(cp as u16, c);
        }
    }

    // Write CMap stream
    pdf.stream(cmap_ref, &unicode_cmap.finish()).finish();

    // 4. Write Font Descriptor & CIDFont
    let font_name = Name(b"Arial");

    let mut desc = pdf.font_descriptor(desc_ref);
    desc.name(font_name);
    desc.flags(FontFlags::SERIF);
    desc.bbox(Rect::new(-500.0, -300.0, 1500.0, 1000.0));
    desc.ascent(750.0);
    desc.descent(-250.0);
    desc.cap_height(700.0);
    desc.stem_v(80.0);
    desc.finish();

    let mut cid = pdf.cid_font(cid_ref);
    cid.subtype(CidFontType::Type2);
    cid.base_font(font_name);
    cid.system_info(SYSTEM_INFO);
    cid.font_descriptor(desc_ref);
    cid.default_width(500.0); // Standard proportional character width (1/1000 em)
    cid.finish();

    // 5. Write Type 0 Parent Font with Identity-H encoding
    let mut type0 = pdf.type0_font(type0_font_ref);
    type0.base_font(font_name);
    type0.encoding_predefined(Name(b"Identity-H"));
    type0.descendant_font(cid_ref);
    type0.to_unicode(cmap_ref);
    type0.finish();

    let mut page_refs = Vec::with_capacity(processed_pages.len());

    // 6. Generate Page Streams
    for page_data in &processed_pages {
        let width_pts = page_data.width_pts;
        let height_pts = page_data.height_pts;
        let width_px = page_data.width_px;
        let height_px = page_data.height_px;

        let scale_x = width_pts / width_px as f32;
        let scale_y = height_pts / height_px as f32;

        let mut content = Content::new();
        let image_ref = alloc.bump();

        // Embed scanned background page image (Option 1: Visible)
        if is_visible {
            let image_name = Name(b"Im1");
            content.save_state();
            content.transform([width_pts, 0.0, 0.0, height_pts, 0.0, 0.0]);
            content.x_object(image_name);
            content.restore_state();

            let mut img_writer = pdf.image_xobject(image_ref, &page_data.jpeg_bytes);
            img_writer.width(width_px as i32);
            img_writer.height(height_px as i32);
            img_writer.color_space().device_rgb();
            img_writer.bits_per_component(8);
            img_writer.filter(Filter::DctDecode); // JPEG DCT compression!
            img_writer.finish();
        }

        // Invisible Searchable Text Layer (Rendering Mode 3 Tr)
        content.begin_text();
        content.set_text_rendering_mode(TextRenderingMode::Invisible);
        content.set_font(Name(b"F1"), 12.0);

        for words in &page_data.ordered_word_groups {
            if words.is_empty() {
                continue;
            }

            // 1. Sort words in Right-to-Left Arabic order across the line using center X coordinate
            let mut sorted_words = words.clone();
            sorted_words.sort_by(|a, b| {
                let cx_a = (a.rect.x1 + a.rect.x2 + a.rect.x3 + a.rect.x4) / 4.0;
                let cx_b = (b.rect.x1 + b.rect.x2 + b.rect.x3 + b.rect.x4) / 4.0;
                cx_b.partial_cmp(&cx_a).unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut logical_words = Vec::new();
            for word in &sorted_words {
                let cleaned = ArabicProcessor::clean_and_normalize(&word.text);
                if !cleaned.trim().is_empty() {
                    logical_words.push(cleaned);
                }
            }

            if logical_words.is_empty() {
                continue;
            }

            // 2. Generate line text and reverse Arabic letters while preserving numbers and Latin words
            let logical_line = logical_words.join(" ");
            let reversed_line = ArabicProcessor::reverse_text_preserving_numbers_and_latin(&logical_line);
            if reversed_line.trim().is_empty() {
                continue;
            }

            // 3. Compute accurate full-line bounding box across the page
            let mut line_left_x = f32::INFINITY;
            let mut line_right_x = f32::NEG_INFINITY;
            let mut line_bottom_y = f32::NEG_INFINITY;
            let mut line_top_y = f32::INFINITY;
            let mut sum_height = 0.0;
            let mut count = 0;

            for word in &sorted_words {
                let x_min = word.rect.x1.min(word.rect.x4).min(word.rect.x2.min(word.rect.x3));
                let x_max = word.rect.x1.max(word.rect.x4).max(word.rect.x2.max(word.rect.x3));
                let y_min = word.rect.y1.min(word.rect.y2).min(word.rect.y3.min(word.rect.y4));
                let y_max = word.rect.y1.max(word.rect.y2).max(word.rect.y3.max(word.rect.y4));

                line_left_x = line_left_x.min(x_min);
                line_right_x = line_right_x.max(x_max);
                line_top_y = line_top_y.min(y_min);
                line_bottom_y = line_bottom_y.max(y_max);
                sum_height += (y_max - y_min) * scale_y;
                count += 1;
            }

            let avg_height_pt = if count > 0 { sum_height / count as f32 } else { 12.0 };
            let font_size = (avg_height_pt * 0.9).max(4.0);

            // PDF text coordinate origin is bottom-left of the line
            // Starting at line_left_x * scale_x ensures exact visual overlay alignment without rightward shift
            let x_pt = line_left_x * scale_x;
            let y_pt = (height_px as f32 - line_bottom_y) * scale_y;
            let physical_width_pt = (line_right_x - line_left_x) * scale_x;

            let char_count = reversed_line.chars().count().max(1);
            let rendered_width_pt = (char_count as f32) * font_size * 0.5;
            let h_scale = if rendered_width_pt > 0.0 && physical_width_pt > 0.0 {
                // Clamped to [10.0, 100.0] matching ReportLab reference
                // Prevents artificial rightward text expansion or overflow beyond line bounds
                ((physical_width_pt / rendered_width_pt) * 100.0).clamp(10.0, 100.0)
            } else {
                100.0
            };

            content.set_font(Name(b"F1"), font_size);
            content.set_horizontal_scaling(h_scale);
            content.set_text_matrix([1.0, 0.0, 0.0, 1.0, x_pt, y_pt]);

            // Emitting the reversed visual line ensures Adobe Acrobat and Chrome Bidi engines
            // extract the text in pristine, unreversed Arabic reading order!
            let mut cid_bytes = Vec::with_capacity(char_count * 2);
            for c in reversed_line.chars() {
                let code = c as u16;
                cid_bytes.push((code >> 8) as u8);
                cid_bytes.push((code & 0xFF) as u8);
            }
            content.show(Str(&cid_bytes));
            content.set_horizontal_scaling(100.0);
        }

        content.end_text();

        // Write page content stream
        let content_ref = alloc.bump();
        pdf.stream(content_ref, &content.finish());

        // Allocate Page object
        let page_ref = alloc.bump();
        page_refs.push(page_ref);

        let mut page_writer = pdf.page(page_ref);
        page_writer.parent(page_tree_ref);
        page_writer.media_box(Rect::new(0.0, 0.0, width_pts, height_pts));
        page_writer.contents(content_ref);

        let mut resources = page_writer.resources();
        resources.fonts().pair(Name(b"F1"), type0_font_ref);
        if is_visible {
            resources.x_objects().pair(Name(b"Im1"), image_ref);
        }
        resources.finish();
        page_writer.finish();
    }

    // Write Pages tree & Catalog
    let mut pages = pdf.pages(page_tree_ref);
    pages.count(page_refs.len() as i32);
    pages.kids(page_refs.iter().copied());
    pages.finish();

    let mut catalog = pdf.catalog(catalog_ref);
    catalog.pages(page_tree_ref);
    catalog.finish();

    // Write final PDF file
    fs::write(output_path, pdf.finish())?;
    notify_gui(&tx, WorkerEvent::Progress(100.0));
    notify_gui(&tx, WorkerEvent::Log(format!("File written successfully: {}", output_path)));

    Ok(())
}
