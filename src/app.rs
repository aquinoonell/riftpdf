use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;

use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::PhysicalSize;
use winit::event::{ElementState, KeyEvent, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{CursorIcon, Window, WindowId};

#[cfg(target_os = "macos")]
use winit::platform::macos::{WindowAttributesExtMacOS, WindowExtMacOS};

use crate::history::{FileState, FitMode, History};
use crate::layout::{self, clamp_scroll, page_at_y, rescale_center, zoom_best_fit, zoom_fit_width};
use crate::pdf::{DocLink, PdfDoc, SearchHit};
use crate::timing::PhaseLog;

const STATUS_H: u32 = 40;
const PAGE_GAP: f32 = 12.0;
const SCROLL_STEP: f32 = 48.0;
const ZOOM_STEP: f32 = 1.1;
const BG: u32 = 0x000e_1018;
const STATUS_BG: u32 = 0x0014_161e;
const STATUS_FG: u32 = 0x00e8_eef8;
const STATUS_DIM: u32 = 0x0080_8898;
const HIGHLIGHT: u32 = 0x00d4_b878;
const HIGHLIGHT_CUR: u32 = 0x00c4_6c78;
const LINK_HL: u32 = 0x0080_c8e0;
const LINK_HOVER: u32 = 0x0098_d8f0;
const TOC_BG: u32 = 0x001e_2430;
const TOC_SEL: u32 = 0x0080_c8e0;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    Normal,
    SearchInput,
    Toc,
}

/// Continuous-scroll layout at the current render scale (device pixels).
struct Layout {
    scale_milli: u32,
    widths: Vec<f32>,
    heights: Vec<f32>,
    y_start: Vec<f32>,
    doc_height: f32,
    max_width: f32,
}

pub struct App {
    path: PathBuf,
    doc: PdfDoc,
    history: History,
    window: Option<Arc<Window>>,
    surface: Option<Surface<Arc<Window>, Arc<Window>>>,
    context: Option<Context<Arc<Window>>>,
    /// Page shown in status / history (topmost mostly-visible page).
    page: usize,
    zoom: f32,
    fit: FitMode,
    /// True after +/- so resize won't clobber the user's zoom.
    user_zoomed: bool,
    /// Horizontal pan in device pixels (sticky across pages).
    scroll_x: f32,
    /// Vertical offset into the continuous document (device pixels).
    scroll_y: f32,
    pending_g: bool,
    dirty: bool,
    win_w: u32,
    win_h: u32,
    scale_factor: f64,
    mode: Mode,
    search_buf: String,
    search_hits: Vec<SearchHit>,
    search_idx: usize,
    status_msg: String,
    toc_sel: usize,
    toc_filter: String,
    layout: Option<Layout>,
    /// `f` — show all link rectangles.
    show_links: bool,
    cursor: (f64, f64),
    hover_link: Option<DocLink>,
    timing: PhaseLog,
    /// `b` toggles the bottom status bar.
    show_status: bool,
    /// Latest modifier state (winit delivers this via `ModifiersChanged`).
    modifiers: ModifiersState,
}

impl App {
    pub fn new(path: PathBuf) -> Result<Self, Box<dyn std::error::Error>> {
        Self::new_with_history(path, History::load())
    }

    pub fn new_with_history(
        path: PathBuf,
        history: History,
    ) -> Result<Self, Box<dyn std::error::Error>> {
        let mut timing = PhaseLog::new();
        let doc = PdfDoc::open(&path)?;
        timing.mark("pdf_open");
        // Always start with fit-width for a clean open in a tiled window.
        // Remember page index only — zoom is recomputed for the real window size.
        let page = history
            .get(&path)
            .map(|st| st.page.min(doc.page_count().saturating_sub(1)))
            .unwrap_or(0);

        let _ = doc.page_size(page)?;
        timing.mark("page_size");

        Ok(Self {
            path,
            doc,
            history,
            window: None,
            surface: None,
            context: None,
            page,
            zoom: 1.0,
            fit: FitMode::FitWidth,
            user_zoomed: false,
            scroll_x: 0.0,
            scroll_y: 0.0,
            pending_g: false,
            dirty: true,
            win_w: 900,
            win_h: 1100,
            scale_factor: 1.0,
            mode: Mode::Normal,
            search_buf: String::new(),
            search_hits: Vec::new(),
            search_idx: 0,
            status_msg: String::new(),
            toc_sel: 0,
            toc_filter: String::new(),
            layout: None,
            show_links: false,
            cursor: (0.0, 0.0),
            hover_link: None,
            timing,
            show_status: true,
            modifiers: ModifiersState::empty(),
        })
    }

    fn persist(&mut self) {
        self.history.save(
            &self.path,
            FileState {
                page: self.page,
                zoom: self.zoom,
                fit: self.fit,
            },
        );
    }

    fn title(&self) -> String {
        let name = self
            .path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("riftpdf");
        format!(
            "{}  [{}/{}]  {:.0}%",
            name,
            self.page + 1,
            self.doc.page_count(),
            self.zoom * 100.0
        )
    }

    fn content_h(&self) -> f32 {
        if self.show_status {
            self.win_h.saturating_sub(STATUS_H) as f32
        } else {
            self.win_h as f32
        }
    }

    fn quit(&mut self, event_loop: &ActiveEventLoop) {
        self.persist();
        event_loop.exit();
    }

    fn half_page_px(&self) -> f32 {
        (self.content_h() * 0.5).max(SCROLL_STEP)
    }

    fn page_px(&self) -> f32 {
        self.content_h().max(SCROLL_STEP)
    }

