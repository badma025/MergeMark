//! Conservative, cell-first extraction of born-digital mark schemes.
//!
//! Geometry is collected once. Profiles name column roles; coordinates are
//! inferred separately for each table. A document is accepted only when all
//! answer pages are accounted for, avoiding partial answers at page/window seams.

use crate::pipeline::AnswerDraft;
use pdfium_render::prelude::*;
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::LazyLock;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

impl Bounds {
    fn union(self, other: Self) -> Self {
        Self {
            left: self.left.min(other.left),
            top: self.top.min(other.top),
            right: self.right.max(other.right),
            bottom: self.bottom.max(other.bottom),
        }
    }
    fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && self.right > other.left
            && self.top < other.bottom
            && self.bottom > other.top
    }
}

#[derive(Debug, Clone)]
pub struct GlyphEvidence {
    pub text: String,
    pub character_index: usize,
    /// Top-down page coordinates in PDF points (not image pixels).
    pub bounds: Bounds,
    pub font_size: f32,
    pub baseline: f32,
    /// Original PDF user-space transformation, retained for provenance.
    pub transformation: [f32; 6],
}

#[derive(Debug, Clone)]
pub struct PageEvidence {
    pub page_index: usize,
    pub width: f32,
    pub height: f32,
    pub glyphs: Vec<GlyphEvidence>,
    pub rules: Vec<Bounds>,
    /// Images, forms, and paths that cannot safely be treated as table rules.
    pub unresolved_graphics: Vec<Bounds>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    Aqa,
    Pearson,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnRole {
    Question,
    Answer,
    Guidance,
    Marks,
    AssessmentObjective,
}

#[derive(Debug, Clone)]
pub struct Column {
    pub role: ColumnRole,
    pub left: f32,
    pub right: f32,
}

#[derive(Debug, Clone)]
pub struct TableRegion {
    pub profile: Profile,
    pub header_bounds: Bounds,
    pub bounds: Bounds,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone)]
