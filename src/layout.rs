// src/layout.rs
//! Document Layout Analysis & Multi-Column Detection

use crate::ocr_ffi::{OcrLine, OcrWord};

#[derive(Debug, Clone)]
pub struct ColumnGutter {
    pub start_x: usize,
    pub end_x: usize,
}

#[derive(Debug, Clone)]
pub struct LineGroup {
    pub y1: f32,
    pub y4: f32,
    pub words: Vec<OcrWord>,
}

pub struct LayoutAnalyzer;

impl LayoutAnalyzer {
    /// Detects vertical column gutters by analyzing 1D projection histogram.
    /// Excludes wide spanning lines (width >= 70% of page) from the histogram calculation
    /// so that title banners and bottom commentary do not obscure the vertical corridor between columns.
    pub fn find_column_gutters(
        lines: &[OcrLine],
        width_px: usize,
    ) -> Vec<ColumnGutter> {
        if width_px == 0 || lines.is_empty() {
            return vec![];
        }

        // Identify non-wide candidate lines (width < 70% of page width)
        let mut candidates: Vec<&OcrLine> = Vec::new();
        for line in lines {
            if line.words.is_empty() {
                continue;
            }
            let mut min_x = f32::INFINITY;
            let mut max_x = f32::NEG_INFINITY;
            for w in &line.words {
                min_x = min_x.min(w.rect.x1.min(w.rect.x4).min(w.rect.x2.min(w.rect.x3)));
                max_x = max_x.max(w.rect.x1.max(w.rect.x4).max(w.rect.x2.max(w.rect.x3)));
            }
            if (max_x - min_x) < (width_px as f32 * 0.70) {
                candidates.push(line);
            }
        }

        // Use candidates if we have at least 6 column-sized lines; otherwise evaluate all lines
        let search_lines: &[&OcrLine] = if candidates.len() >= 6 {
            &candidates
        } else {
            // Borrow all lines as reference slice
            &lines.iter().collect::<Vec<&OcrLine>>()
        };

        let mut occupancy = vec![0usize; width_px];
        for line in search_lines {
            for word in &line.words {
                let x1 = (word.rect.x1.min(word.rect.x4).max(0.0) as usize).min(width_px - 1);
                let x2 = (word.rect.x2.max(word.rect.x3).max(0.0) as usize).min(width_px - 1);
                if x1 < x2 {
                    for x in x1..=x2 {
                        occupancy[x] += 1;
                    }
                }
            }
        }

        let total_lines = search_lines.len();
        // 8% threshold matches Python reference (gui_app_a2a84422.py)
        let threshold = (total_lines as f32 * 0.08).max(1.0) as usize;
        let margin_threshold = (width_px as f32 * 0.05) as usize;
        let min_gutter_width = (width_px as f32 * 0.015).max(15.0) as usize;

        let mut raw_gaps: Vec<(usize, usize)> = Vec::new();
        let mut in_gap = false;
        let mut gap_start = 0;

        for (x, &count) in occupancy.iter().enumerate() {
            if count <= threshold {
                if !in_gap {
                    gap_start = x;
                    in_gap = true;
                }
            } else if in_gap {
                raw_gaps.push((gap_start, x.saturating_sub(1)));
                in_gap = false;
            }
        }
        if in_gap {
            raw_gaps.push((gap_start, width_px - 1));
        }

        // Filter valid gutters:
        // Must have sufficient width, sit inside page margins, and have substantial content on BOTH sides (>= 15%)
        let mut valid_gutters: Vec<ColumnGutter> = Vec::new();
        for (s, e) in raw_gaps {
            let width = e.saturating_sub(s) + 1;
            if width >= min_gutter_width && s > margin_threshold && e < (width_px.saturating_sub(margin_threshold)) {
                let mut words_left = 0usize;
                let mut words_right = 0usize;
                for line in lines {
                    for word in &line.words {
                        let wx = (word.rect.x1 + word.rect.x2) / 2.0;
                        if wx < s as f32 {
                            words_left += 1;
                        } else if wx > e as f32 {
                            words_right += 1;
                        }
                    }
                }
                let total = words_left + words_right;
                if total > 0 
                    && (words_left as f32 / total as f32) >= 0.15 
                    && (words_right as f32 / total as f32) >= 0.15 
                {
                    valid_gutters.push(ColumnGutter { start_x: s, end_x: e });
                }
            }
        }

        valid_gutters.sort_by_key(|g| g.start_x);
        valid_gutters
    }