    fn handle_ctrl_motion(&mut self, key: &str) -> bool {
        match key {
            "d" => {
                self.scroll_vertical(self.half_page_px());
                true
            }
            "u" => {
                self.scroll_vertical(-self.half_page_px());
                true
            }
            "f" => {
                self.scroll_vertical(self.page_px());
                true
            }
            "b" => {
                self.scroll_vertical(-self.page_px());
                true
            }
            "e" => {
                self.scroll_vertical(SCROLL_STEP);
                true
            }
            "y" => {
                self.scroll_vertical(-SCROLL_STEP);
                true
            }
            _ => false,
        }
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn render_scale(&self) -> f32 {
        self.zoom * self.scale_factor as f32
    }

    fn invalidate_layout(&mut self) {
        self.layout = None;
    }

    fn ensure_layout(&mut self) {
        let scale = self.render_scale();
        let milli = (scale * 1000.0).round() as u32;
        if let Some(layout) = &self.layout {
            if layout.scale_milli == milli {
                return;
            }
        }

        let n = self.doc.page_count();
        let mut widths = Vec::with_capacity(n);
        let mut heights = Vec::with_capacity(n);
        let mut y_start = Vec::with_capacity(n);
        let mut y = 0.0f32;
        let mut max_width = 0.0f32;

        for i in 0..n {
            let (pw, ph) = self.doc.page_size(i).unwrap_or((612.0, 792.0));
            let w = pw * scale;
            let h = ph * scale;
            y_start.push(y);
            widths.push(w);
            heights.push(h);
            max_width = max_width.max(w);
            y += h + PAGE_GAP;
        }

        self.layout = Some(Layout {
            scale_milli: milli,
            widths,
            heights,
            y_start,
            doc_height: (y - PAGE_GAP).max(0.0),
            max_width,
        });
    }

    /// After zoom change, keep the document point that was in the middle of
    /// the viewport still centered (zoom-to-center, not top-left).
    fn rescale_scroll(&mut self, old_scale: f32, new_scale: f32) {
        let view_w = self.win_w as f32;
        let view_h = self.content_h();
        let (sx, sy) = rescale_center(
            self.scroll_x,
            self.scroll_y,
            view_w,
            view_h,
            old_scale,
            new_scale,
        );
        self.scroll_x = sx;
        self.scroll_y = sy;
        self.invalidate_layout();
        self.clamp_scroll(true);
    }

    fn apply_fit(&mut self) {
        if self.win_w < 32 || self.win_h < 32 {
            return; // ignore bogus pre-tile sizes
        }
        let keep_page = self.page;
        let Ok((pw, ph)) = self.doc.page_size(keep_page) else {
            return;
        };
        // inner_size is physical pixels; page size is in PDF points.
        let view_w = self.win_w as f32;
        let view_h = self.content_h();
        if view_w < 1.0 || view_h < 1.0 || pw < 1.0 || ph < 1.0 {
            return;
        }
        // zoom is "logical"; render_scale = zoom * dpi
        let dpi = self.scale_factor as f32;
        match self.fit {
            FitMode::BestFit => {
                self.zoom = zoom_best_fit(view_w, view_h, pw, ph, dpi);
            }
            FitMode::FitWidth => {
                self.zoom = zoom_fit_width(view_w, pw, dpi);
            }
            FitMode::Manual => return,
        }
        // Rebuild layout at new zoom, then restore the saved page.
        // (Do NOT clamp/sync while scroll_y is still 0 — that was wiping history to page 1.)
        self.invalidate_layout();
        self.ensure_layout();
        self.page = keep_page;
        self.scroll_x = 0.0;
        self.scroll_y = self
            .layout
            .as_ref()
            .and_then(|l| l.y_start.get(keep_page).copied())
            .unwrap_or(0.0);
        self.clamp_scroll(false);
        self.dirty = true;
    }

    fn clamp_scroll(&mut self, sync_page: bool) {
        self.ensure_layout();
        let Some(layout) = &self.layout else {
            return;
        };
        let (sx, sy) = clamp_scroll(
            self.scroll_x,
            self.scroll_y,
            layout.doc_height,
            layout.max_width,
            self.content_h(),
            self.win_w as f32,
        );
        self.scroll_x = sx;
        self.scroll_y = sy;
        if sync_page {
            self.sync_page_from_scroll();
        }
    }

    fn sync_page_from_scroll(&mut self) {
        self.ensure_layout();
        let Some(layout) = &self.layout else {
            return;
        };
        let page = page_at_y(
            &layout.y_start,
            layout::focus_y(self.scroll_y, self.content_h()),
        );
        if page != self.page {
            self.page = page;
            self.persist();
        }
    }

    fn goto_page(&mut self, page: usize) {
        let max = self.doc.page_count().saturating_sub(1);
        let page = page.min(max);
        self.ensure_layout();
        if let Some(layout) = &self.layout {
            self.scroll_y = layout.y_start.get(page).copied().unwrap_or(0.0);
        }
        // Keep scroll_x + zoom — only move vertically to that page.
        self.page = page;
        self.clamp_scroll(false);
        self.dirty = true;
        self.persist();
    }

    fn scroll_vertical(&mut self, delta: f32) {
        if delta == 0.0 {
            return;
        }
        self.scroll_y += delta;
        self.clamp_scroll(true);
        self.dirty = true;
    }

    fn scroll_horizontal(&mut self, delta: f32) {
        self.scroll_x += delta;
        self.clamp_scroll(true);
        self.dirty = true;
    }

    fn zoom_by(&mut self, factor: f32) {
        let old = self.render_scale();
        self.fit = FitMode::Manual;
        self.user_zoomed = true;
        self.zoom = (self.zoom * factor).clamp(0.1, 8.0);
        let new = self.render_scale();
        self.rescale_scroll(old, new);
        self.dirty = true;
        self.persist();
    }

    fn run_search(&mut self) {
        let needle = self.search_buf.trim().to_string();
        if needle.is_empty() {
            self.search_hits.clear();
            self.status_msg = "empty search".into();
            self.mode = Mode::Normal;
            self.dirty = true;
            return;
        }
        match self.doc.search(&needle) {
            Ok(hits) => {
                let n = hits.len();
                self.search_hits = hits;
                self.search_idx = 0;
                if n == 0 {
                    self.status_msg = format!("no matches: {needle}");
                } else {
                    self.status_msg = format!("{n} matches: {needle}");
                    self.jump_to_hit(0);
                }
            }
            Err(e) => self.status_msg = format!("search error: {e}"),
        }
        self.mode = Mode::Normal;
        self.dirty = true;
    }

    fn jump_to_hit(&mut self, idx: usize) {
        if self.search_hits.is_empty() {
            return;
        }
        let idx = idx % self.search_hits.len();
        self.search_idx = idx;
        let hit = self.search_hits[idx].clone();
        self.ensure_layout();
        let scale = self.render_scale();
        let y0 = self
            .layout
            .as_ref()
            .and_then(|l| l.y_start.get(hit.page).copied())
            .unwrap_or(0.0);
        self.scroll_y = (y0 + hit.y0 * scale - self.content_h() * 0.35).max(0.0);
        self.page = hit.page;
        self.clamp_scroll(false);
        self.status_msg = format!(
            "match {}/{}: {}",
            idx + 1,
            self.search_hits.len(),
            self.search_buf
        );
        self.dirty = true;
        self.persist();
    }

    fn next_hit(&mut self, reverse: bool) {
        if self.search_hits.is_empty() {
            self.status_msg = "no search — press /".into();
            self.dirty = true;
            return;
        }
        let n = self.search_hits.len();
        let idx = if reverse {
            (self.search_idx + n - 1) % n
        } else {
            (self.search_idx + 1) % n
        };
        self.jump_to_hit(idx);
    }

    fn filtered_toc(&self) -> Vec<(usize, &crate::pdf::TocEntry)> {
        let q = self.toc_filter.to_lowercase();
        self.doc
            .toc()
            .iter()
            .enumerate()
            .filter(|(_, e)| q.is_empty() || e.title.to_lowercase().contains(&q))
            .collect()
    }

    fn page_draw_x(&self, page_w_px: u32) -> i32 {
        let w = self.win_w;
        if page_w_px < w {
            ((w - page_w_px) / 2) as i32
        } else {
            (-self.scroll_x).round() as i32
        }
    }

    fn update_hover(&mut self) {
        let link = self.hit_test_link(self.cursor.0, self.cursor.1);
        let changed = match (&self.hover_link, &link) {
            (None, None) => false,
            (Some(a), Some(b)) => a.uri != b.uri || a.page != b.page,
            _ => true,
        };
        if changed {
            self.hover_link = link;
            if let Some(w) = &self.window {
                w.set_cursor(if self.hover_link.is_some() {
                    CursorIcon::Pointer
                } else {
                    CursorIcon::Default
                });
            }
            self.dirty = true;
            self.request_redraw();
        }
    }

    fn hit_test_link(&mut self, px: f64, py: f64) -> Option<DocLink> {
        if self.mode != Mode::Normal || py >= self.content_h() as f64 {
            return None;
        }
        self.ensure_layout();
        let scale = self.render_scale();
        let layout = self.layout.as_ref()?;
        let doc_y = self.scroll_y + py as f32;

        let mut page = None;
        for (i, &y0) in layout.y_start.iter().enumerate() {
            let y1 = y0 + layout.heights[i];
            if doc_y >= y0 && doc_y <= y1 {
                page = Some((i, y0, layout.widths[i]));
                break;
            }
        }
        let (page, y0, page_w) = page?;
        let draw_x = self.page_draw_x(page_w.round() as u32) as f32;
        let draw_y = y0 - self.scroll_y;
        let local_x = (px as f32 - draw_x) / scale;
        let local_y = (py as f32 - draw_y) / scale;

        let links = self.doc.links_on_page(page).ok()?;
        links
            .iter()
            .find(|l| local_x >= l.x0 && local_x <= l.x1 && local_y >= l.y0 && local_y <= l.y1)
            .cloned()
    }

    fn activate_link(&mut self, link: &DocLink) {
        if let Some(dest) = link.dest_page {
            self.goto_page(dest);
            self.status_msg = format!("→ p{}", dest + 1);
            return;
        }
        let uri = link.uri.trim();
        if uri.starts_with("http://") || uri.starts_with("https://") || uri.starts_with("mailto:") {
            let _ = std::process::Command::new("open").arg(uri).spawn();
            self.status_msg = truncate(uri, 48);
            self.dirty = true;
        } else if !uri.is_empty() {
            // Relative / other schemes — try open anyway
            let _ = std::process::Command::new("open").arg(uri).spawn();
            self.status_msg = truncate(uri, 48);
            self.dirty = true;
        }
    }

    fn redraw(&mut self) {
        let Some(window) = self.window.clone() else {
            return;
        };
        if self.surface.is_none() {
            return;
        }

        let w = self.win_w.max(1);
        let h = self.win_h.max(1);
        let (Some(nw), Some(nh)) = (NonZeroU32::new(w), NonZeroU32::new(h)) else {
            return;
        };

        self.ensure_layout();
        let scale = self.render_scale();
        let content_h = self.content_h();
        let scroll_x = self.scroll_x;
        let scroll_y = self.scroll_y;
        let cur_hit = self.search_idx;

        // Which pages intersect the viewport?
        let mut visible: Vec<(usize, f32, f32, f32)> = Vec::new(); // page, y_start, width, height
        if let Some(layout) = &self.layout {
            let view_bot = scroll_y + content_h;
            for i in 0..layout.y_start.len() {
                let y0 = layout.y_start[i];
                let y1 = y0 + layout.heights[i];
                if y1 >= scroll_y && y0 <= view_bot {
                    visible.push((i, y0, layout.widths[i], layout.heights[i]));
                }
            }
        }

        // Render bitmaps for visible pages
        let mut pages_drawn: Vec<(usize, f32, crate::pdf::PageBitmap)> = Vec::new();
        for (page, y0, _, _) in &visible {
            match self.doc.render(*page, scale) {
                Ok(bmp) => pages_drawn.push((*page, *y0, bmp)),
                Err(e) => eprintln!("render {page}: {e}"),
            }
        }

        let hits = self.search_hits.clone();
        let mode = self.mode;
        let show_links = self.show_links;
        let hover = self.hover_link.clone();
        let (status_primary, status_hint) = self.status_line();
        let show_status = self.show_status;
        let toc_sel = self.toc_sel;
        let toc_rows: Vec<(usize, String, usize, usize)> = if mode == Mode::Toc {
            self.filtered_toc()
                .iter()
                .map(|(i, e)| (*i, e.title.clone(), e.page, e.depth))
                .collect()
        } else {
            Vec::new()
        };

        // Prefetch links for visible pages (for highlight + hit-test)
        let mut page_links: Vec<(usize, Vec<DocLink>)> = Vec::new();
        for (page, _, _, _) in &visible {
            if let Ok(links) = self.doc.links_on_page(*page) {
                page_links.push((*page, links.to_vec()));
            }
        }

        let surface = self.surface.as_mut().unwrap();
        if surface.resize(nw, nh).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };
        buffer.fill(BG);

        let content_hu = content_h as u32;

        for (page, y0, bmp) in &pages_drawn {
            // Center narrow pages; otherwise pan with scroll_x
            let draw_x = if bmp.width < w {
                ((w - bmp.width) / 2) as i32
            } else {
                (-scroll_x).round() as i32
            };
            let draw_y = (y0 - scroll_y).round() as i32;

            blit(
                &mut buffer,
                w,
                content_hu,
                &bmp.pixels,
                bmp.width,
                bmp.height,
                draw_x,
                draw_y,
            );

            // Link highlights
            if let Some((_, links)) = page_links.iter().find(|(p, _)| p == page) {
                for link in links {
                    let hovered = hover
                        .as_ref()
                        .is_some_and(|h| h.page == link.page && h.uri == link.uri);
                    if show_links || hovered {
                        let color = if hovered { LINK_HOVER } else { LINK_HL };
                        let alpha = if hovered { 0.35 } else { 0.22 };
                        let x0 = draw_x + (link.x0 * scale).round() as i32;
                        let hy0 = draw_y + (link.y0 * scale).round() as i32;
                        let x1 = draw_x + (link.x1 * scale).round() as i32;
                        let hy1 = draw_y + (link.y1 * scale).round() as i32;
                        fill_rect_blend(
                            &mut buffer,
                            w,
                            content_hu,
                            x0,
                            hy0,
                            x1.max(x0 + 2),
                            hy1.max(hy0 + 2),
                            color,
                            alpha,
                        );
                    }
                }
            }

            // Search highlights (on top of links)
            for (i, hit) in hits.iter().enumerate() {
                if hit.page != *page {
                    continue;
                }
                let color = if i == cur_hit {
                    HIGHLIGHT_CUR
                } else {
                    HIGHLIGHT
                };
                let x0 = draw_x + (hit.x0 * scale).round() as i32;
                let hy0 = draw_y + (hit.y0 * scale).round() as i32;
                let x1 = draw_x + (hit.x1 * scale).round() as i32;
                let hy1 = draw_y + (hit.y1 * scale).round() as i32;
                fill_rect_blend(
                    &mut buffer,
                    w,
                    content_hu,
                    x0,
                    hy0,
                    x1.max(x0 + 2),
                    hy1.max(hy0 + 2),
                    color,
                    0.45,
                );
            }
        }

        if mode == Mode::Toc {
            draw_toc_panel(&mut buffer, w, content_hu, &toc_rows, toc_sel);
        }

        if show_status {
            for y in content_hu..h {
                for x in 0..w {
                    buffer[(y * w + x) as usize] = STATUS_BG;
                }
            }
            // 2× bitmap font for readability
            draw_text_5x7_scaled(
                &mut buffer,
                w,
                h,
                8,
                content_hu + 4,
                &status_primary,
                STATUS_FG,
                2,
            );
            draw_text_5x7_scaled(
                &mut buffer,
                w,
                h,
                8,
                content_hu + 22,
                &status_hint,
                STATUS_DIM,
                1,
            );
        }

        let _ = buffer.present();
        window.set_title(&self.title());
        self.dirty = false;
    }