pub struct Cell {
    pub role: ColumnRole,
    pub bounds: Bounds,
    pub character_indexes: Vec<usize>,
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct PhysicalRow {
    pub page_index: usize,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuestionKey {
    pub whole: u32,
    pub parts: Vec<String>,
    pub printed: String,
}

#[derive(Debug, Clone)]
pub struct LogicalEntry {
    pub key: QuestionKey,
    /// Retains separate answer, guidance, mark and AO cells, including
    /// alternative methods and continuation rows. Never sums repeated codes.
    pub rows: Vec<PhysicalRow>,
}

#[derive(Debug)]
pub struct Extraction {
    pub entries: Vec<LogicalEntry>,
    pub page_count: usize,
}

fn cancelled(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Relaxed) {
        Err("Import cancelled by user".into())
    } else {
        Ok(())
    }
}

/// Native PDFium handles never leave this collection pass.
pub fn collect_page_evidence(
    path: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<PageEvidence>, String> {
    cancelled(cancel)?;
    let pdfium = crate::pdf_render::get_pdfium()?;
    let document = pdfium
        .load_pdf_from_file(path, None)
        .map_err(|e| e.to_string())?;
    if document.pages().len() as usize > crate::pdf_render::MAX_PAGES_PER_IMPORT {
        return Err("Mark scheme exceeds the page limit".into());
    }
    let mut result = Vec::new();
    for (page_index, page) in document.pages().iter().enumerate() {
        cancelled(cancel)?;
        // The first profiles support unrotated pages. Decline rather than
        // silently mixing rotated text coordinates with path coordinates.
        if page.rotation().map_err(|e| e.to_string())? != PdfPageRenderRotation::None {
            return Err(format!(
                "page {}: rotated layout unsupported",
                page_index + 1
            ));
        }
        let height = page.height().value;
        let bounds = |b: PdfRect| Bounds {
            left: b.left().value,
            right: b.right().value,
            top: height - b.top().value,
            bottom: height - b.bottom().value,
        };
        let mut evidence = PageEvidence {
            page_index,
            width: page.width().value,
            height,
            glyphs: Vec::new(),
            rules: Vec::new(),
            unresolved_graphics: Vec::new(),
        };
        let text = page.text().map_err(|e| e.to_string())?;
        for ch in text.chars().iter() {
            if ch.index() as usize % 256 == 0 {
                cancelled(cancel)?;
            }
            if ch.is_generated().unwrap_or(false) {
                continue;
            }
            let value = ch
                .unicode_string()
                .ok_or_else(|| format!("page {}: unmapped glyph {}", page_index + 1, ch.index()))?;
            if value.trim().is_empty() {
                continue;
            }
            let matrix = ch.matrix().map_err(|e| e.to_string())?;
            let (_, y) = ch.origin().map_err(|e| e.to_string())?;
            evidence.glyphs.push(GlyphEvidence {
                text: value,
                character_index: ch.index() as usize,
                bounds: bounds(ch.tight_bounds().map_err(|e| e.to_string())?),
                font_size: ch.scaled_font_size().value,
                baseline: height - y.value,
                transformation: [
                    matrix.a(),
                    matrix.b(),
                    matrix.c(),
                    matrix.d(),
                    matrix.e(),
                    matrix.f(),
                ],
            });
        }
        for object in page.objects().iter() {
            match &object {
                PdfPageObject::Text(_) => (),
                PdfPageObject::Path(path) => {
                    let quad = path.bounds().map_err(|e| e.to_string())?;
                    let b = Bounds {
                        left: quad.left().value,
                        right: quad.right().value,
                        top: height - quad.top().value,
                        bottom: height - quad.bottom().value,
                    };
                    let matrix = path.matrix().map_err(|e| e.to_string())?;
                    let mut start = None;
                    let mut previous = None;
                    let mut rules = Vec::new();
                    let mut unsupported = false;
                    for segment in path.segments().iter() {
                        let (x, y) = segment.point();
                        let (x, y) = matrix.apply_to_points(x, y);
                        let point = (x.value, height - y.value);
                        match segment.segment_type() {
                            PdfPathSegmentType::MoveTo => {
                                start = Some(point);
                                previous = Some(point);
                            }
                            PdfPathSegmentType::LineTo => {
                                if let Some((px, py)) = previous {
                                    if (px - point.0).abs() <= 1.0 || (py - point.1).abs() <= 1.0 {
                                        rules.push(Bounds {
                                            left: px.min(point.0),
                                            right: px.max(point.0),
                                            top: py.min(point.1),
                                            bottom: py.max(point.1),
                                        });
                                    } else {
                                        unsupported = true;
                                    }
                                }
                                previous = Some(point);
                            }
                            _ => unsupported = true,
                        }
                        if segment.is_close() {
                            if let (Some((px, py)), Some((sx, sy))) = (previous, start) {
                                if (px - sx).abs() <= 1.0 || (py - sy).abs() <= 1.0 {
                                    rules.push(Bounds {
                                        left: px.min(sx),
                                        right: px.max(sx),
                                        top: py.min(sy),
                                        bottom: py.max(sy),
                                    });
                                } else {
                                    unsupported = true;
                                }
                            }
                        }
                    }
                    if unsupported || rules.is_empty() {
                        evidence.unresolved_graphics.push(b);
                    } else {
                        evidence.rules.extend(rules);
                    }
                }
                _ => {
                    let quad = object.bounds().map_err(|e| e.to_string())?;
                    evidence.unresolved_graphics.push(Bounds {
                        left: quad.left().value,
                        right: quad.right().value,
                        top: height - quad.top().value,
                        bottom: height - quad.bottom().value,
                    });
                }
            }
        }
        result.push(evidence);
    }
    Ok(result)
}

#[derive(Clone)]
struct Word {
    text: String,
    bounds: Bounds,
    font: f32,
}

fn lines<'a>(glyphs: impl IntoIterator<Item = &'a GlyphEvidence>) -> Vec<Vec<&'a GlyphEvidence>> {
    let mut sorted: Vec<_> = glyphs.into_iter().collect();
    sorted.sort_by(|a, b| {
        a.baseline
            .total_cmp(&b.baseline)
            .then(a.bounds.left.total_cmp(&b.bounds.left))
    });
    let mut lines: Vec<Vec<&GlyphEvidence>> = Vec::new();
    for glyph in sorted {
        if let Some(line) = lines.last_mut() {
            if (line[0].baseline - glyph.baseline).abs()
                <= glyph.font_size.min(line[0].font_size) * 0.2
            {
                line.push(glyph);
                continue;
            }
        }
        lines.push(vec![glyph]);
    }
    for line in &mut lines {
        line.sort_by(|a, b| a.bounds.left.total_cmp(&b.bounds.left));
    }
    lines
}

fn words(line: &[&GlyphEvidence]) -> Vec<Word> {
    let mut result: Vec<Word> = Vec::new();
    for glyph in line {
        if let Some(word) = result.last_mut() {
            if glyph.bounds.left - word.bounds.right < glyph.font_size * 0.22 {
                word.text.push_str(&glyph.text);
                word.bounds = word.bounds.union(glyph.bounds);
                continue;
            }
        }
        result.push(Word {
            text: glyph.text.clone(),
            bounds: glyph.bounds,
            font: glyph.font_size,
        });
    }
    result
}