    /// Partitions words into reading order columns and handles spanning headlines.
    /// Sorts lines top-to-bottom vertically, and flushes columns from Right to Left (RTL).
    pub fn partition_into_reading_order(
        lines: &[OcrLine],
        gutters: &[ColumnGutter],
    ) -> Vec<Vec<OcrWord>> {
        // If single column (no valid column gutters detected), preserve original OCR lines intact
        if gutters.is_empty() {
            return lines
                .iter()
                .filter(|l| !l.words.is_empty())
                .map(|l| l.words.clone())
                .collect();
        }

        let split_points: Vec<f32> = gutters
            .iter()
            .map(|g| (g.start_x + g.end_x) as f32 / 2.0)
            .collect();
        let num_columns = split_points.len() + 1;

        // Sort lines vertically from top to bottom
        let mut sorted_lines: Vec<OcrLine> = lines.to_vec();
        sorted_lines.sort_by(|a, b| {
            let cy_a = if a.words.is_empty() { 0.0 } else {
                a.words.iter().map(|w| (w.rect.y1 + w.rect.y4) / 2.0).sum::<f32>() / a.words.len() as f32
            };
            let cy_b = if b.words.is_empty() { 0.0 } else {
                b.words.iter().map(|w| (w.rect.y1 + w.rect.y4) / 2.0).sum::<f32>() / b.words.len() as f32
            };
            cy_a.partial_cmp(&cy_b).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut accumulated_by_column: Vec<Vec<OcrWord>> = vec![Vec::new(); num_columns];
        let mut ordered_groups: Vec<Vec<OcrWord>> = Vec::new();

        for line in sorted_lines {
            if line.words.is_empty() {
                continue;
            }

            // Check if this line spans across any gutter
            let mut line_min_x = f32::INFINITY;
            let mut line_max_x = f32::NEG_INFINITY;
            for w in &line.words {
                line_min_x = line_min_x.min(w.rect.x1.min(w.rect.x4));
                line_max_x = line_max_x.max(w.rect.x2.max(w.rect.x3));
            }

            let mut is_spanning = false;
            for g in gutters {
                let cross_bounds = line_min_x < g.start_x as f32 && line_max_x > g.end_x as f32;
                let has_left_word = line.words.iter().any(|w| w.rect.x2 < g.start_x as f32);
                let has_right_word = line.words.iter().any(|w| w.rect.x1 > g.end_x as f32);
                if cross_bounds || (has_left_word && has_right_word) {
                    is_spanning = true;
                    break;
                }
            }

            if is_spanning {
                // Flush accumulated columns in RTL reading order (right column first)
                for col_idx in (0..num_columns).rev() {
                    if !accumulated_by_column[col_idx].is_empty() {
                        let lines_in_col = Self::cluster_words_into_lines(&accumulated_by_column[col_idx]);
                        for line_group in lines_in_col {
                            ordered_groups.push(line_group.words);
                        }
                        accumulated_by_column[col_idx].clear();
                    }
                }
                ordered_groups.push(line.words.clone());
            } else {
                // Distribute words to their matching column bucket
                for word in &line.words {
                    let cx = (word.rect.x1 + word.rect.x2) / 2.0;
                    let mut col_idx = 0;
                    for &split in &split_points {
                        if cx > split {
                            col_idx += 1;
                        } else {
                            break;
                        }
                    }
                    accumulated_by_column[col_idx].push(word.clone());
                }
            }
        }

        // Flush remaining column contents in RTL reading order
        for col_idx in (0..num_columns).rev() {
            if !accumulated_by_column[col_idx].is_empty() {
                let lines_in_col = Self::cluster_words_into_lines(&accumulated_by_column[col_idx]);
                for line_group in lines_in_col {
                    ordered_groups.push(line_group.words);
                }
            }
        }

        ordered_groups
    }

    /// Clusters unstructured words in a column into coherent horizontal lines
    pub fn cluster_words_into_lines(words: &[OcrWord]) -> Vec<LineGroup> {
        let mut sorted = words.to_vec();
        sorted.sort_by(|a, b| {
            let cy_a = (a.rect.y1 + a.rect.y4) / 2.0;
            let cy_b = (b.rect.y1 + b.rect.y4) / 2.0;
            cy_a.partial_cmp(&cy_b).unwrap_or(std::cmp::Ordering::Equal)
        });

        let mut lines: Vec<LineGroup> = Vec::new();

        for word in sorted {
            let y1 = word.rect.y1.min(word.rect.y2);
            let y4 = word.rect.y3.max(word.rect.y4);
            let cy = (y1 + y4) / 2.0;
            let h = y4 - y1;
            let mut placed = false;

            for line in &mut lines {
                let line_h = line.y4 - line.y1;
                let line_cy = (line.y1 + line.y4) / 2.0;
                let allowed_diff = h.min(line_h) * 0.6;

                if (cy - line_cy).abs() < allowed_diff {
                    line.words.push(word.clone());
                    line.y1 = line.y1.min(y1);
                    line.y4 = line.y4.max(y4);
                    placed = true;
                    break;
                }
            }

            if !placed {
                lines.push(LineGroup {
                    y1,
                    y4,
                    words: vec![word],
                });
            }
        }

        lines.sort_by(|a, b| a.y1.partial_cmp(&b.y1).unwrap_or(std::cmp::Ordering::Equal));
        // Sort words within each line group in RTL reading order
        for line in &mut lines {
            line.words.sort_by(|a, b| {
                let cx_a = (a.rect.x1 + a.rect.x2) / 2.0;
                let cx_b = (b.rect.x1 + b.rect.x2) / 2.0;
                cx_b.partial_cmp(&cx_a).unwrap_or(std::cmp::Ordering::Equal)
            });
        }
        lines
    }
}