    fn status_line(&self) -> (String, String) {
        // (primary, hint) — drawn as two rows when the bar is visible
        match self.mode {
            Mode::SearchInput => (
                format!("/{}_", self.search_buf),
                "Enter search · Esc cancel · n/N next".into(),
            ),
            Mode::Toc => {
                let n = self.filtered_toc().len();
                if self.toc_filter.is_empty() {
                    (
                        format!("TOC — {n} chapters"),
                        "j/k move · Enter jump · type to filter · Esc".into(),
                    )
                } else {
                    (
                        format!("TOC filter: {}_ ({n})", self.toc_filter),
                        "Enter jump · Esc clear".into(),
                    )
                }
            }
            Mode::Normal => {
                let name = self
                    .path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("pdf");
                let primary = if !self.status_msg.is_empty() {
                    truncate(&self.status_msg, 52)
                } else {
                    format!(
                        "{}  {}/{}  {:.0}%",
                        truncate(name, 24),
                        self.page + 1,
                        self.doc.page_count(),
                        self.zoom * 100.0
                    )
                };
                let hint = "hjkl · C-d/u · / search · n/N · q".into();
                (primary, hint)
            }
        }
    }

    fn handle_key(&mut self, event: KeyEvent, event_loop: &ActiveEventLoop) {
        if event.state != ElementState::Pressed {
            return;
        }

        match self.mode {
            Mode::SearchInput => {
                self.handle_search_input(&event);
                self.request_redraw();
                return;
            }
            Mode::Toc => {
                self.handle_toc_input(&event);
                self.request_redraw();
                return;
            }
            Mode::Normal => {}
        }

        if self.modifiers.control_key() {
            if let Key::Character(c) = &event.logical_key {
                if self.handle_ctrl_motion(c.as_str()) {
                    self.request_redraw();
                    return;
                }
            }
        }

        let mut consumed_pending = false;
        match &event.logical_key {
            Key::Character(c) => match c.as_str() {
                "q" => {
                    self.quit(event_loop);
                    return;
                }
                "j" => self.scroll_vertical(SCROLL_STEP),
                "k" => self.scroll_vertical(-SCROLL_STEP),
                "h" => self.scroll_horizontal(-SCROLL_STEP),
                "l" => self.scroll_horizontal(SCROLL_STEP),
                "J" | " " => self.goto_page(self.page + 1),
                "K" => self.goto_page(self.page.saturating_sub(1)),
                "+" | "=" => self.zoom_by(ZOOM_STEP),
                "-" => self.zoom_by(1.0 / ZOOM_STEP),
                "a" => {
                    self.user_zoomed = false;
                    self.fit = FitMode::BestFit;
                    self.apply_fit();
                    self.persist();
                }
                "s" => {
                    self.user_zoomed = false;
                    self.fit = FitMode::FitWidth;
                    self.apply_fit();
                    self.persist();
                }
                "/" => {
                    self.mode = Mode::SearchInput;
                    self.search_buf.clear();
                    self.status_msg.clear();
                    self.dirty = true;
                }
                "n" => self.next_hit(false),
                "N" => self.next_hit(true),
                "b" => {
                    self.show_status = !self.show_status;
                    self.clamp_scroll(false);
                    if self.show_status {
                        self.status_msg = "status bar on".into();
                    }
                    self.dirty = true;
                }
                "f" => {
                    self.show_links = !self.show_links;
                    self.status_msg = if self.show_links {
                        "links on — click to follow".into()
                    } else {
                        "links hidden (hover still works)".into()
                    };
                    self.dirty = true;
                }
                "t" | "o" => {
                    if self.doc.toc().is_empty() {
                        self.status_msg = "no table of contents".into();
                    } else {
                        self.mode = Mode::Toc;
                        self.toc_sel = 0;
                        self.toc_filter.clear();
                    }
                    self.dirty = true;
                }
                "g" => {
                    if self.pending_g {
                        self.goto_page(0);
                        self.pending_g = false;
                        consumed_pending = true;
                    } else {
                        self.pending_g = true;
                        return;
                    }
                }
                "G" => self.goto_page(usize::MAX),
                _ => {}
            },
            Key::Named(NamedKey::Escape) => {
                if !self.search_hits.is_empty() || !self.status_msg.is_empty() {
                    self.search_hits.clear();
                    self.status_msg.clear();
                    self.dirty = true;
                } else {
                    self.quit(event_loop);
                    return;
                }
            }
            Key::Named(NamedKey::PageDown) => self.goto_page(self.page + 1),
            Key::Named(NamedKey::PageUp) => self.goto_page(self.page.saturating_sub(1)),
            Key::Named(NamedKey::Home) => self.goto_page(0),
            Key::Named(NamedKey::End) => self.goto_page(usize::MAX),
            _ => {}
        }

        if !consumed_pending {
            self.pending_g = false;
        }
        if self.dirty {
            self.request_redraw();
        }
    }

