// src/arabic.rs
//! Arabic Text Processing & HarfBuzz Glyph Shaping

use rustybuzz::{Face, UnicodeBuffer};
use std::str::FromStr;
use unicode_bidi::BidiInfo;
use unicode_normalization::UnicodeNormalization;

pub struct ArabicProcessor;

impl ArabicProcessor {
    /// Converts Eastern Arabic numerals (٠-٩) to standard Western Arabic numerals (0-9)
    pub fn convert_digits_to_western(text: &str) -> String {
        text.chars()
            .map(|c| match c {
                '٠' => '0',
                '١' => '1',
                '٢' => '2',
                '٣' => '3',
                '٤' => '4',
                '٥' => '5',
                '٦' => '6',
                '٧' => '7',
                '٨' => '8',
                '٩' => '9',
                other => other,
            })
            .collect()
    }

    /// Normalizes Unicode (NFKC), normalizes Farsi Yeh to Arabic Yeh, and strips Harakat
    pub fn clean_and_normalize(text: &str) -> String {
        let text = Self::convert_digits_to_western(text);
        
        // NFKC normalization
        let normalized: String = text.nfkc().collect();
        let normalized = normalized.replace('ی', "ي"); // Persian Yeh -> Arabic Yeh

        // Strip Arabic diacritics / Tashkeel (ً to ٟ, ٰ)
        normalized
            .chars()
            .filter(|&c| {
                let u = c as u32;
                !(0x064B..=0x065F).contains(&u) && u != 0x0670 && !c.is_control()
            })
            .collect()
    }

    /// Full HarfBuzz glyph shaping using rustybuzz (pure-Rust HarfBuzz port)
    /// Produces correct connected glyphs, initial/medial/final forms, and ligatures
    pub fn shape_text(
        font_data: &[u8],
        text: &str,
    ) -> Result<Vec<(u32, i32, i32)>, String> {
        let cleaned = Self::clean_and_normalize(text);
        let face = Face::from_slice(font_data, 0)
            .ok_or_else(|| "Failed to parse TrueType font face".to_string())?;

        let mut buffer = UnicodeBuffer::new();
        buffer.push_str(&cleaned);
        buffer.set_direction(rustybuzz::Direction::RightToLeft);
        buffer.set_script(rustybuzz::script::ARABIC);
        if let Ok(lang) = rustybuzz::Language::from_str("ara") {
            buffer.set_language(lang);
        }

        let glyph_buffer = rustybuzz::shape(&face, &[], buffer);
        let infos = glyph_buffer.glyph_infos();
        let positions = glyph_buffer.glyph_positions();

        let mut shaped_glyphs = Vec::with_capacity(infos.len());
        for (info, pos) in infos.iter().zip(positions.iter()) {
            shaped_glyphs.push((info.glyph_id, pos.x_advance, pos.x_offset));
        }

        Ok(shaped_glyphs)
    }

    /// BiDi reordering helper for standard PDF Unicode text streams
    pub fn reorder_bidi(text: &str) -> String {
        let bidi_info = BidiInfo::new(text, Some(unicode_bidi::Level::rtl()));
        if bidi_info.paragraphs.is_empty() {
            return text.to_string();
        }

        let para = &bidi_info.paragraphs[0];
        let line = para.range.clone();
        bidi_info.reorder_line(para, line).to_string()
    }

    /// Reverses Arabic text for standard PDF LTR streams while preserving numbers (0-9) and Latin words.
    /// Does not manually invert symmetrical brackets because Adobe Acrobat, Chrome, and Poppler
    /// automatically handle standard UAX #9 BiDi mirroring during copy/paste and search extraction.
    pub fn reverse_text_preserving_numbers_and_latin(text: &str) -> String {
        let text = Self::convert_digits_to_western(text);
        
        let mut tokens: Vec<(String, bool)> = Vec::new();
        let mut cur = String::new();
        let mut cur_is_ltr = false;

        for c in text.chars() {
            let is_ltr = c.is_ascii_alphanumeric() && (c.is_ascii_digit() || c.is_ascii_alphabetic());
            if cur.is_empty() {
                cur.push(c);
                cur_is_ltr = is_ltr;
            } else if is_ltr == cur_is_ltr {
                cur.push(c);
            } else {
                tokens.push((cur, cur_is_ltr));
                cur = String::new();
                cur.push(c);
                cur_is_ltr = is_ltr;
            }
        }
        if !cur.is_empty() {
            tokens.push((cur, cur_is_ltr));
        }

        tokens.reverse();

        let mut result = String::with_capacity(text.len());
        for (token, is_ltr) in tokens {
            if is_ltr {
                result.push_str(&token);
            } else {
                let rev_chars: String = token.chars().rev().collect();
                result.push_str(&rev_chars);
            }
        }

        result
    }
}