fn line_text(line: &[&GlyphEvidence]) -> String {
    words(line)
        .iter()
        .map(|w| w.text.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

fn header(line: &[&GlyphEvidence]) -> Option<(Profile, Vec<(ColumnRole, Bounds)>)> {
    let ws = words(line);
    let mut anchors: Vec<(ColumnRole, Bounds)> = Vec::new();
    let mut pearson = false;
    let mut additional = false;
    for word in ws {
        let text = word.text.to_ascii_lowercase();
        let role = match text.trim_matches(|c: char| !c.is_alphabetic()) {
            "question" => Some(ColumnRole::Question),
            "scheme" => {
                pearson = true;
                Some(ColumnRole::Answer)
            }
            "answer" | "answers" | "marking" => Some(ColumnRole::Answer),
            "additional" | "comments" => {
                additional = true;
                Some(ColumnRole::Guidance)
            }
            "guidance" if additional => Some(ColumnRole::Guidance),
            "guidance" => Some(ColumnRole::Answer),
            "mark" | "marks" => Some(ColumnRole::Marks),
            "ao" | "aos" => Some(ColumnRole::AssessmentObjective),
            _ => None,
        };
        if let Some(role) = role {
            if let Some((last_role, b)) = anchors.last_mut() {
                if *last_role == role {
                    *b = b.union(word.bounds);
                    continue;
                }
            }
            anchors.push((role, word.bounds));
        } else if let Some((_, b)) = anchors.last_mut() {
            if word.bounds.left - b.right < word.font * 2.0 {
                *b = b.union(word.bounds);
            }
        }
    }
    if anchors.first()?.0 != ColumnRole::Question
        || anchors.iter().filter(|a| a.0 == ColumnRole::Answer).count() != 1
        || anchors.iter().filter(|a| a.0 == ColumnRole::Marks).count() != 1
    {
        return None;
    }
    Some((
        if pearson {
            Profile::Pearson
        } else {
            Profile::Aqa
        },
        anchors,
    ))
}

/// Per-region gutters combine headers with continuous empty vertical strips
/// or explicit ruling lines. A glyph spanning a gutter will be refused later.
pub fn detect_table_regions(page: &PageEvidence) -> Result<Vec<TableRegion>, String> {
    let page_lines = lines(&page.glyphs);
    let headers: Vec<_> = page_lines
        .iter()
        .enumerate()
        .filter_map(|(i, line)| {
            header(line).map(|(profile, anchors)| {
                let mut bottom = line.iter().map(|g| g.bounds.bottom).fold(0.0, f32::max);
                // Recognize wrapped header tails without swallowing the first
                // answer line. These must contain only known header vocabulary.
                for tail in page_lines.iter().skip(i + 1).take(2) {
                    if tail[0].bounds.top > bottom + line[0].font_size * 1.3 {
                        break;
                    }
                    let ws = words(tail);
                    if ws.iter().all(|w| {
                        ["number", "guidance", "comments", "objectives"]
                            .contains(&w.text.to_ascii_lowercase().as_str())
                    }) {
                        bottom = tail.iter().map(|g| g.bounds.bottom).fold(bottom, f32::max);
                    } else {
                        break;
                    }
                }
                (
                    profile,
                    anchors,
                    line.iter()
                        .map(|g| g.bounds.top)
                        .fold(f32::INFINITY, f32::min),
                    bottom,
                )
            })
        })
        .collect();
    let mut regions = Vec::new();
    for (index, (profile, anchors, top, bottom)) in headers.iter().enumerate() {
        let end = headers
            .get(index + 1)
            .map(|h| h.2)
            .unwrap_or(page.height * 0.95);
        let body: Vec<_> = page
            .glyphs
            .iter()
            .filter(|g| g.bounds.top >= *bottom && g.bounds.bottom <= end)
            .collect();
        let mut edges = vec![0.0];
        for pair in anchors.windows(2) {
            let (lo, hi) = (pair[0].1.right + 1.0, pair[1].1.left - 1.0);
            if lo >= hi {
                return Err("overlapping column headers".into());
            }
            let ruled = page
                .rules
                .iter()
                .filter(|r| {
                    r.right - r.left <= 2.0
                        && r.left > lo
                        && r.right < hi
                        && r.top <= bottom + 3.0
                        && r.bottom >= end - 20.0
                })
                .map(|r| (r.left + r.right) / 2.0)
                .next();
            let edge = if let Some(x) = ruled {
                x
            } else {
                // Find the rightmost persistent whitespace gutter before the
                // next header. Its full strip must remain clear in the body.
                let mut x = hi;
                let mut found = None;
                while x >= lo {
                    if body
                        .iter()
                        .all(|g| g.bounds.right <= x - 1.0 || g.bounds.left >= x + 1.0)
                    {
                        found = Some(x);
                        break;
                    }
                    x -= 1.0;
                }
                found.ok_or("no unambiguous column gutter")?
            };
            edges.push(edge);
        }
        edges.push(page.width);
        regions.push(TableRegion {
            profile: *profile,
            header_bounds: Bounds {
                left: 0.0,
                top: *top,
                right: page.width,
                bottom: *bottom,
            },
            bounds: Bounds {
                left: 0.0,
                top: *bottom,
                right: page.width,
                bottom: end,
            },
            columns: anchors
                .iter()
                .enumerate()
                .map(|(i, (role, _))| Column {
                    role: *role,
                    left: edges[i],
                    right: edges[i + 1],
                })
                .collect(),
        });
    }
    Ok(regions)
}

static KEY: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r"(?i)^(?:Q)?0*([1-9][0-9]{0,2})((?:\.[0-9]+)|(?:\([a-zivx0-9]+\))*)$")
        .unwrap()
});
static PART: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[a-zivx0-9]+").unwrap());