    fn handle_search_input(&mut self, event: &KeyEvent) {
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Normal;
                self.search_buf.clear();
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) => self.run_search(),
            Key::Named(NamedKey::Backspace) => {
                self.search_buf.pop();
                self.dirty = true;
            }
            Key::Character(c) => {
                for ch in c.chars() {
                    if !ch.is_control() {
                        self.search_buf.push(ch);
                    }
                }
                self.dirty = true;
            }
            _ => {}
        }
    }

    fn handle_toc_input(&mut self, event: &KeyEvent) {
        let entries = self.filtered_toc();
        let len = entries.len();
        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.mode = Mode::Normal;
                self.toc_filter.clear();
                self.dirty = true;
            }
            Key::Named(NamedKey::Enter) => {
                if len > 0 {
                    let sel = self.toc_sel.min(len - 1);
                    let page = entries[sel].1.page;
                    let title = entries[sel].1.title.clone();
                    self.mode = Mode::Normal;
                    self.toc_filter.clear();
                    self.goto_page(page);
                    self.status_msg = format!("chapter: {title}");
                }
            }
            Key::Named(NamedKey::Backspace) => {
                self.toc_filter.pop();
                self.toc_sel = 0;
                self.dirty = true;
            }
            Key::Character(c) => {
                let s = c.as_str();
                match s {
                    "j" if self.toc_filter.is_empty() => {
                        if len > 0 {
                            self.toc_sel = (self.toc_sel + 1).min(len - 1);
                            self.dirty = true;
                        }
                    }
                    "k" if self.toc_filter.is_empty() => {
                        self.toc_sel = self.toc_sel.saturating_sub(1);
                        self.dirty = true;
                    }
                    _ => {
                        for ch in c.chars() {
                            if !ch.is_control() {
                                self.toc_filter.push(ch);
                            }
                        }
                        self.toc_sel = 0;
                        self.dirty = true;
                    }
                }
            }
            Key::Named(NamedKey::ArrowDown) => {
                if len > 0 {
                    self.toc_sel = (self.toc_sel + 1).min(len - 1);
                    self.dirty = true;
                }
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.toc_sel = self.toc_sel.saturating_sub(1);
                self.dirty = true;
            }
            _ => {}
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        // Seed size from the monitor so the first frame is closer to Rift's tile
        // (avoids a tiny 450×550 flash before the WM places the window).
        if let Some(monitor) = event_loop.primary_monitor() {
            let size = monitor.size();
            if size.width >= 640 && size.height >= 480 {
                // Leave a little slack for SketchyBar / outer gaps; Rift will still resize.
                self.win_w = size.width.saturating_sub(48);
                self.win_h = size.height.saturating_sub(64);
            }
        }

        let mut attrs = Window::default_attributes()
            .with_title(self.title())
            .with_inner_size(PhysicalSize::new(self.win_w, self.win_h));

        #[cfg(target_os = "macos")]
        {
            attrs = attrs
                .with_titlebar_transparent(true)
                .with_fullsize_content_view(true)
                .with_titlebar_buttons_hidden(true)
                .with_title_hidden(true)
                .with_movable_by_window_background(true);
        }

        let window = Arc::new(event_loop.create_window(attrs).expect("create window"));
        // Native drop-shadow sits outside the frame and hides JankyBorders' 3px stroke.
        #[cfg(target_os = "macos")]
        window.set_has_shadow(false);

        self.scale_factor = window.scale_factor();
        let size = window.inner_size();
        self.win_w = size.width;
        self.win_h = size.height;
        self.timing.mark("window_created");

        let context = Context::new(window.clone()).expect("softbuffer context");
        let surface = Surface::new(&context, window.clone()).expect("softbuffer surface");

        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window);

        // Fit after we know size; Rift may still Resize immediately after —
        // Resized handler re-applies fit until the user zooms manually.
        self.apply_fit();
        self.timing.mark("first_fit");
        self.request_redraw();
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                self.quit(event_loop);
            }
            WindowEvent::RedrawRequested => self.redraw(),
            WindowEvent::Resized(size) => {
                let prev = (self.win_w, self.win_h);
                self.win_w = size.width;
                self.win_h = size.height;
                if prev != (self.win_w, self.win_h) {
                    self.timing.mark("wm_resize");
                }
                if !self.user_zoomed && self.fit != FitMode::Manual {
                    self.apply_fit();
                } else {
                    self.clamp_scroll(true);
                }
                self.dirty = true;
                self.request_redraw();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let old = self.render_scale();
                self.scale_factor = scale_factor;
                let new = self.render_scale();
                self.rescale_scroll(old, new);
                self.dirty = true;
                self.request_redraw();
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::KeyboardInput { event, .. } => self.handle_key(event, event_loop),
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = (position.x, position.y);
                if self.mode == Mode::Normal {
                    self.update_hover();
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.mode == Mode::Normal => {
                if let Some(link) = self.hit_test_link(self.cursor.0, self.cursor.1) {
                    self.activate_link(&link);
                    self.request_redraw();
                }
            }
            WindowEvent::MouseWheel { delta, .. } if self.mode == Mode::Normal => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => -y * SCROLL_STEP,
                    MouseScrollDelta::PixelDelta(p) => -p.y as f32,
                };
                self.scroll_vertical(dy);
                self.update_hover();
                self.request_redraw();
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
    }
}
fn draw_toc_panel(
    buffer: &mut [u32],
    w: u32,
    content_h: u32,
    entries: &[(usize, String, usize, usize)],
    toc_sel: usize,
) {
    let panel_w = (w * 2 / 5).clamp(220, 420);
    for y in 0..content_h {
        for x in 0..panel_w {
            let i = (y * w + x) as usize;
            if i < buffer.len() {
                buffer[i] = blend(buffer[i], TOC_BG, 0.88);
            }
        }
    }
    if entries.is_empty() {
        draw_text_5x7(buffer, w, content_h, 12, 16, "no chapters", STATUS_FG);
        return;
    }
    let sel = toc_sel.min(entries.len() - 1);
    let row_h = 14u32;
    let visible = (content_h.saturating_sub(24) / row_h) as usize;
    let start = sel.saturating_sub(visible / 2);
    for (row, (orig_i, title, page, depth)) in entries.iter().skip(start).take(visible).enumerate()
    {
        let y = 12 + row as u32 * row_h;
        let selected = *orig_i == entries[sel].0;
        if selected {
            for yy in y..y + row_h - 2 {
                for x in 4..panel_w.saturating_sub(4) {
                    let i = (yy * w + x) as usize;
                    if i < buffer.len() {
                        buffer[i] = TOC_SEL;
                    }
                }
            }
        }
        let indent = *depth as u32 * 10;
        let fg = if selected { BG } else { STATUS_FG };
        let label = format!(
            "{}{}  p{}",
            " ".repeat(*depth),
            truncate(title, 36),
            page + 1
        );
        draw_text_5x7(buffer, w, content_h, 10 + indent, y + 2, &label, fg);
    }
}

