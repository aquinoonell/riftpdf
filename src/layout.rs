//! Pure layout / scroll math (no windowing). Kept separate so regressions in
//! fit-width, zoom-to-center, and page sync stay easy to catch in unit tests.

/// Index of the page that owns `focus_y` in a continuous layout.
///
/// `focus_y` is typically `scroll_y + content_h * 0.25` (upper quarter of the
/// viewport), matching the GUI status-line page number.
pub fn page_at_y(y_start: &[f32], focus_y: f32) -> usize {
    let mut page = 0usize;
    for (i, &y) in y_start.iter().enumerate() {
        if y <= focus_y {
            page = i;
        } else {
            break;
        }
    }
    page
}

pub fn focus_y(scroll_y: f32, content_h: f32) -> f32 {
    scroll_y + content_h * 0.25
}

/// Logical zoom so a page of width `page_w` (PDF points) fills `view_w`
/// physical pixels at the given DPI scale factor.
pub fn zoom_fit_width(view_w: f32, page_w: f32, dpi: f32) -> f32 {
    if page_w <= 0.0 || dpi <= 0.0 || view_w <= 0.0 {
        return 1.0;
    }
    view_w / (page_w * dpi)
}

pub fn zoom_best_fit(view_w: f32, view_h: f32, page_w: f32, page_h: f32, dpi: f32) -> f32 {
    if page_w <= 0.0 || page_h <= 0.0 || dpi <= 0.0 || view_w <= 0.0 || view_h <= 0.0 {
        return 1.0;
    }
    let zx = view_w / (page_w * dpi);
    let zy = view_h / (page_h * dpi);
    zx.min(zy)
}

/// Keep the document point that was in the middle of the viewport centered
/// after a scale change (zoom-to-center).
pub fn rescale_center(
    scroll_x: f32,
    scroll_y: f32,
    view_w: f32,
    view_h: f32,
    old_scale: f32,
    new_scale: f32,
) -> (f32, f32) {
    if old_scale <= 0.0 || new_scale <= 0.0 || (new_scale - old_scale).abs() <= f32::EPSILON {
        return (scroll_x, scroll_y);
    }
    let ratio = new_scale / old_scale;
    let cx = scroll_x + view_w * 0.5;
    let cy = scroll_y + view_h * 0.5;
    (cx * ratio - view_w * 0.5, cy * ratio - view_h * 0.5)
}

pub fn clamp_scroll(
    scroll_x: f32,
    scroll_y: f32,
    doc_height: f32,
    max_width: f32,
    content_h: f32,
    win_w: f32,
) -> (f32, f32) {
    let max_y = (doc_height - content_h).max(0.0);
    let max_x = (max_width - win_w).max(0.0);
    (scroll_x.clamp(0.0, max_x), scroll_y.clamp(0.0, max_y))
}

/// Horizontal pan that puts the page in the middle of the viewport.
/// Zero when the page is narrower than the window (draw path centers those).
pub fn center_scroll_x(max_width: f32, win_w: f32) -> f32 {
    (max_width - win_w).max(0.0) * 0.5
}

/// Continuous-document Y offsets for pages of the given heights (device pixels).
pub fn y_starts(heights: &[f32], page_gap: f32) -> (Vec<f32>, f32) {
    let mut y_start = Vec::with_capacity(heights.len());
    let mut y = 0.0f32;
    for &h in heights {
        y_start.push(y);
        y += h + page_gap;
    }
    let doc_height = if heights.is_empty() {
        0.0
    } else {
        (y - page_gap).max(0.0)
    };
    (y_start, doc_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_at_y_picks_last_start_at_or_before_focus() {
        let y = [0.0, 100.0, 200.0, 300.0];
        assert_eq!(page_at_y(&y, 0.0), 0);
        assert_eq!(page_at_y(&y, 99.0), 0);
        assert_eq!(page_at_y(&y, 100.0), 1);
        assert_eq!(page_at_y(&y, 250.0), 2);
        assert_eq!(page_at_y(&y, 999.0), 3);
    }

    #[test]
    fn fit_width_and_best_fit() {
        assert!((zoom_fit_width(600.0, 300.0, 2.0) - 1.0).abs() < 1e-5);
        assert!((zoom_best_fit(600.0, 800.0, 300.0, 500.0, 1.0) - 1.6).abs() < 1e-5);
        assert!((zoom_best_fit(600.0, 400.0, 300.0, 500.0, 1.0) - 0.8).abs() < 1e-5);
    }

    #[test]
    fn rescale_keeps_viewport_center() {
        let (sx, sy) = rescale_center(100.0, 200.0, 400.0, 600.0, 1.0, 2.0);
        // Old center was (300, 500); after ×2 → (600, 1000); new scroll = center - half view
        assert!((sx - 400.0).abs() < 1e-3);
        assert!((sy - 700.0).abs() < 1e-3);
    }

    #[test]
    fn clamp_keeps_scroll_in_range() {
        let (sx, sy) = clamp_scroll(-10.0, 5000.0, 1000.0, 800.0, 400.0, 600.0);
        assert!((sx - 0.0).abs() < 1e-5);
        assert!((sy - 600.0).abs() < 1e-5); // max_y = 1000 - 400
    }

    #[test]
    fn center_scroll_x_is_half_overflow() {
        assert!((center_scroll_x(400.0, 600.0) - 0.0).abs() < 1e-5);
        assert!((center_scroll_x(1200.0, 600.0) - 300.0).abs() < 1e-5);
    }

    #[test]
    fn y_starts_accounts_for_gap() {
        let (starts, doc_h) = y_starts(&[100.0, 200.0], 12.0);
        assert_eq!(starts, vec![0.0, 112.0]);
        assert!((doc_h - 312.0).abs() < 1e-5); // 100 + 12 + 200
    }

    /// Regression: syncing page while scroll_y is still 0 after a fit must not
    /// force page 0 when the intended page is restored via y_start afterward.
    #[test]
    fn restoring_page_via_y_start_survives_focus_query() {
        let heights = [800.0f32, 800.0, 800.0];
        let (starts, _) = y_starts(&heights, 12.0);
        let page = 2;
        let scroll_y = starts[page];
        let content_h = 600.0;
        assert_eq!(page_at_y(&starts, focus_y(scroll_y, content_h)), page);
    }
}