fn question_key(text: &str, previous: Option<&QuestionKey>) -> Option<QuestionKey> {
    let compact = text.replace(' ', "");
    if compact.starts_with('(') && compact.ends_with(')') {
        let previous = previous?;
        let part = compact.trim_matches(['(', ')']);
        let parent = if part.chars().all(|c| "ivx".contains(c)) && previous.parts.len() >= 2 {
            previous.parts[..previous.parts.len() - 1]
                .iter()
                .map(|p| format!("({p})"))
                .collect::<String>()
        } else {
            String::new()
        };
        return question_key(&format!("{}{parent}{compact}", previous.whole), None);
    }
    let caps = KEY.captures(&compact)?;
    Some(QuestionKey {
        whole: caps[1].parse().ok()?,
        parts: PART
            .find_iter(&caps[2])
            .map(|m| m.as_str().to_string())
            .collect(),
        printed: compact,
    })
}

fn reconstruct(glyphs: &[&GlyphEvidence]) -> Result<String, String> {
    let ls = lines(glyphs.iter().copied());
    for line in &ls {
        for g in line {
            if !g.font_size.is_finite()
                || g.font_size <= 0.0
                || g.transformation.iter().any(|n| !n.is_finite())
                || g.transformation[1].abs() > 0.01
                || g.transformation[2].abs() > 0.01
                || g.transformation[0] <= 0.0
                || g.transformation[3] <= 0.0
                || g.text.contains('\u{fffd}')
                || g.text.chars().any(|c| c == '\0')
            {
                return Err("unreliable glyph geometry or encoding".into());
            }
            if g.text.chars().any(|c| "=^_∫√∑∏".contains(c)) {
                return Err("formula transcription requires source review".into());
            }
        }
    }
    for pair in ls.windows(2) {
        if pair[1][0].baseline - pair[0][0].baseline < pair[0][0].font_size * 0.8 {
            return Err("spatial math or overlapping baselines requires source review".into());
        }
    }
    Ok(ls
        .iter()
        .map(|l| line_text(l))
        .collect::<Vec<_>>()
        .join("\n"))
}