#[allow(clippy::too_many_arguments)]
fn blit(
    dst: &mut [u32],
    dst_w: u32,
    dst_h: u32,
    src: &[u32],
    src_w: u32,
    src_h: u32,
    ox: i32,
    oy: i32,
) {
    for sy in 0..src_h {
        let dy = oy + sy as i32;
        if dy < 0 || dy >= dst_h as i32 {
            continue;
        }
        for sx in 0..src_w {
            let dx = ox + sx as i32;
            if dx < 0 || dx >= dst_w as i32 {
                continue;
            }
            dst[(dy as u32 * dst_w + dx as u32) as usize] = src[(sy * src_w + sx) as usize];
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fill_rect_blend(
    buf: &mut [u32],
    w: u32,
    h: u32,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    color: u32,
    alpha: f32,
) {
    // Clamp in signed space first. Casting a negative y1/x1 to u32 wraps to a
    // huge value (e.g. -1 → u32::MAX), which then `.min(h)` still yields a
    // full-height/width fill — that was the vertical search-highlight streaks.
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(w as i32);
    let y1 = y1.min(h as i32);
    if x1 <= x0 || y1 <= y0 {
        return;
    }
    let x0 = x0 as u32;
    let y0 = y0 as u32;
    let x1 = x1 as u32;
    let y1 = y1 as u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let i = (y * w + x) as usize;
            if i < buf.len() {
                buf[i] = blend(buf[i], color, alpha);
            }
        }
    }
}

fn blend(dst: u32, src: u32, a: f32) -> u32 {
    let a = a.clamp(0.0, 1.0);
    let dr = ((dst >> 16) & 0xff) as f32;
    let dg = ((dst >> 8) & 0xff) as f32;
    let db = (dst & 0xff) as f32;
    let sr = ((src >> 16) & 0xff) as f32;
    let sg = ((src >> 8) & 0xff) as f32;
    let sb = (src & 0xff) as f32;
    let r = (dr * (1.0 - a) + sr * a).round() as u32;
    let g = (dg * (1.0 - a) + sg * a).round() as u32;
    let b = (db * (1.0 - a) + sb * a).round() as u32;
    (r.min(255) << 16) | (g.min(255) << 8) | b.min(255)
}

fn truncate(s: &str, max: usize) -> String {
    let mut t: String = s.chars().take(max).collect();
    if s.chars().count() > max {
        t.push('…');
    }
    t
}

fn draw_text_5x7(buf: &mut [u32], w: u32, h: u32, x0: u32, y0: u32, text: &str, color: u32) {
    draw_text_5x7_scaled(buf, w, h, x0, y0, text, color, 1);
}

#[allow(clippy::too_many_arguments)]
fn draw_text_5x7_scaled(
    buf: &mut [u32],
    w: u32,
    h: u32,
    x0: u32,
    y0: u32,
    text: &str,
    color: u32,
    scale: u32,
) {
    let scale = scale.max(1);
    let mut x = x0;
    for ch in text.chars() {
        if let Some(glyph) = glyph5x7(ch) {
            for row in 0..7u32 {
                let bits = glyph[row as usize];
                for col in 0..5u32 {
                    if bits & (1 << (4 - col)) != 0 {
                        for dy in 0..scale {
                            for dx in 0..scale {
                                let px = x + col * scale + dx;
                                let py = y0 + row * scale + dy;
                                if px < w && py < h {
                                    buf[(py * w + px) as usize] = color;
                                }
                            }
                        }
                    }
                }
            }
        }
        x += 6 * scale;
        if x + 5 * scale >= w {
            break;
        }
    }
}

fn glyph5x7(ch: char) -> Option<[u8; 7]> {
    const fn g(rows: [u8; 7]) -> [u8; 7] {
        rows
    }
    Some(match ch {
        ' ' => g([0, 0, 0, 0, 0, 0, 0]),
        '%' => g([0x12, 0x14, 0x08, 0x08, 0x08, 0x14, 0x12]),
        '/' => g([0x02, 0x02, 0x04, 0x08, 0x10, 0x10, 0x10]),
        '[' => g([0x0E, 0x08, 0x08, 0x08, 0x08, 0x08, 0x0E]),
        ']' => g([0x1C, 0x04, 0x04, 0x04, 0x04, 0x04, 0x1C]),
        '.' => g([0x00, 0x00, 0x00, 0x00, 0x00, 0x0C, 0x0C]),
        ':' => g([0x00, 0x0C, 0x0C, 0x00, 0x0C, 0x0C, 0x00]),
        '_' => g([0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x1F]),
        '-' => g([0x00, 0x00, 0x00, 0x1F, 0x00, 0x00, 0x00]),
        '…' => g([0x00, 0x00, 0x00, 0x00, 0x00, 0x15, 0x00]),
        '0' => g([0x0E, 0x11, 0x13, 0x15, 0x19, 0x11, 0x0E]),
        '1' => g([0x04, 0x0C, 0x04, 0x04, 0x04, 0x04, 0x0E]),
        '2' => g([0x0E, 0x11, 0x01, 0x06, 0x08, 0x10, 0x1F]),
        '3' => g([0x1F, 0x02, 0x04, 0x02, 0x01, 0x11, 0x0E]),
        '4' => g([0x02, 0x06, 0x0A, 0x12, 0x1F, 0x02, 0x02]),
        '5' => g([0x1F, 0x10, 0x1E, 0x01, 0x01, 0x11, 0x0E]),
        '6' => g([0x06, 0x08, 0x10, 0x1E, 0x11, 0x11, 0x0E]),
        '7' => g([0x1F, 0x01, 0x02, 0x04, 0x08, 0x08, 0x08]),
        '8' => g([0x0E, 0x11, 0x11, 0x0E, 0x11, 0x11, 0x0E]),
        '9' => g([0x0E, 0x11, 0x11, 0x0F, 0x01, 0x02, 0x0C]),
        'A' | 'a' => g([0x0E, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
        'B' | 'b' => g([0x1E, 0x11, 0x11, 0x1E, 0x11, 0x11, 0x1E]),
        'C' | 'c' => g([0x0E, 0x11, 0x10, 0x10, 0x10, 0x11, 0x0E]),
        'D' | 'd' => g([0x1C, 0x12, 0x11, 0x11, 0x11, 0x12, 0x1C]),
        'E' | 'e' => g([0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x1F]),
        'F' | 'f' => g([0x1F, 0x10, 0x10, 0x1E, 0x10, 0x10, 0x10]),
        'G' | 'g' => g([0x0E, 0x11, 0x10, 0x17, 0x11, 0x11, 0x0F]),
        'H' | 'h' => g([0x11, 0x11, 0x11, 0x1F, 0x11, 0x11, 0x11]),
        'I' | 'i' => g([0x0E, 0x04, 0x04, 0x04, 0x04, 0x04, 0x0E]),
        'J' | 'j' => g([0x07, 0x02, 0x02, 0x02, 0x02, 0x12, 0x0C]),
        'K' | 'k' => g([0x11, 0x12, 0x14, 0x18, 0x14, 0x12, 0x11]),
        'L' | 'l' => g([0x10, 0x10, 0x10, 0x10, 0x10, 0x10, 0x1F]),
        'M' | 'm' => g([0x11, 0x1B, 0x15, 0x15, 0x11, 0x11, 0x11]),
        'N' | 'n' => g([0x11, 0x19, 0x15, 0x13, 0x11, 0x11, 0x11]),
        'O' | 'o' => g([0x0E, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
        'P' | 'p' => g([0x1E, 0x11, 0x11, 0x1E, 0x10, 0x10, 0x10]),
        'Q' | 'q' => g([0x0E, 0x11, 0x11, 0x11, 0x15, 0x12, 0x0D]),
        'R' | 'r' => g([0x1E, 0x11, 0x11, 0x1E, 0x14, 0x12, 0x11]),
        'S' | 's' => g([0x0F, 0x10, 0x10, 0x0E, 0x01, 0x01, 0x1E]),
        'T' | 't' => g([0x1F, 0x04, 0x04, 0x04, 0x04, 0x04, 0x04]),
        'U' | 'u' => g([0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x0E]),
        'V' | 'v' => g([0x11, 0x11, 0x11, 0x11, 0x11, 0x0A, 0x04]),
        'W' | 'w' => g([0x11, 0x11, 0x11, 0x15, 0x15, 0x1B, 0x11]),
        'X' | 'x' => g([0x11, 0x11, 0x0A, 0x04, 0x0A, 0x11, 0x11]),
        'Y' | 'y' => g([0x11, 0x11, 0x0A, 0x04, 0x04, 0x04, 0x04]),
        'Z' | 'z' => g([0x1F, 0x01, 0x02, 0x04, 0x08, 0x10, 0x1F]),
        _ => g([0x1F, 0x11, 0x11, 0x11, 0x11, 0x11, 0x1F]),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn hello() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/hello.pdf")
    }

    fn temp_history() -> History {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("riftpdf-app-hist-{nanos}.json"));
        let _ = std::fs::remove_file(&path);
        History::load_from(path)
    }

    fn app_at(w: u32, h: u32) -> App {
        let mut app = App::new_with_history(hello(), temp_history()).expect("open fixture");
        app.win_w = w;
        app.win_h = h;
        app.scale_factor = 1.0;
        app
    }

    #[test]
    fn opens_fixture() {
        let app = App::new_with_history(hello(), temp_history()).expect("open");
        assert_eq!(app.doc.page_count(), 2);
        assert_eq!(app.fit, FitMode::FitWidth);
    }

    #[test]
    fn restores_page_from_history_on_open() {
        let path = hello();
        let mut hist = temp_history();
        hist.save(
            &path,
            FileState {
                page: 1,
                zoom: 1.0,
                fit: FitMode::FitWidth,
            },
        );
        let app = App::new_with_history(path, hist).expect("open");
        assert_eq!(app.page, 1);
    }

    #[test]
    fn apply_fit_width_sets_zoom_from_page_size() {
        let mut app = app_at(600, 800);
        app.fit = FitMode::FitWidth;
        app.page = 0;
        app.apply_fit();
        let (pw, _) = app.doc.page_size(0).unwrap();
        let expected = zoom_fit_width(600.0, pw, 1.0);
        assert!((app.zoom - expected).abs() < 1e-4);
    }

    #[test]
    fn apply_fit_preserves_page() {
        let mut app = app_at(600, 800);
        app.page = 1;
        app.fit = FitMode::FitWidth;
        app.apply_fit();
        assert_eq!(app.page, 1);
        assert!(app.scroll_y > 0.0, "page 1 should not sit at scroll_y=0");
    }

    #[test]
    fn goto_page_updates_scroll_and_page() {
        let mut app = app_at(600, 800);
        app.apply_fit();
        app.goto_page(1);
        assert_eq!(app.page, 1);
        let y1 = app.layout.as_ref().unwrap().y_start[1];
        assert!((app.scroll_y - y1).abs() < 1.0);
    }

    #[test]
    fn scroll_vertical_clamps_and_syncs_page() {
        let mut app = app_at(600, 800);
        app.apply_fit();
        app.scroll_vertical(10_000.0);
        let layout = app.layout.as_ref().unwrap();
        let max_y = (layout.doc_height - app.content_h()).max(0.0);
        assert!((app.scroll_y - max_y).abs() < 1e-3);
        assert_eq!(app.page, app.doc.page_count() - 1);
    }

    #[test]
    fn zoom_by_marks_manual_and_user_zoomed() {
        let mut app = app_at(600, 800);
        app.apply_fit();
        let before = app.zoom;
        app.zoom_by(ZOOM_STEP);
        assert!(app.user_zoomed);
        assert_eq!(app.fit, FitMode::Manual);
        assert!(app.zoom > before);
    }

    #[test]
    fn title_includes_page_and_zoom() {
        let mut app = app_at(600, 800);
        app.page = 0;
        app.zoom = 1.5;
        let t = app.title();
        assert!(t.contains("[1/2]"));
        assert!(t.contains("150%"));
    }

    #[test]
    fn content_h_respects_status_toggle() {
        let mut app = app_at(600, 800);
        assert_eq!(app.content_h(), (800 - STATUS_H) as f32);
        app.show_status = false;
        assert_eq!(app.content_h(), 800.0);
    }

    #[test]
    fn fill_rect_blend_skips_fully_offscreen_negative_y() {
        // Regression: negative y1 cast to u32 used to wrap and fill the whole buffer.
        let w = 40u32;
        let h = 60u32;
        let mut buf = vec![0u32; (w * h) as usize];
        fill_rect_blend(&mut buf, w, h, 10, -200, 20, -100, 0x00ff_0000, 1.0);
        assert!(
            buf.iter().all(|&p| p == 0),
            "off-screen rect must not paint"
        );
    }

    #[test]
    fn fill_rect_blend_clips_partially_visible() {
        let w = 40u32;
        let h = 60u32;
        let mut buf = vec![0u32; (w * h) as usize];
        // Rect straddles the top edge: only y=0..5 should be painted.
        fill_rect_blend(&mut buf, w, h, 5, -5, 8, 5, 0x00ff_0000, 1.0);
        let painted = buf.iter().filter(|&&p| p != 0).count();
        assert_eq!(painted, 3 * 5); // width 3, height 5
                                    // Nothing below y=5
        for y in 5..h {
            for x in 0..w {
                assert_eq!(buf[(y * w + x) as usize], 0);
            }
        }
    }
}