fn extract_rows(page: &PageEvidence, region: &TableRegion) -> Result<Vec<PhysicalRow>, String> {
    if page
        .unresolved_graphics
        .iter()
        .any(|b| b.intersects(region.bounds))
    {
        return Err("table contains unresolved graphical content".into());
    }
    let mut columns: Vec<Vec<&GlyphEvidence>> = vec![Vec::new(); region.columns.len()];
    for rule in &page.rules {
        if rule.top > region.bounds.top
            && rule.bottom < region.bounds.bottom
            && rule.right - rule.left > 3.0
            && rule.bottom - rule.top < 2.0
            && rule.right - rule.left < page.width * 0.65
        {
            // A short in-cell rule could be a fraction bar. Table rules must
            // align with inferred cell edges; never discard it as decoration.
            let at_edge = |x: f32| {
                region
                    .columns
                    .iter()
                    .any(|c| (c.left - x).abs() < 4.0 || (c.right - x).abs() < 4.0)
            };
            if !at_edge(rule.left) || !at_edge(rule.right) {
                return Err("unresolved in-cell rule (possible fraction bar)".into());
            }
        }
    }
    for glyph in &page.glyphs {
        if !glyph.bounds.intersects(region.bounds) {
            continue;
        }
        let owners: Vec<_> = region
            .columns
            .iter()
            .enumerate()
            .filter(|(_, col)| glyph.bounds.left >= col.left && glyph.bounds.right <= col.right)
            .collect();
        if owners.len() != 1 {
            return Err("glyph crosses a column gutter".into());
        }
        columns[owners[0].0].push(glyph);
    }
    // Physical bands come from question-cell lines. Wrapped prose is grouped
    // only after glyph ownership has been established independently per cell.
    let key_lines = lines(columns[0].iter().copied());
    let mut starts = vec![region.bounds.top];
    for (index, line) in key_lines.iter().enumerate() {
        let top = line
            .iter()
            .map(|g| g.bounds.top)
            .fold(f32::INFINITY, f32::min);
        if index == 0 {
            // An actual continuation above the first new key must remain a
            // separate physical row, rather than being assigned to that key.
            if columns
                .iter()
                .skip(1)
                .flatten()
                .any(|g| g.bounds.bottom < top - line[0].font_size)
            {
                starts.push(top - line[0].font_size * 0.4);
            }
        } else {
            let separator = page
                .rules
                .iter()
                .filter(|r| {
                    r.bottom - r.top <= 2.0
                        && r.top < top
                        && r.top > starts.last().copied().unwrap_or(0.0)
                        && r.right - r.left > page.width * 0.65
                })
                .map(|r| r.bottom)
                .max_by(f32::total_cmp);
            starts.push(separator.unwrap_or(top - line[0].font_size * 0.4));
        }
    }
    starts.sort_by(f32::total_cmp);
    starts.extend(
        page.rules
            .iter()
            .filter(|r| {
                r.bottom - r.top <= 2.0
                    && r.right - r.left > page.width * 0.65
                    && r.top > region.bounds.top
                    && r.bottom < region.bounds.bottom
            })
            .map(|r| r.bottom),
    );
    starts.sort_by(f32::total_cmp);
    starts.dedup_by(|a, b| (*a - *b).abs() < 2.0);
    starts.push(region.bounds.bottom);
    let mut rows = Vec::new();
    for band in starts.windows(2) {
        let mut cells = Vec::new();
        for (col, glyphs) in region.columns.iter().zip(&columns) {
            if glyphs
                .iter()
                .any(|g| g.bounds.top < band[0] && g.bounds.bottom > band[0])
            {
                return Err("glyph crosses a physical row separator".into());
            }
            let selected: Vec<_> = glyphs
                .iter()
                .copied()
                .filter(|g| g.bounds.top >= band[0] && g.bounds.top < band[1])
                .collect();
            cells.push(Cell {
                role: col.role,
                bounds: Bounds {
                    left: col.left,
                    right: col.right,
                    top: band[0],
                    bottom: band[1],
                },
                character_indexes: selected.iter().map(|g| g.character_index).collect(),
                text: reconstruct(&selected)?,
            });
        }
        if cells.iter().any(|c| !c.text.is_empty()) {
            rows.push(PhysicalRow {
                page_index: page.page_index,
                cells,
            });
        }
    }
    Ok(rows)
}

fn cell_text(row: &PhysicalRow, role: ColumnRole) -> &str {
    row.cells
        .iter()
        .find(|c| c.role == role)
        .map(|c| c.text.trim())
        .unwrap_or("")
}

/// Complete-document acceptance is intentional for the initial profiles:
/// unknown pages may continue a known question, so partial output cannot
/// safely suppress an overlapping LLM window yet.
pub fn extract(pages: &[PageEvidence], cancel: &AtomicBool) -> Result<Extraction, String> {
    let mut entries: Vec<LogicalEntry> = Vec::new();
    let mut profile = None;
    for page in pages {
        cancelled(cancel)?;
        if page.glyphs.iter().any(|g| {
            [
                g.bounds.left,
                g.bounds.right,
                g.bounds.top,
                g.bounds.bottom,
                g.baseline,
            ]
            .iter()
            .any(|v| !v.is_finite())
                || g.bounds.left > g.bounds.right
                || g.bounds.top > g.bounds.bottom
        }) {
            return Err("invalid glyph coordinates".into());
        }
        if !page.width.is_finite()
            || !page.height.is_finite()
            || page.width <= 0.0
            || page.height <= 0.0
        {
            return Err("invalid page dimensions".into());
        }
        let regions =
            detect_table_regions(page).map_err(|e| format!("page {}: {e}", page.page_index + 1))?;
        if regions.is_empty() {
            let prose = lines(&page.glyphs)
                .iter()
                .map(|l| line_text(l))
                .collect::<Vec<_>>()
                .join("\n")
                .to_lowercase();
            let suspected_answer = lines(&page.glyphs).iter().any(|line| {
                words(line)
                    .first()
                    .is_some_and(|w| question_key(&w.text, None).is_some())
            });
            let preamble = entries.is_empty()
                && !suspected_answer
                && page
                    .unresolved_graphics
                    .iter()
                    .all(|b| b.bottom < page.height * 0.15)
                && [
                    "mark scheme",
                    "marking instructions",
                    "general marking",
                    "copyright",
                ]
                .iter()
                .any(|s| prose.contains(s));
            if preamble || (page.glyphs.is_empty() && page.unresolved_graphics.is_empty()) {
                continue;
            }
            return Err(format!(
                "page {}: unrecognized table or continuation",
                page.page_index + 1
            ));
        }
        for line in lines(&page.glyphs) {
            let b = line.iter().map(|g| g.bounds).reduce(Bounds::union).unwrap();
            if b.top < page.height * 0.06 || b.bottom > page.height * 0.95 {
                continue;
            }
            if !regions
                .iter()
                .any(|r| b.intersects(r.bounds) || b.intersects(r.header_bounds))
            {
                return Err(format!(
                    "page {}: text outside recognized tables",
                    page.page_index + 1
                ));
            }
        }
        for region in regions {
            if profile.is_some_and(|p| p != region.profile) {
                return Err("mixed layout profiles".into());
            }
            profile = Some(region.profile);
            for row in extract_rows(page, &region)
                .map_err(|e| format!("page {}: {e}", page.page_index + 1))?
            {
                let id = cell_text(&row, ColumnRole::Question);
                if id.eq_ignore_ascii_case("total") {
                    // Preserve totals as source rows; do not count them as
                    // another question or as extra marking points.
                    entries
                        .last_mut()
                        .ok_or("total before first question")?
                        .rows
                        .push(row);
                    continue;
                }
                if id.is_empty() {
                    entries
                        .last_mut()
                        .ok_or("continuation without a question")?
                        .rows
                        .push(row);
                    continue;
                }
                let key = question_key(id, entries.last().map(|e| &e.key))
                    .ok_or_else(|| format!("invalid question key: {id}"))?;
                if let Some(previous) = entries.last_mut() {
                    if key.whole < previous.key.whole {
                        return Err("question sequence moves backwards".into());
                    }
                    if key.whole == previous.key.whole && key.parts == previous.key.parts {
                        previous.rows.push(row);
                        continue;
                    }
                }
                entries.push(LogicalEntry {
                    key,
                    rows: vec![row],
                });
            }
        }
    }
    if entries.is_empty() {
        return Err("no recognized marking entries".into());
    }
    for entry in &entries {
        if !entry
            .rows
            .iter()
            .any(|r| !cell_text(r, ColumnRole::Answer).is_empty())
            || !entry.rows.iter().any(|r| {
                cell_text(r, ColumnRole::Marks)
                    .chars()
                    .any(|c| c.is_ascii_digit())
            })
        {
            return Err(format!("{}: missing answer or marks", entry.key.printed));
        }
    }
    Ok(Extraction {
        entries,
        page_count: pages.len(),
    })
}

impl Extraction {
    pub fn into_drafts(self) -> Vec<AnswerDraft> {
        let mut questions: BTreeMap<u32, Vec<String>> = BTreeMap::new();
        for entry in self.entries {
            let output = questions.entry(entry.key.whole).or_default();
            output.push(format!("**{}**", entry.key.printed));
            for row in entry.rows {
                for cell in row.cells {
                    if cell.text.is_empty() || cell.role == ColumnRole::Question {
                        continue;
                    }
                    let label = match cell.role {
                        ColumnRole::Guidance => "Guidance: ",
                        ColumnRole::Marks => "Marks: ",
                        ColumnRole::AssessmentObjective => "AO: ",
                        _ => "",
                    };
                    output.push(format!("{label}{}", cell.text));
                }
            }
        }
        questions
            .into_iter()
            .map(|(question_number, parts)| AnswerDraft {
                question_number,
                markdown: parts.join("\n\n"),
            })
            .collect()
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn page(index: usize) -> PageEvidence {
        PageEvidence {
            page_index: index,
            width: 600.0,
            height: 800.0,
            glyphs: Vec::new(),
            rules: Vec::new(),
            unresolved_graphics: Vec::new(),
        }
    }

    fn put(page: &mut PageEvidence, mut x: f32, y: f32, text: &str) {
        for ch in text.chars() {
            if !ch.is_whitespace() {
                page.glyphs.push(GlyphEvidence {
                    text: ch.to_string(),
                    character_index: page.glyphs.len(),
                    bounds: Bounds {
                        left: x,
                        right: x + 4.5,
                        top: y,
                        bottom: y + 8.0,
                    },
                    font_size: 10.0,
                    baseline: y + 8.0,
                    transformation: [1.0, 0.0, 0.0, 1.0, x, 800.0 - y - 8.0],
                });
            }
            x += 5.0;
        }
    }

    fn aqa(page: &mut PageEvidence, y: f32) {
        for (x, text) in [
            (20.0, "Question"),
            (100.0, "Marking guidance"),
            (330.0, "Additional comments"),
            (470.0, "Mark"),
            (530.0, "AO"),
        ] {
            put(page, x, y, text);
        }
    }

    fn sample() -> PageEvidence {
        let mut p = page(0);
        aqa(&mut p, 40.0);
        for (x, text) in [
            (20.0, "01.1"),
            (100.0, "The force increases"),
            (330.0, "Accept greater force"),
            (470.0, "2"),
            (530.0, "AO1"),
        ] {
            put(&mut p, x, 80.0, text);
        }
        p
    }

    #[test]
    fn aqa_wrapped_cells_retain_ownership_and_provenance() {
        let mut p = sample();
        put(&mut p, 100.0, 94.0, "as extension increases.");
        put(&mut p, 330.0, 94.0, "Do not accept mass.");
        let result = extract(&[p], &AtomicBool::new(false)).unwrap();
        assert_eq!(result.entries[0].key.parts, vec!["1"]);
        let row = &result.entries[0].rows[0];
        assert_eq!(
            cell_text(row, ColumnRole::Answer),
            "The force increases\nas extension increases."
        );
        assert_eq!(
            cell_text(row, ColumnRole::Guidance),
            "Accept greater force\nDo not accept mass."
        );
        let ids: Vec<_> = row
            .cells
            .iter()
            .flat_map(|c| c.character_indexes.iter())
            .collect();
        assert_eq!(
            ids.len(),
            ids.iter().collect::<std::collections::BTreeSet<_>>().len()
        );
    }

    #[test]
    fn pearson_subparts_alternatives_and_continuations_are_not_flattened() {
        let mut p = page(0);
        for (x, t) in [
            (20.0, "Question Number"),
            (130.0, "Scheme"),
            (400.0, "Marks"),
            (480.0, "AO"),
        ] {
            put(&mut p, x, 40.0, t);
        }
        for (x, t) in [
            (20.0, "1(a)"),
            (130.0, "Correct method"),
            (400.0, "M1"),
            (480.0, "1.1a"),
        ] {
            put(&mut p, x, 80.0, t);
        }
        put(&mut p, 130.0, 96.0, "Alternative method");
        put(&mut p, 400.0, 96.0, "M1");
        for (x, t) in [(20.0, "(b)"), (130.0, "Correct answer"), (400.0, "A1")] {
            put(&mut p, x, 140.0, t);
        }
        let extraction = extract(&[p], &AtomicBool::new(false)).unwrap();
        assert_eq!(extraction.entries.len(), 2);
        assert_eq!(extraction.entries[1].key.printed, "1(b)");
        let drafts = extraction.into_drafts();
        assert_eq!(drafts.len(), 1);
        assert!(drafts[0].markdown.contains("Alternative method"));
        assert!(drafts[0].markdown.contains("M1\nM1"));
    }

    #[test]
    fn page_continuation_keeps_prior_key() {
        let first = sample();
        let mut second = page(1);
        aqa(&mut second, 40.0);
        put(&mut second, 100.0, 80.0, "Further permitted wording.");
        put(&mut second, 330.0, 80.0, "Ignore spelling.");
        let result = extract(&[first, second], &AtomicBool::new(false)).unwrap();
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0].rows.len(), 2);
        assert_eq!(result.entries[0].rows[1].page_index, 1);
    }

    #[test]
    fn gutters_are_inferred_for_each_region() {
        let mut p = sample();
        for (x, t) in [(30.0, "Question"), (180.0, "Answers"), (480.0, "Mark")] {
            put(&mut p, x, 180.0, t);
        }
        for (x, t) in [
            (30.0, "02.1"),
            (180.0, "Conservation of charge."),
            (480.0, "1"),
        ] {
            put(&mut p, x, 220.0, t);
        }
        let regions = detect_table_regions(&p).unwrap();
        assert_eq!(regions.len(), 2);
        assert_ne!(regions[0].columns[1].left, regions[1].columns[1].left);
        assert_eq!(
            extract(&[p], &AtomicBool::new(false))
                .unwrap()
                .entries
                .len(),
            2
        );
    }

    #[test]
    fn wrapped_header_tail_is_not_an_answer() {
        let mut p = sample();
        put(&mut p, 20.0, 53.0, "Number");
        let result = extract(&[p], &AtomicBool::new(false)).unwrap();
        assert_eq!(result.entries.len(), 1);
    }

    #[test]
    fn rejects_column_bleed_and_unmapped_glyphs() {
        let mut p = sample();
        // Fill the complete header gap, leaving no safe gutter.
        p.glyphs.push(GlyphEvidence {
            text: "?".into(),
            character_index: 999,
            bounds: Bounds {
                left: 160.0,
                right: 340.0,
                top: 100.0,
                bottom: 110.0,
            },
            font_size: 10.0,
            baseline: 110.0,
            transformation: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
        });
        assert!(extract(&[p], &AtomicBool::new(false)).is_err());
        let mut p = sample();
        p.glyphs.last_mut().unwrap().text = "\u{fffd}".into();
        assert!(extract(&[p], &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn refuses_unrecognized_continuation_and_missing_marks() {
        let mut next = page(1);
        put(&mut next, 100.0, 100.0, "Unrecognized continuation");
        assert!(extract(&[sample(), next], &AtomicBool::new(false)).is_err());
        let mut p = sample();
        p.glyphs
            .retain(|g| !(g.bounds.left >= 470.0 && g.bounds.left < 480.0 && g.bounds.top > 60.0));
        assert!(extract(&[p], &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn refuses_spatial_math_and_unresolved_graphics() {
        let mut p = sample();
        put(&mut p, 210.0, 75.0, "2");
        assert!(extract(&[p], &AtomicBool::new(false)).is_err());
        let mut p = sample();
        p.rules.push(Bounds {
            left: 100.0,
            right: 130.0,
            top: 95.0,
            bottom: 95.0,
        });
        assert!(extract(&[p], &AtomicBool::new(false)).is_err());
        let mut p = sample();
        p.unresolved_graphics.push(Bounds {
            left: 100.0,
            right: 200.0,
            top: 120.0,
            bottom: 180.0,
        });
        assert!(extract(&[p], &AtomicBool::new(false)).is_err());
    }

    #[test]
    fn cancellation_never_accepts_partial_output() {
        assert_eq!(
            extract(&[sample()], &AtomicBool::new(true)).unwrap_err(),
            "Import cancelled by user"
        );
    }

    #[test]
    fn ruled_rows_keep_vertically_centered_keys_with_their_answers() {
        let mut p = page(0);
        aqa(&mut p, 40.0);
        for y in [65.0, 115.0, 170.0] {
            p.rules.push(Bounds {
                left: 15.0,
                right: 580.0,
                top: y,
                bottom: y,
            });
        }
        put(&mut p, 20.0, 85.0, "01.1");
        put(&mut p, 100.0, 72.0, "First answer starts above key");
        put(&mut p, 100.0, 99.0, "and continues below it.");
        put(&mut p, 470.0, 85.0, "2");
        put(&mut p, 20.0, 140.0, "01.2");
        put(&mut p, 100.0, 123.0, "Second answer starts above key");
        put(&mut p, 470.0, 140.0, "1");
        let result = extract(&[p], &AtomicBool::new(false)).unwrap();
        assert_eq!(result.entries.len(), 2);
        assert!(
            cell_text(&result.entries[0].rows[0], ColumnRole::Answer).starts_with("First answer")
        );
        assert!(
            cell_text(&result.entries[1].rows[0], ColumnRole::Answer).starts_with("Second answer")
        );
    }

    #[test]
    fn unrecognized_answer_page_is_not_skipped_as_a_cover() {
        let mut p = page(0);
        put(&mut p, 20.0, 30.0, "Mark scheme");
        put(&mut p, 20.0, 90.0, "01.1 Unknown first answer");
        let mut second = sample();
        second.page_index = 1;
        assert!(extract(&[p, second], &AtomicBool::new(false)).is_err());
    }

    /// Authored in the test; no copyrighted fixture or network dependency.
    pub(crate) fn write_pdf_fixture(path: &Path, unsupported: bool) {
        let pdfium = crate::pdf_render::get_pdfium().unwrap();
        let mut document = pdfium.create_new_pdf().unwrap();
        let font = document.fonts_mut().helvetica();
        {
            let mut page = document
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::a4())
                .unwrap();
            for (x, y, text) in [
                (20.0, 750.0, "Question"),
                (110.0, 750.0, "Answers"),
                (340.0, 750.0, "Additional comments"),
                (480.0, 750.0, "Mark"),
                (20.0, 710.0, "01.1"),
                (110.0, 710.0, "The force increases."),
                (340.0, 710.0, "Accept greater force."),
                (480.0, 710.0, "2"),
            ] {
                page.objects_mut()
                    .create_text_object(
                        PdfPoints::new(x),
                        PdfPoints::new(y),
                        text,
                        font,
                        PdfPoints::new(10.0),
                    )
                    .unwrap();
            }
        }
        if unsupported {
            let mut page = document
                .pages_mut()
                .create_page_at_end(PdfPagePaperSize::a4())
                .unwrap();
            page.objects_mut()
                .create_text_object(
                    PdfPoints::new(100.0),
                    PdfPoints::new(700.0),
                    "Unrecognized continuation",
                    font,
                    PdfPoints::new(10.0),
                )
                .unwrap();
        }
        document.save_to_file(path).unwrap();
    }

    #[test]
    fn pdfium_collection_and_cell_extraction_round_trip() {
        let _lock = crate::pdf_render::pdfium_test_lock();
        let path = std::env::temp_dir().join(format!("mm_ms_{}.pdf", uuid::Uuid::new_v4()));
        write_pdf_fixture(&path, false);
        let pages = collect_page_evidence(&path, &AtomicBool::new(false)).unwrap();
        assert_eq!(pages.len(), 1);
        assert!(!pages[0].glyphs.is_empty());
        let drafts = extract(&pages, &AtomicBool::new(false))
            .unwrap()
            .into_drafts();
        assert_eq!(drafts[0].question_number, 1);
        assert!(drafts[0].markdown.contains("The force increases."));
        std::fs::remove_file(path).unwrap();
    }
}
