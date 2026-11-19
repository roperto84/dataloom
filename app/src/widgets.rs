use crate::db::{Driver, QueryOutcome, SortSpec, TableInfo};
use crate::theme::{self, Palette};
use eframe::egui::{
    self, Align, Align2, Color32, FontId, Layout, Rect, Response, RichText, Rounding, ScrollArea,
    Sense, Stroke, TextEdit, Ui, Vec2,
};
use std::path::Path;

/// A recognizable brand tint for each engine chip.
pub(crate) fn driver_color(p: &Palette, driver: Driver) -> Color32 {
    match driver {
        Driver::Sqlite => p.green,
        Driver::MySql => Color32::from_rgb(0, 117, 143),
        Driver::Postgres => Color32::from_rgb(51, 103, 145),
    }
}

pub(crate) fn fmt_ms(ms: u128) -> String {
    if ms == 0 {
        "<1 ms".to_string()
    } else {
        format!("{ms} ms")
    }
}

/// Short elapsed time for the transaction badge: `4s`, `1m 05s`.
pub(crate) fn fmt_duration(duration: std::time::Duration) -> String {
    let secs = duration.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else {
        format!("{}m {:02}s", secs / 60, secs % 60)
    }
}

pub(crate) fn truncate(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

pub(crate) fn initials(name: &str) -> String {
    let mut out = String::new();
    for word in name.split_whitespace().take(2) {
        if let Some(c) = word.chars().next() {
            out.extend(c.to_uppercase());
        }
    }
    if out.is_empty() {
        "DB".to_string()
    } else {
        out
    }
}

pub(crate) fn chip(ui: &mut Ui, p: &Palette, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::new(1.0_f32, p.border))
        .rounding(Rounding::same(4.0))
        .inner_margin(egui::Margin::symmetric(5.0, 1.5))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(9.8).color(color));
        });
}

pub(crate) fn dot(ui: &mut Ui, color: Color32, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size + 2.0), Sense::hover());
    ui.painter().circle_filled(rect.center(), size * 0.5, color);
}

fn blend(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()))
}

fn gradient_vertex(
    mesh: &mut egui::Mesh,
    rect: Rect,
    top: Color32,
    bottom: Color32,
    pos: egui::Pos2,
) -> u32 {
    let t = (((pos.x - rect.left()) / rect.width()) + ((pos.y - rect.top()) / rect.height()))
        .clamp(0.0, 1.0)
        * 0.5;
    mesh.colored_vertex(pos, blend(top, bottom, t));
    (mesh.vertices.len() - 1) as u32
}

/// The Dataloom mark: a rounded indigo-to-sky tile with a white database
/// cylinder and a mint spark. Keep the geometry in sync with
/// `tools/make_icon.py`, which renders the same drawing as the app icon.
pub(crate) fn draw_logo(ui: &mut Ui, size: f32, _p: &Palette) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    let radius = size * 0.225;

    let mut mesh = egui::Mesh::default();
    let mut ring: Vec<u32> = Vec::new();
    let corners = [
        (
            rect.right() - radius,
            rect.top() + radius,
            -std::f32::consts::FRAC_PI_2,
        ),
        (rect.right() - radius, rect.bottom() - radius, 0.0),
        (
            rect.left() + radius,
            rect.bottom() - radius,
            std::f32::consts::FRAC_PI_2,
        ),
        (
            rect.left() + radius,
            rect.top() + radius,
            std::f32::consts::PI,
        ),
    ];
    for (cx, cy, start) in corners {
        for step in 0..=8 {
            let angle = start + std::f32::consts::FRAC_PI_2 * step as f32 / 8.0;
            let pos = egui::pos2(cx + angle.cos() * radius, cy + angle.sin() * radius);
            ring.push(gradient_vertex(
                &mut mesh,
                rect,
                theme::LOGO_TOP,
                theme::LOGO_BOTTOM,
                pos,
            ));
        }
    }
    let center = gradient_vertex(
        &mut mesh,
        rect,
        theme::LOGO_TOP,
        theme::LOGO_BOTTOM,
        rect.center(),
    );
    for i in 0..ring.len() {
        mesh.indices
            .extend_from_slice(&[center, ring[i], ring[(i + 1) % ring.len()]]);
    }
    painter.add(egui::Shape::mesh(mesh));
    painter.rect_stroke(
        rect,
        Rounding::same(radius),
        Stroke::new((size * 0.012).max(1.0), Color32::from_white_alpha(24)),
    );

    let cx = rect.center().x;
    let cy = rect.center().y + size * 0.005;
    let rx = size * 0.208;
    let ry = size * 0.0855;
    let top_y = cy - size * 0.134;
    let bottom_y = cy + size * 0.134;

    painter.add(egui::Shape::ellipse_filled(
        egui::pos2(cx, bottom_y + size * 0.014),
        Vec2::new(rx * 1.02, ry * 1.10),
        Color32::from_black_alpha(22),
    ));
    painter.add(egui::Shape::ellipse_filled(
        egui::pos2(cx, bottom_y),
        Vec2::new(rx, ry),
        theme::LOGO_GLYPH,
    ));
    painter.rect_filled(
        Rect::from_min_max(egui::pos2(cx - rx, top_y), egui::pos2(cx + rx, bottom_y)),
        Rounding::ZERO,
        theme::LOGO_GLYPH,
    );
    painter.add(egui::Shape::ellipse_filled(
        egui::pos2(cx, top_y),
        Vec2::new(rx, ry),
        theme::LOGO_GLYPH,
    ));

    let band = theme::LOGO_BAND.gamma_multiply(0.42);
    if size >= 26.0 {
        for fraction in [0.42, 0.72] {
            painter.add(egui::Shape::ellipse_stroke(
                egui::pos2(cx, top_y + (bottom_y - top_y) * fraction),
                Vec2::new(rx, ry),
                Stroke::new(size * 0.014, band),
            ));
        }
    }
    painter.add(egui::Shape::ellipse_stroke(
        egui::pos2(cx, top_y),
        Vec2::new(rx, ry),
        Stroke::new(size * 0.008, theme::LOGO_BAND.gamma_multiply(0.30)),
    ));

    if size >= 34.0 {
        let spark = egui::pos2(rect.left() + size * 0.835, rect.top() + size * 0.825);
        let outer = size * 0.105;
        let inner = size * 0.036;
        painter.circle_filled(spark, inner * 1.15, theme::LOGO_SPARK);
        for k in 0..4 {
            let angle = -std::f32::consts::FRAC_PI_2 + std::f32::consts::FRAC_PI_2 * k as f32;
            let tip = egui::pos2(spark.x + angle.cos() * outer, spark.y + angle.sin() * outer);
            let left = egui::pos2(
                spark.x + (angle + std::f32::consts::FRAC_PI_4).cos() * inner,
                spark.y + (angle + std::f32::consts::FRAC_PI_4).sin() * inner,
            );
            let right = egui::pos2(
                spark.x + (angle - std::f32::consts::FRAC_PI_4).cos() * inner,
                spark.y + (angle - std::f32::consts::FRAC_PI_4).sin() * inner,
            );
            painter.add(egui::Shape::convex_polygon(
                vec![left, tip, right],
                theme::LOGO_SPARK,
                Stroke::NONE,
            ));
        }
    }
}

pub(crate) fn draw_table_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    painter.rect_stroke(rect, Rounding::same(2.0), Stroke::new(1.0_f32, color));
    for f in [0.36, 0.68] {
        painter.line_segment(
            [
                egui::pos2(rect.left(), rect.top() + rect.height() * f),
                egui::pos2(rect.right(), rect.top() + rect.height() * f),
            ],
            Stroke::new(1.0_f32, color),
        );
    }
    painter.line_segment(
        [
            egui::pos2(rect.left() + rect.width() * 0.38, rect.top()),
            egui::pos2(rect.left() + rect.width() * 0.38, rect.bottom()),
        ],
        Stroke::new(1.0_f32, color),
    );
}

pub(crate) fn draw_cross(painter: &egui::Painter, rect: Rect, color: Color32) {
    let s = Stroke::new(1.1_f32, color);
    painter.line_segment(
        [
            egui::pos2(rect.left(), rect.top()),
            egui::pos2(rect.right(), rect.bottom()),
        ],
        s,
    );
    painter.line_segment(
        [
            egui::pos2(rect.right(), rect.top()),
            egui::pos2(rect.left(), rect.bottom()),
        ],
        s,
    );
}

pub(crate) fn draw_plus(painter: &egui::Painter, center: egui::Pos2, radius: f32, color: Color32) {
    let s = Stroke::new(1.4_f32, color);
    painter.line_segment(
        [
            egui::pos2(center.x - radius, center.y),
            egui::pos2(center.x + radius, center.y),
        ],
        s,
    );
    painter.line_segment(
        [
            egui::pos2(center.x, center.y - radius),
            egui::pos2(center.x, center.y + radius),
        ],
        s,
    );
}

pub(crate) fn draw_search_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = egui::pos2(rect.left() + 8.0, rect.center().y);
    painter.circle_stroke(center, 4.2, Stroke::new(1.2_f32, color));
    painter.line_segment(
        [
            egui::pos2(center.x + 3.0, center.y + 3.0),
            egui::pos2(center.x + 6.5, center.y + 6.5),
        ],
        Stroke::new(1.2_f32, color),
    );
}

/// Down chevron used by the row detail field menus and the search box.
pub(crate) fn draw_chevron_down(painter: &egui::Painter, rect: Rect, color: Color32) {
    let center = rect.center();
    let w = rect.width().min(rect.height()) * 0.28;
    let h = w * 0.6;
    painter.line_segment(
        [
            egui::pos2(center.x - w, center.y - h),
            egui::pos2(center.x, center.y + h * 0.8),
        ],
        Stroke::new(1.4_f32, color),
    );
    painter.line_segment(
        [
            egui::pos2(center.x, center.y + h * 0.8),
            egui::pos2(center.x + w, center.y - h),
        ],
        Stroke::new(1.4_f32, color),
    );
}

/// Small folder glyph used by connection groups in the sidebar/welcome list.
pub(crate) fn draw_folder_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let body = Rect::from_min_max(
        egui::pos2(rect.left(), rect.top() + rect.height() * 0.22),
        egui::pos2(rect.right(), rect.bottom()),
    );
    painter.rect_stroke(body, Rounding::same(1.5), Stroke::new(1.2_f32, color));
    painter.add(egui::Shape::line(
        vec![
            egui::pos2(rect.left(), rect.top() + rect.height() * 0.22),
            egui::pos2(rect.left(), rect.top() + rect.height() * 0.08),
            egui::pos2(
                rect.left() + rect.width() * 0.42,
                rect.top() + rect.height() * 0.08,
            ),
            egui::pos2(
                rect.left() + rect.width() * 0.52,
                rect.top() + rect.height() * 0.22,
            ),
            egui::pos2(rect.left(), rect.top() + rect.height() * 0.22),
        ],
        Stroke::new(1.2_f32, color),
    ));
}

/// Dismisses the whole context menu. A leaf item in a submenu only closes that
/// submenu, so egui's shared context-menu state is cleared explicitly.
pub(crate) fn close_context_menu(ui: &mut Ui) {
    ui.close_menu();
    ui.ctx().data_mut(|data| {
        data.remove::<egui::menu::BarState>(egui::Id::new("__egui::context_menu"));
    });
}

/// Three slider lines: the matching-strategy/settings button beside the row
/// detail search field.
pub(crate) fn draw_sliders_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let stroke = Stroke::new(1.3_f32, color);
    for (i, knob) in [0.68_f32, 0.32_f32, 0.58_f32].into_iter().enumerate() {
        let y = rect.top() + rect.height() * (i as f32 + 0.5) / 3.0;
        painter.line_segment(
            [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
            stroke,
        );
        let x = rect.left() + rect.width() * knob;
        painter.circle_filled(egui::pos2(x, y), 1.6, color);
    }
}

/// Split-pane glyph used by the row detail toggle: a panel outline with three
/// field lines in its right half.
fn draw_split_pane_icon(painter: &egui::Painter, rect: Rect, color: Color32) {
    let inner = Rect::from_center_size(rect.center(), Vec2::new(15.0, 13.0));
    painter.rect_stroke(inner, Rounding::same(2.0), Stroke::new(1.2_f32, color));
    let split = inner.left() + 6.0;
    painter.line_segment(
        [
            egui::pos2(split, inner.top()),
            egui::pos2(split, inner.bottom()),
        ],
        Stroke::new(1.2_f32, color),
    );
    for i in 0..3 {
        let y = inner.top() + 3.0 + i as f32 * 3.0;
        painter.line_segment(
            [egui::pos2(split + 2.0, y), egui::pos2(inner.right() - 2.0, y)],
            Stroke::new(1.0_f32, color),
        );
    }
}

/// Toolbar toggle for the vertical row editor: a split pane with field lines.
pub(crate) fn row_detail_button(ui: &mut Ui, p: &Palette, active: bool) -> Response {
    let enabled = ui.is_enabled();
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), Sense::click());
    if active && enabled {
        ui.painter()
            .rect_filled(rect, Rounding::same(6.0), p.accent_soft);
    } else if resp.hovered() && enabled {
        ui.painter()
            .rect_filled(rect, Rounding::same(6.0), p.row_hover);
    }
    let color = if !enabled {
        p.faint
    } else if active {
        p.accent
    } else if resp.hovered() {
        p.text
    } else {
        p.muted
    };
    draw_split_pane_icon(ui.painter(), rect, color);
    resp
}

/// Row detail toggle in a toolbar. With the panel open it is the compact icon
/// (the table header is tight); closed, it shows the icon plus a "Row detail"
/// label so the way to bring the panel back is always visible.
pub(crate) fn row_detail_toggle(ui: &mut Ui, p: &Palette, active: bool) -> Response {
    let enabled = ui.is_enabled();
    if active && enabled {
        return row_detail_button(ui, p, true);
    }
    let font = FontId::proportional(10.8);
    let galley = ui.fonts(|fonts| {
        fonts.layout_no_wrap("Row detail".to_owned(), font.clone(), p.muted)
    });
    let width = 26.0 + galley.size().x + 14.0;
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, 26.0), Sense::click());
    ui.painter()
        .rect_filled(rect, Rounding::same(5.0), p.card);
    ui.painter()
        .rect_stroke(rect, Rounding::same(5.0), Stroke::new(1.0_f32, p.border));
    let color = if !enabled {
        p.faint
    } else if resp.hovered() {
        p.text
    } else {
        p.muted
    };
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + 13.0, rect.center().y),
        Vec2::splat(14.0),
    );
    draw_split_pane_icon(ui.painter(), icon_rect, color);
    ui.painter().text(
        egui::pos2(rect.left() + 24.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Row detail",
        font,
        color,
    );
    resp
}

pub(crate) fn plus_button(ui: &mut Ui, p: &Palette, tooltip: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(6.0), p.row_hover);
    }
    draw_plus(
        ui.painter(),
        rect.center(),
        7.0,
        if resp.hovered() { p.text } else { p.muted },
    );
    resp.on_hover_text(tooltip)
}

pub(crate) fn run_button(ui: &mut Ui, p: &Palette, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(86.0, 28.0), Sense::click());
    let bg = if !enabled {
        p.card
    } else if resp.hovered() {
        p.accent.gamma_multiply(0.88)
    } else {
        p.accent
    };
    ui.painter().rect_filled(rect, Rounding::same(6.0), bg);
    let fg = if enabled { Color32::WHITE } else { p.faint };
    let tri = Rect::from_center_size(
        egui::pos2(rect.left() + 17.0, rect.center().y),
        Vec2::new(8.0, 9.0),
    );
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            tri.left_top(),
            egui::pos2(tri.right(), tri.center().y),
            tri.left_bottom(),
        ],
        fg,
        Stroke::NONE,
    ));
    ui.painter().text(
        egui::pos2(rect.left() + 30.0, rect.center().y),
        Align2::LEFT_CENTER,
        "Run",
        FontId::proportional(12.6),
        fg,
    );
    resp.on_hover_text("Run Current Query  Cmd+Enter")
}

pub(crate) fn icon_toggle(ui: &mut Ui, p: &Palette, dark: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(6.0), p.row_hover);
    }
    let center = rect.center();
    if dark {
        ui.painter().circle_filled(center, 6.0, p.muted);
        ui.painter()
            .circle_filled(center + Vec2::new(3.0, -2.5), 5.2, p.topbar);
    } else {
        ui.painter()
            .circle_stroke(center, 5.0, Stroke::new(1.4_f32, p.muted));
        for i in 0..8 {
            let a = i as f32 * std::f32::consts::TAU / 8.0;
            let dir = Vec2::new(a.cos(), a.sin());
            ui.painter().line_segment(
                [center + dir * 7.0, center + dir * 9.0],
                Stroke::new(1.2_f32, p.muted),
            );
        }
    }
    resp.on_hover_text(if dark {
        "Switch to light theme"
    } else {
        "Switch to dark theme"
    })
}

/// Small circled "i" that opens the About panel, so the panel is reachable
/// without knowing the Cmd+, shortcut.
pub(crate) fn info_button(ui: &mut Ui, p: &Palette) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(28.0, 28.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(6.0), p.row_hover);
    }
    let color = if resp.hovered() { p.text } else { p.muted };
    let center = rect.center();
    ui.painter()
        .circle_stroke(center, 6.3, Stroke::new(1.3_f32, color));
    ui.painter().text(
        center + Vec2::new(0.0, -0.6),
        Align2::CENTER_CENTER,
        "i",
        FontId::proportional(9.6),
        color,
    );
    resp.on_hover_text("About Dataloom  Cmd+,")
}

pub(crate) fn section_header(
    ui: &mut Ui,
    p: &Palette,
    title: &str,
    count: usize,
    open: &mut bool,
) -> bool {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 26.0), Sense::click());
    if resp.hovered() {
        ui.painter()
            .rect_filled(rect, Rounding::same(5.0), p.row_hover);
    }
    let center = egui::pos2(rect.left() + 8.0, rect.center().y);
    draw_chevron(ui.painter(), center, *open, p.muted);
    ui.painter().text(
        egui::pos2(rect.left() + 20.0, rect.center().y),
        Align2::LEFT_CENTER,
        title,
        FontId::proportional(10.3),
        p.faint,
    );
    ui.painter().text(
        egui::pos2(rect.right() - 6.0, rect.center().y),
        Align2::RIGHT_CENTER,
        count.to_string(),
        FontId::proportional(10.0),
        p.faint,
    );
    if resp.clicked() {
        *open = !*open;
        true
    } else {
        false
    }
}

fn draw_chevron(painter: &egui::Painter, center: egui::Pos2, open: bool, color: Color32) {
    let shape = if open {
        vec![
            egui::pos2(center.x - 3.5, center.y - 1.6),
            egui::pos2(center.x + 3.5, center.y - 1.6),
            egui::pos2(center.x, center.y + 2.4),
        ]
    } else {
        vec![
            egui::pos2(center.x - 1.6, center.y - 3.5),
            egui::pos2(center.x + 2.4, center.y),
            egui::pos2(center.x - 1.6, center.y + 3.5),
        ]
    };
    painter.add(egui::Shape::convex_polygon(shape, color, Stroke::NONE));
}

pub(crate) fn table_row(ui: &mut Ui, p: &Palette, table: &TableInfo, selected: bool) -> Response {
    let (rect, resp) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), 25.0), Sense::click());
    let bg = if selected {
        p.accent_soft
    } else if resp.hovered() {
        p.row_hover
    } else {
        Color32::TRANSPARENT
    };
    if bg != Color32::TRANSPARENT {
        ui.painter().rect_filled(rect, Rounding::same(5.0), bg);
    }
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(rect.left_top(), Vec2::new(2.0, rect.height())),
            Rounding::same(1.0),
            p.accent,
        );
    }
    let icon_rect = Rect::from_center_size(
        egui::pos2(rect.left() + 16.0, rect.center().y),
        Vec2::splat(11.0),
    );
    if table.is_view {
        ui.painter()
            .circle_stroke(icon_rect.center(), 5.0, Stroke::new(1.0_f32, p.purple));
        ui.painter().line_segment(
            [
                egui::pos2(icon_rect.center().x - 2.0, icon_rect.center().y),
                egui::pos2(icon_rect.center().x + 2.0, icon_rect.center().y),
            ],
            Stroke::new(1.0_f32, p.purple),
        );
    } else {
        draw_table_icon(ui.painter(), icon_rect, p.muted);
    }
    let label = match &table.schema {
        Some(schema) if !schema.is_empty() => format!("{schema}.{}", table.name),
        _ => table.name.clone(),
    };
    ui.painter().text(
        egui::pos2(rect.left() + 28.0, rect.center().y),
        Align2::LEFT_CENTER,
        truncate(&label, 24),
        FontId::proportional(theme::SIDEBAR_FONT_SIZE),
        if selected { p.text } else { p.text },
    );
    let right = match table.rows {
        Some(n) => format_count(n),
        None => String::new(),
    };
    if !right.is_empty() {
        ui.painter().text(
            egui::pos2(rect.right() - 6.0, rect.center().y),
            Align2::RIGHT_CENTER,
            right,
            FontId::proportional(10.0),
            p.faint,
        );
    }
    resp
}

fn format_count(n: i64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 10_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}

pub(crate) fn result_tab(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let text = RichText::new(label)
        .size(11.4)
        .color(if selected { p.text } else { p.muted });
    let resp = ui.add(
        egui::Button::new(text)
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::NONE),
    );
    let rect = resp.rect;
    if selected {
        ui.painter().rect_filled(
            Rect::from_min_size(
                egui::pos2(rect.left() + 2.0, rect.bottom() - 2.0),
                Vec2::new(rect.width() - 4.0, 2.0),
            ),
            Rounding::same(1.0),
            p.accent,
        );
    }
    resp
}

pub(crate) fn segmented(ui: &mut Ui, p: &Palette, label: &str, selected: bool) -> Response {
    let text = RichText::new(label)
        .size(11.3)
        .color(if selected { p.text } else { p.muted });
    let resp = ui.add(
        egui::Button::new(text)
            .fill(if selected {
                p.accent_soft
            } else {
                Color32::TRANSPARENT
            })
            .stroke(if selected {
                Stroke::new(1.0_f32, p.accent.gamma_multiply(0.5))
            } else {
                Stroke::NONE
            })
            .rounding(Rounding::same(5.0)),
    );
    resp
}

pub(crate) fn icon_button(ui: &mut Ui, p: &Palette, label: &str) -> Response {
    ui.add(
        egui::Button::new(RichText::new(label).size(10.8).color(p.muted))
            .fill(p.card)
            .stroke(Stroke::new(1.0_f32, p.border))
            .rounding(Rounding::same(5.0)),
    )
}

pub(crate) fn copy_button(ui: &mut Ui, p: &Palette) -> bool {
    ui.add(
        egui::Button::new(RichText::new("Copy").size(10.8).color(p.muted))
            .fill(Color32::TRANSPARENT)
            .stroke(Stroke::new(1.0_f32, p.border))
            .rounding(Rounding::same(5.0)),
    )
    .clicked()
}

pub(crate) fn error_box(ui: &mut Ui, p: &Palette, message: &str) {
    ui.add_space(12.0);
    ui.horizontal(|ui| {
        ui.add_space(14.0);
        egui::Frame::none()
            .fill(if p.dark {
                Color32::from_rgb(58, 30, 31)
            } else {
                Color32::from_rgb(254, 243, 242)
            })
            .stroke(Stroke::new(1.0_f32, p.red.gamma_multiply(0.6)))
            .rounding(Rounding::same(7.0))
            .inner_margin(egui::Margin::symmetric(12.0, 10.0))
            .show(ui, |ui| {
                ui.set_max_width((ui.available_width() - 16.0).max(240.0));
                ui.label(
                    RichText::new("Query error")
                        .size(11.0)
                        .strong()
                        .color(p.red),
                );
                ui.add_space(2.0);
                ui.label(RichText::new(message).size(11.8).color(p.text));
            });
    });
}

pub(crate) fn empty_panel(ui: &mut Ui, p: &Palette, title: &str, hint: &str) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width(), ui.available_height().max(120.0)),
        Sense::hover(),
    );
    let painter = ui.painter();
    let center = rect.center() - Vec2::new(0.0, 8.0);
    painter.circle_stroke(center, 16.0, Stroke::new(1.0_f32, p.border));
    painter.line_segment(
        [
            egui::pos2(center.x - 6.0, center.y - 2.0),
            egui::pos2(center.x + 6.0, center.y - 2.0),
        ],
        Stroke::new(1.0_f32, p.faint),
    );
    painter.line_segment(
        [
            egui::pos2(center.x - 6.0, center.y + 2.0),
            egui::pos2(center.x + 2.0, center.y + 2.0),
        ],
        Stroke::new(1.0_f32, p.faint),
    );
    painter.text(
        center + Vec2::new(0.0, 32.0),
        Align2::CENTER_CENTER,
        title,
        FontId::proportional(theme::SIDEBAR_FONT_SIZE),
        p.muted,
    );
    painter.text(
        center + Vec2::new(0.0, 49.0),
        Align2::CENTER_CENTER,
        hint,
        FontId::proportional(11.0),
        p.faint,
    );
}

pub(crate) fn messages_ui(ui: &mut Ui, p: &Palette, outcome: Option<&QueryOutcome>) {
    ScrollArea::vertical().show(ui, |ui| {
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            ui.add_space(16.0);
            ui.vertical(|ui| match outcome {
                Some(out) => {
                    if let Some(err) = &out.error {
                        ui.label(
                            RichText::new("Query error")
                                .size(11.0)
                                .strong()
                                .color(p.red),
                        );
                        ui.label(RichText::new(err).size(12.2).color(p.text));
                        ui.add_space(10.0);
                    } else {
                        ui.label(RichText::new(out.ok_message()).size(12.4).color(p.text));
                        ui.add_space(8.0);
                    }
                    for (label, value) in [
                        ("Statements", out.statements.to_string()),
                        ("Rows returned", out.rows.len().to_string()),
                        ("Rows affected", out.rows_affected.to_string()),
                        ("Execution time", fmt_ms(out.elapsed_ms)),
                        ("Columns", out.columns.len().to_string()),
                    ] {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(label).size(11.0).color(p.faint));
                            ui.label(RichText::new(value).size(11.4).color(p.muted));
                        });
                    }
                    if out.truncated {
                        ui.add_space(8.0);
                        ui.label(RichText::new(&out.message).size(11.0).color(p.amber));
                    }
                }
                None => {
                    ui.label(
                        RichText::new("No query has run yet.")
                            .size(12.0)
                            .color(p.muted),
                    );
                    ui.add_space(3.0);
                    ui.label(
                        RichText::new("Press Cmd+Enter in the editor to run one.")
                            .size(11.2)
                            .color(p.faint),
                    );
                }
            });
        });
    });
}

pub(crate) fn outcome_to_tsv(out: &QueryOutcome) -> String {
    let mut lines = Vec::new();
    lines.push(out.columns.join("\t"));
    for row in &out.rows {
        let cells: Vec<String> = row
            .iter()
            .map(|c| match c {
                Some(v) => v.replace(['\t', '\n', '\r'], " "),
                None => String::new(),
            })
            .collect();
        lines.push(cells.join("\t"));
    }
    lines.join("\n")
}

pub(crate) fn save_png(image: &egui::ColorImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Could not create the screenshot folder: {e}"))?;
        }
    }
    let [width, height] = image.size;
    let mut rgba = Vec::with_capacity(width * height * 4);
    for pixel in &image.pixels {
        rgba.extend_from_slice(&[pixel.r(), pixel.g(), pixel.b(), pixel.a()]);
    }
    image::save_buffer(
        path,
        &rgba,
        width as u32,
        height as u32,
        image::ColorType::Rgba8,
    )
    .map_err(|e| e.to_string())
}

// ----------------------------------------------------------------------
// Data grid
// ----------------------------------------------------------------------

/// Rectangular selection in the data grid. `anchor` is where the selection
/// started (Shift-click/drag) and `cursor` is the moving end; the pair defines
/// a cell range and powers Copy, multi-row highlighting and keyboard moves.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct GridSelection {
    anchor: Option<(usize, usize)>,
    cursor: Option<(usize, usize)>,
}

impl GridSelection {
    pub fn set_cell(&mut self, row: usize, col: usize) {
        self.anchor = Some((row, col));
        self.cursor = Some((row, col));
    }

    pub fn extend_to(&mut self, row: usize, col: usize) {
        if self.anchor.is_none() {
            self.anchor = Some((row, col));
        }
        self.cursor = Some((row, col));
    }

    pub fn select_row(&mut self, row: usize, columns: usize) {
        if columns == 0 {
            self.clear();
            return;
        }
        self.anchor = Some((row, 0));
        self.cursor = Some((row, columns - 1));
    }

    pub fn extend_row_to(&mut self, row: usize, columns: usize) {
        if columns == 0 {
            self.clear();
            return;
        }
        let anchor_row = self.anchor.map(|(row, _)| row).unwrap_or(row);
        self.anchor = Some((anchor_row, 0));
        self.cursor = Some((row, columns - 1));
    }

    pub fn select_all(&mut self, rows: usize, columns: usize) {
        if rows == 0 || columns == 0 {
            self.clear();
            return;
        }
        self.anchor = Some((0, 0));
        self.cursor = Some((rows - 1, columns - 1));
    }

    pub fn clear(&mut self) {
        self.anchor = None;
        self.cursor = None;
    }

    pub fn is_empty(&self) -> bool {
        self.anchor.is_none()
    }

    pub fn cursor(&self) -> Option<(usize, usize)> {
        self.cursor
    }

    pub fn cursor_row(&self) -> Option<usize> {
        self.cursor.map(|(row, _)| row)
    }

    /// Inclusive `(top-left, bottom-right)` corners, independent of the drag
    /// direction.
    pub fn normalized(&self) -> Option<((usize, usize), (usize, usize))> {
        let (a, b) = (self.anchor?, self.cursor?);
        Some((
            (a.0.min(b.0), a.1.min(b.1)),
            (a.0.max(b.0), a.1.max(b.1)),
        ))
    }

    pub fn contains(&self, row: usize, col: usize) -> bool {
        self.normalized()
            .map(|(min, max)| {
                row >= min.0 && row <= max.0 && col >= min.1 && col <= max.1
            })
            .unwrap_or(false)
    }

    pub fn row_selected(&self, row: usize) -> bool {
        self.normalized()
            .map(|(min, max)| row >= min.0 && row <= max.0)
            .unwrap_or(false)
    }

    /// `(rows, columns)` covered by the selection, for the footer summary.
    pub fn summary(&self) -> Option<(usize, usize)> {
        let (min, max) = self.normalized()?;
        Some((max.0 - min.0 + 1, max.1 - min.1 + 1))
    }

    /// Selection as tab-separated text (Excel/Numbers/Sheets friendly). NULL
    /// cells become empty fields; fields containing tabs, newlines or quotes
    /// are quoted as in TSV.
    pub fn to_tsv(
        &self,
        columns: &[String],
        rows: &[Vec<Option<String>>],
    ) -> Option<String> {
        let (min, max) = self.normalized()?;
        let row_end = max.0.min(rows.len().checked_sub(1)?);
        let col_end = max.1.min(columns.len().checked_sub(1)?);
        if min.0 > row_end || min.1 > col_end {
            return None;
        }
        let mut out = String::new();
        for row in min.0..=row_end {
            if row > min.0 {
                out.push('\n');
            }
            for col in min.1..=col_end {
                if col > min.1 {
                    out.push('\t');
                }
                let value = rows
                    .get(row)
                    .and_then(|values| values.get(col))
                    .and_then(|value| value.as_deref())
                    .unwrap_or("");
                out.push_str(&tsv_field(value));
            }
        }
        Some(out)
    }
}

fn tsv_field(value: &str) -> String {
    if value.contains(['\t', '\n', '\r', '"']) {
        format!("\"{}\"", value.replace('"', "\"\""))
    } else {
        value.to_string()
    }
}

/// Focus id shared by the grid and the app's global shortcut handler. The
/// app checks this id to keep Space/Cmd+Arrow working while the grid has
/// keyboard focus, without letting those shortcuts fire inside a text field.
pub(crate) fn grid_focus_id(grid_id: egui::Id) -> egui::Id {
    grid_id.with("grid-focus")
}

/// State of the in-cell editor. The grid paints the cell, borrows this for the
/// text and reports what the user did through [`GridAction`]. The right-click
/// menu offers Set NULL, Set empty and Copy, mirroring the row detail panel.
pub(crate) struct GridEdit {
    pub row: usize,
    pub col: usize,
    pub text: String,
    /// Value loaded from the database. `None` is SQL NULL, and it lets the
    /// commit path keep NULL as NULL instead of turning it into ''.
    pub original: Option<String>,
    /// The text field has to take focus on its first frame.
    pub focus: bool,
    /// Editor chosen from the column's declared type.
    pub kind: crate::db::FieldEditor,
}

/// Visual metrics for the grid, fed from the persistent settings.
#[derive(Clone, Copy)]
pub(crate) struct GridStyle {
    pub row_height: f32,
    pub font_size: f32,
    pub alternate_rows: bool,
    pub show_row_numbers: bool,
}

/// What happened inside the data grid this frame.
#[derive(Clone)]
pub(crate) enum GridAction {
    None,
    /// The user double-clicked a cell and wants to edit it.
    OpenEditor(usize, usize),
    /// The in-cell editor wants its text written back.
    Commit,
    /// The in-cell editor was dismissed with Escape.
    Cancel,
    /// Right-click menu: write SQL NULL into the cell.
    SetNull(usize, usize),
    /// Right-click menu: write an empty string (not NULL).
    SetEmpty(usize, usize),
    /// Copy the cell value or the whole selection to the clipboard.
    CopyText(String),
    /// A header click changed the sort order; the caller may need to refresh.
    SortChanged,
    /// Write a value chosen by a type-aware editor (boolean toggle, enum
    /// pick, date picker). `None` is SQL NULL.
    SetValue(usize, usize, Option<String>),
    /// JSON/blob values that do not fit a cell open in the full value window.
    ExpandValue(usize, usize),
    /// The user dragged a column separator; the new width is already in the
    /// width vector, but the caller may want to persist it.
    WidthsChanged,
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn data_grid(
    ui: &mut Ui,
    p: &Palette,
    id: egui::Id,
    columns: &[String],
    rows: &[Vec<Option<String>>],
    selection: &mut GridSelection,
    row_offset: usize,
    empty_text: &str,
    edit: Option<&mut GridEdit>,
    can_edit: bool,
    // One flag per column: true when the column accepts SQL NULL. Used to
    // disable the grid's "Set NULL" menu item on NOT NULL / primary-key
    // columns.
    null_allowed: &[bool],
    sort: &mut Vec<SortSpec>,
    sortable: bool,
    style: GridStyle,
    widths: &mut Vec<f32>,
    column_types: &[String],
    scroll: &mut f32,
    restore_scroll: Option<f32>,
) -> GridAction {
    let row_h = style.row_height.max(style.font_size + 6.0).max(16.0);
    let font = style.font_size.max(9.0);
    const HEADER_H: f32 = 30.0;
    let gutter_w: f32 = if style.show_row_numbers { 54.0 } else { 10.0 };

    if widths.len() != columns.len() {
        *widths = measure_column_widths(ui, columns, rows, font);
    }
    let total_w = gutter_w + widths.iter().sum::<f32>() + 2.0;
    let mut action = GridAction::None;
    let mut edit = edit;
    let mut scroll_to_cursor = false;

    // A rerun query can return different columns; drop terms that no longer
    // exist instead of rendering a stale arrow or emitting bad SQL.
    sort.retain(|spec| columns.iter().any(|column| column == &spec.column));

    // A tiny focusable widget lives outside the scroll area, so it never
    // scrolls out of view and loses keyboard focus. Clicking a cell requests
    // it; the key handling below only runs while it owns focus.
    let focus_id = grid_focus_id(id);
    let focus = ui.interact(
        Rect::from_min_size(ui.cursor().min, Vec2::splat(1.0)),
        focus_id,
        Sense::focusable_noninteractive(),
    );
    // Keep Tab and the arrow keys for the grid instead of egui's focus
    // navigation. The filter only takes effect once the widget has held focus
    // for a frame, which is enough because focus is requested on pointer-down.
    ui.memory_mut(|memory| {
        memory.set_focus_lock_filter(
            focus_id,
            egui::EventFilter {
                tab: true,
                horizontal_arrows: true,
                vertical_arrows: true,
                escape: false,
            },
        );
    });
    let mut area = ScrollArea::both()
        .id_salt(id.with("scroll"))
        .auto_shrink([false, false]);
    if let Some(offset) = restore_scroll {
        area = area.vertical_scroll_offset(offset);
    }
    let output = area.show_viewport(ui, |ui, viewport| {
            let full_w = total_w.max(ui.available_width());
            // Virtualized rows need an exact pitch; the table's nib records
            // zero intercell spacing.
            ui.spacing_mut().item_spacing = Vec2::ZERO;
            let body_h = if rows.is_empty() {
                94.0
            } else {
                rows.len() as f32 * row_h
            };
            let (_, content_rect) =
                ui.allocate_space(Vec2::new(full_w, HEADER_H + body_h));
            let origin = content_rect.min;

            // Header row
            {
                let hrect = Rect::from_min_size(origin, Vec2::new(full_w, HEADER_H));
                let painter = ui.painter().clone();
                painter.rect_filled(hrect, Rounding::ZERO, p.grid_header);
                painter.line_segment(
                    [hrect.left_bottom(), hrect.right_bottom()],
                    Stroke::new(1.0_f32, p.border),
                );
                if style.show_row_numbers {
                    painter.text(
                        egui::pos2(hrect.left() + gutter_w - 12.0, hrect.center().y),
                        Align2::RIGHT_CENTER,
                        "#",
                        FontId::proportional(10.5),
                        p.faint,
                    );
                }
                let mut x = hrect.left() + gutter_w;
                for (i, name) in columns.iter().enumerate() {
                    let width = widths[i];
                    let header_rect = Rect::from_min_size(
                        egui::pos2(x, hrect.top()),
                        Vec2::new(width, HEADER_H),
                    );
                    let header =
                        ui.interact(header_rect, id.with(("header", i)), Sense::click());
                    if sortable && header.hovered() {
                        painter.rect_filled(header_rect, Rounding::ZERO, p.row_hover);
                    }
                    if header.is_pointer_button_down_on() {
                        focus.request_focus();
                    }
                    if header.clicked() && sortable {
                        apply_sort_click(
                            sort,
                            name,
                            ui.input(|input| input.modifiers.shift),
                        );
                        focus.request_focus();
                        action = GridAction::SortChanged;
                    }
                    let sorted = sort.iter().position(|spec| &spec.column == name);
                    let name_width = if sorted.is_some() {
                        width - 34.0
                    } else {
                        width - 30.0
                    };
                    painter.text(
                        egui::pos2(x + 10.0, hrect.center().y),
                        Align2::LEFT_CENTER,
                        fit_text(name, name_width),
                        FontId::proportional((font + 0.7).max(10.0)),
                        if sorted.is_some() { p.text } else { p.muted },
                    );
                    if sorted.is_none() && sortable {
                        // A permanent, faint up/down glyph tells the user the
                        // header is clickable before the first sort.
                        draw_sort_affordance(
                            &painter,
                            egui::pos2(x + width - 12.0, hrect.center().y),
                            if header.hovered() { p.muted } else { p.faint },
                        );
                    }
                    if let Some(index) = sorted {
                        draw_sort_triangle(
                            &painter,
                            egui::pos2(x + width - 9.0, hrect.center().y),
                            !sort[index].descending,
                            p.accent,
                        );
                        if sort.len() > 1 {
                            painter.text(
                                egui::pos2(x + width - 18.0, hrect.center().y),
                                Align2::RIGHT_CENTER,
                                (index + 1).to_string(),
                                FontId::monospace(8.0),
                                p.faint,
                            );
                        }
                    }
                    let header = if sortable {
                        header.on_hover_cursor(egui::CursorIcon::PointingHand)
                    } else {
                        header
                    };
                    if sortable {
                        let _ = header
                            .clone()
                            .on_hover_text("Click to sort · Shift-click to add a column");
                    }
                    x += width;
                    painter.line_segment(
                        [
                            egui::pos2(x, hrect.top() + 7.0),
                            egui::pos2(x, hrect.bottom() - 7.0),
                        ],
                        Stroke::new(1.0_f32, p.border),
                    );
                    // Drag the separator to resize; double-click auto-fits.
                    let sep_rect = Rect::from_min_size(
                        egui::pos2(x - 3.0, hrect.top()),
                        Vec2::new(6.0, HEADER_H),
                    );
                    let sep = ui.interact(sep_rect, id.with(("resize", i)), Sense::click_and_drag());
                    let sep = sep.on_hover_text("Drag to resize · double-click to auto-fit");
                    if sep.hovered() || sep.dragged() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    if sep.dragged() {
                        widths[i] = (widths[i] + sep.drag_delta().x).clamp(48.0, 900.0);
                        action = GridAction::WidthsChanged;
                    }
                    if sep.double_clicked() {
                        widths[i] = measure_column_width(ui, columns, rows, i, font);
                        action = GridAction::WidthsChanged;
                    }
                    header.context_menu(|ui| {
                        ui.set_min_width(184.0);
                        if sortable {
                            if ui.button("Sort ascending").clicked() {
                                sort.clear();
                                sort.push(SortSpec {
                                    column: name.clone(),
                                    descending: false,
                                });
                                action = GridAction::SortChanged;
                                close_context_menu(ui);
                            }
                            if ui.button("Sort descending").clicked() {
                                sort.clear();
                                sort.push(SortSpec {
                                    column: name.clone(),
                                    descending: true,
                                });
                                action = GridAction::SortChanged;
                                close_context_menu(ui);
                            }
                            if sorted.is_some() && ui.button("Clear sort").clicked() {
                                sort.retain(|spec| &spec.column != name);
                                action = GridAction::SortChanged;
                                close_context_menu(ui);
                            }
                            ui.separator();
                        }
                        if ui.button("Auto-fit column").clicked() {
                            widths[i] = measure_column_width(ui, columns, rows, i, font);
                            action = GridAction::WidthsChanged;
                            close_context_menu(ui);
                        }
                        if ui.button("Auto-fit all columns").clicked() {
                            *widths = measure_column_widths(ui, columns, rows, font);
                            action = GridAction::WidthsChanged;
                            close_context_menu(ui);
                        }
                        if ui.button("Reset widths").clicked() {
                            widths.clear();
                            action = GridAction::WidthsChanged;
                            close_context_menu(ui);
                        }
                    });
                }
            }

            if rows.is_empty() {
                let rect = Rect::from_min_size(
                    egui::pos2(origin.x, origin.y + HEADER_H),
                    Vec2::new(full_w, 94.0),
                );
                ui.painter().text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    empty_text,
                    FontId::proportional(12.0),
                    p.faint,
                );
                return;
            }

            // Keyboard operates only while the grid owns focus and no cell
            // editor is open (the editor handles its own keys).
            if edit.is_none() && focus.has_focus() && !ui.memory(|memory| memory.any_popup_open()) {
                let shift = ui.input(|input| input.modifiers.shift);
                let mods = if shift {
                    egui::Modifiers::SHIFT
                } else {
                    egui::Modifiers::NONE
                };
                let mut delta = (0isize, 0isize);
                if ui.input_mut(|input| input.consume_key(mods, egui::Key::ArrowDown)) {
                    delta = (1, 0);
                } else if ui.input_mut(|input| input.consume_key(mods, egui::Key::ArrowUp)) {
                    delta = (-1, 0);
                } else if ui.input_mut(|input| input.consume_key(mods, egui::Key::ArrowRight)) {
                    delta = (0, 1);
                } else if ui.input_mut(|input| input.consume_key(mods, egui::Key::ArrowLeft)) {
                    delta = (0, -1);
                }
                let tab = ui.input_mut(|input| input.consume_key(mods, egui::Key::Tab));
                if (delta != (0, 0) || tab) && !rows.is_empty() && !columns.is_empty() {
                    {
                        let (mut row, mut col) = selection.cursor().unwrap_or((0, 0));
                        row = row.min(rows.len() - 1);
                        col = col.min(columns.len() - 1);
                        if tab {
                            if shift {
                                if col > 0 {
                                    col -= 1;
                                } else if row > 0 {
                                    row -= 1;
                                    col = columns.len() - 1;
                                }
                            } else if col + 1 < columns.len() {
                                col += 1;
                            } else if row + 1 < rows.len() {
                                row += 1;
                                col = 0;
                            }
                        } else {
                            row = (row as isize + delta.0)
                                .clamp(0, rows.len() as isize - 1)
                                as usize;
                            col = (col as isize + delta.1)
                                .clamp(0, columns.len() as isize - 1)
                                as usize;
                        }
                        if shift && !tab {
                            selection.extend_to(row, col);
                        } else {
                            selection.set_cell(row, col);
                        }
                        scroll_to_cursor = true;
                    }
                }
                if can_edit
                    && (ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::Enter)
                    }) || ui.input_mut(|input| {
                        input.consume_key(egui::Modifiers::NONE, egui::Key::F2)
                    }))
                {
                    if let Some((row, col)) = selection.cursor() {
                        action = GridAction::OpenEditor(row, col);
                    }
                }
                if ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::A)
                }) {
                    // Select everything without jumping the viewport; the
                    // cursor lands on the last cell by convention.
                    selection.select_all(rows.len(), columns.len());
                }
                if ui.input_mut(|input| {
                    input.consume_key(egui::Modifiers::COMMAND, egui::Key::C)
                }) {
                    if let Some(text) = selection.to_tsv(columns, rows) {
                        action = GridAction::CopyText(text);
                    }
                }
            }

            // One row of overscan on each side keeps fast scrolling free of
            // blank bands while still painting only what is visible.
            let rows_top = origin.y + HEADER_H;
            for r in visible_row_range(viewport, rows_top, row_h, rows.len()) {
                let row = &rows[r];
                let rrect = Rect::from_min_size(
                    egui::pos2(origin.x, rows_top + r as f32 * row_h),
                    Vec2::new(full_w, row_h),
                );
                let painter = ui.painter().clone();
                if style.alternate_rows && r % 2 == 1 {
                    painter.rect_filled(rrect, Rounding::ZERO, p.row_alt);
                }
                painter.line_segment(
                    [rrect.left_bottom(), rrect.right_bottom()],
                    Stroke::new(1.0_f32, p.border.gamma_multiply(0.45)),
                );

                // Row-number gutter: clicking it selects the whole row, and
                // Shift-click extends a row range.
                let gutter_rect =
                    Rect::from_min_size(rrect.min, Vec2::new(gutter_w, row_h));
                let gutter =
                    ui.interact(gutter_rect, id.with(("gutter", r)), Sense::click());
                let row_selected = selection.row_selected(r);
                if row_selected {
                    painter.rect_filled(gutter_rect, Rounding::ZERO, p.selection);
                }
                if style.show_row_numbers {
                    painter.text(
                        egui::pos2(rrect.left() + gutter_w - 12.0, rrect.center().y),
                        Align2::RIGHT_CENTER,
                        (row_offset + r + 1).to_string(),
                        FontId::monospace((font - 0.8).max(8.5)),
                        if row_selected { p.accent } else { p.faint },
                    );
                }
                if gutter.is_pointer_button_down_on() {
                    focus.request_focus();
                }
                if gutter.clicked() {
                    focus.request_focus();
                    if ui.input(|input| input.modifiers.shift) {
                        selection.extend_row_to(r, columns.len());
                    } else {
                        selection.select_row(r, columns.len());
                    }
                }

                let mut x = rrect.left() + gutter_w;
                for c in 0..columns.len() {
                    let width = widths[c];
                    let cell =
                        Rect::from_min_size(egui::pos2(x, rrect.top()), Vec2::new(width, row_h));
                    let resp = ui.interact(cell, egui::Id::new(("grid-cell", id, r, c)), Sense::click());
                    // Inline editor: the TextEdit replaces the painted text in
                    // this cell until it is committed or cancelled.
                    if let Some(editing) = edit.as_deref_mut() {
                        if editing.row == r && editing.col == c {
                            painter.rect_filled(cell, Rounding::ZERO, p.editor);
                            painter.rect_stroke(
                                cell.shrink(0.5),
                                Rounding::ZERO,
                                Stroke::new(2.0_f32, p.accent),
                            );
                            let mut editor_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(cell.shrink2(Vec2::new(2.0, 1.0)))
                                    .layout(Layout::left_to_right(Align::Center)),
                            );
                            match &editing.kind {
                                crate::db::FieldEditor::Bool => {
                                    let mut checked = is_truthy(editing.text.as_str());
                                    let resp = editor_ui.add(egui::Checkbox::new(&mut checked, ""));
                                    if editing.focus {
                                        resp.request_focus();
                                        editing.focus = false;
                                    }
                                    if resp.changed() {
                                        action = GridAction::SetValue(
                                            r,
                                            c,
                                            Some(if checked { "1" } else { "0" }.to_string()),
                                        );
                                        focus.request_focus();
                                    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        action = GridAction::Cancel;
                                        focus.request_focus();
                                    }
                                    x += width;
                                    continue;
                                }
                                crate::db::FieldEditor::Enum(options) => {
                                    let current = if editing.text.is_empty() {
                                        "—".to_string()
                                    } else {
                                        editing.text.clone()
                                    };
                                    let resp = egui::ComboBox::from_id_salt((
                                        "cell-enum", r, c,
                                    ))
                                    .selected_text(
                                        RichText::new(current).size(font).color(p.text),
                                    )
                                    .width((width - 12.0).max(60.0))
                                    .show_ui(&mut editor_ui, |ui| {
                                        for option in options {
                                            ui.selectable_value(
                                                &mut editing.text,
                                                option.clone(),
                                                option,
                                            );
                                        }
                                    });
                                    if resp.response.changed() {
                                        action = GridAction::SetValue(
                                            r,
                                            c,
                                            Some(editing.text.clone()),
                                        );
                                        focus.request_focus();
                                    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        action = GridAction::Cancel;
                                        focus.request_focus();
                                    }
                                    x += width;
                                    continue;
                                }
                                crate::db::FieldEditor::Json
                                | crate::db::FieldEditor::Blob => {
                                    editor_ui.add(
                                        TextEdit::singleline(&mut editing.text)
                                            .id(id.with(("cell-editor", r, c)))
                                            .font(FontId::proportional(font)),
                                    );
                                    let expand = egui::Button::new("⤢").min_size(Vec2::splat(16.0));
                                    if editor_ui.add(expand).clicked() {
                                        action = GridAction::ExpandValue(r, c);
                                        focus.request_focus();
                                    } else if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                        action = GridAction::Cancel;
                                        focus.request_focus();
                                    }
                                    x += width;
                                    continue;
                                }
                                _ => {}
                            }
                            let text_resp = editor_ui.add(
                                TextEdit::singleline(&mut editing.text)
                                    .id(id.with(("cell-editor", r, c)))
                                    .font(FontId::proportional(font))
                                    .hint_text(if editing.original.is_none() {
                                        "NULL"
                                    } else {
                                        ""
                                    }),
                            );
                            if editing.focus {
                                text_resp.request_focus();
                                editing.focus = false;
                            }
                            let mut picker_open = false;
                            if matches!(
                                editing.kind,
                                crate::db::FieldEditor::Date
                                    | crate::db::FieldEditor::DateTime
                            ) {
                                let button = egui::Button::new("▾").min_size(Vec2::splat(15.0));
                                if editor_ui.add(button).clicked() {
                                    picker_open = true;
                                }
                            }
                            let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
                            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            let touched = text_resp.has_focus() || text_resp.lost_focus();
                            action = if touched && escape {
                                GridAction::Cancel
                            } else if touched && (enter || text_resp.lost_focus()) {
                                GridAction::Commit
                            } else {
                                GridAction::None
                            };
                            if matches!(action, GridAction::Commit | GridAction::Cancel) {
                                focus.request_focus();
                            }
                            if picker_open {
                                let popup_id = id.with(("date-popup", r, c));
                                ui.memory_mut(|memory| memory.toggle_popup(popup_id));
                                let anchor = ui.interact(
                                    cell,
                                    id.with(("date-anchor", r, c)),
                                    Sense::hover(),
                                );
                                egui::popup_below_widget(
                                    ui,
                                    popup_id,
                                    &anchor,
                                    egui::PopupCloseBehavior::CloseOnClickOutside,
                                    |ui| {
                                        ui.set_min_width(228.0);
                                        if calendar_ui(ui, p, &mut editing.text) {
                                            action = GridAction::Commit;
                                            ui.memory_mut(|memory| memory.close_popup());
                                        }
                                    },
                                );
                            }
                            x += width;
                            continue;
                        }
                    }

                    let is_cursor = selection.cursor() == Some((r, c));
                    if selection.contains(r, c) {
                        painter.rect_filled(
                            cell,
                            Rounding::ZERO,
                            if is_cursor {
                                p.selection
                            } else {
                                p.selection.gamma_multiply(0.65)
                            },
                        );
                    } else if resp.hovered() {
                        painter.rect_filled(cell, Rounding::ZERO, p.row_hover);
                    }
                    if is_cursor {
                        painter.rect_stroke(
                            cell.shrink(0.5),
                            Rounding::ZERO,
                            Stroke::new(1.0_f32, p.accent),
                        );
                    }
                    let value = row.get(c).and_then(|v| v.as_ref());
                    let (text, color) = match value {
                        Some(v) => (v.as_str(), p.text),
                        None => ("NULL", p.faint),
                    };
                    // Numbers read better right-aligned, like most SQL clients.
                    let numeric = column_types
                        .get(c)
                        .map(|decl| {
                            let decl = decl.to_ascii_lowercase();
                            decl.contains("int")
                                || decl.contains("real")
                                || decl.contains("float")
                                || decl.contains("double")
                                || decl.contains("decimal")
                                || decl.contains("numeric")
                        })
                        .unwrap_or(false)
                        && value.is_some();
                    if numeric {
                        painter.text(
                            egui::pos2(x + width - 10.0, cell.center().y),
                            Align2::RIGHT_CENTER,
                            fit_text_measured(ui, text, width - 20.0, font),
                            FontId::proportional(font),
                            color,
                        );
                    } else {
                        painter.text(
                            egui::pos2(x + 10.0, cell.center().y),
                            Align2::LEFT_CENTER,
                            fit_text_measured(ui, text, width - 20.0, font),
                            FontId::proportional(font),
                            color,
                        );
                    }
                    if resp.is_pointer_button_down_on() {
                        focus.request_focus();
                    }
                    if resp.secondary_clicked() {
                        // Keep a multi-cell selection when right-clicking
                        // inside it, so "Copy selection" copies the range
                        // instead of collapsing to one cell.
                        focus.request_focus();
                        if !selection.contains(r, c) {
                            selection.set_cell(r, c);
                        }
                    } else if resp.clicked() {
                        focus.request_focus();
                        if ui.input(|input| input.modifiers.shift) {
                            selection.extend_to(r, c);
                        } else {
                            selection.set_cell(r, c);
                        }
                    }
                    if resp.double_clicked() && can_edit {
                        focus.request_focus();
                        selection.set_cell(r, c);
                        action = GridAction::OpenEditor(r, c);
                    }
                    // Right-click menu: the grid's equivalent of the row
                    // detail panel's Set NULL / Set empty / Copy menu.
                    let hovered = resp.hovered();
                    if can_edit {
                        resp.context_menu(|ui| {
                            ui.set_min_width(150.0);
                            if ui.button("Edit cell").clicked() {
                                action = GridAction::OpenEditor(r, c);
                                close_context_menu(ui);
                            }
                            let nullable = null_allowed.get(c).copied().unwrap_or(true);
                            if ui
                                .add_enabled(nullable, egui::Button::new("Set NULL"))
                                .clicked()
                            {
                                action = GridAction::SetNull(r, c);
                                close_context_menu(ui);
                            }
                            if ui.button("Set empty").clicked() {
                                action = GridAction::SetEmpty(r, c);
                                close_context_menu(ui);
                            }
                            ui.separator();
                            if ui.button("Copy value").clicked() {
                                action =
                                    GridAction::CopyText(value.cloned().unwrap_or_default());
                                close_context_menu(ui);
                            }
                            if !selection.is_empty()
                                && ui.button("Copy selection").clicked()
                            {
                                if let Some(text) = selection.to_tsv(columns, rows) {
                                    action = GridAction::CopyText(text);
                                }
                                close_context_menu(ui);
                            }
                        });
                    } else {
                        resp.context_menu(|ui| {
                            ui.set_min_width(150.0);
                            if ui.button("Copy value").clicked() {
                                action =
                                    GridAction::CopyText(value.cloned().unwrap_or_default());
                                close_context_menu(ui);
                            }
                            if !selection.is_empty()
                                && ui.button("Copy selection").clicked()
                            {
                                if let Some(text) = selection.to_tsv(columns, rows) {
                                    action = GridAction::CopyText(text);
                                }
                                close_context_menu(ui);
                            }
                        });
                    }
                    if hovered {
                        resp.on_hover_text(text);
                    }
                    x += width;
                }
            }

            if scroll_to_cursor {
                if let Some((row, col)) = selection.cursor() {
                    if row < rows.len() && col < columns.len() {
                        let x = origin.x
                            + gutter_w
                            + widths[..col].iter().sum::<f32>();
                        let y = rows_top + row as f32 * row_h;
                        let rect = Rect::from_min_size(
                            egui::pos2(x, y),
                            Vec2::new(widths[col], row_h),
                        );
                        if !viewport.contains_rect(rect) {
                            ui.scroll_to_rect(rect, Some(Align::Center));
                        }
                    }
                }
            }
        });
    *scroll = output.state.offset.y;

    action
}

/// Applies a header click: a plain click cycles one column (asc → desc → no
/// sort); Shift-click adds/toggles extra columns for a multi-column sort.
fn apply_sort_click(sort: &mut Vec<SortSpec>, column: &str, extend: bool) {
    if extend {
        if let Some(index) = sort.iter().position(|spec| spec.column == column) {
            if sort[index].descending {
                sort.remove(index);
            } else {
                sort[index].descending = true;
            }
        } else {
            sort.push(SortSpec {
                column: column.to_string(),
                descending: false,
            });
        }
    } else {
        let current = sort
            .first()
            .filter(|spec| spec.column == column)
            .map(|spec| spec.descending);
        sort.clear();
        match current {
            None => sort.push(SortSpec {
                column: column.to_string(),
                descending: false,
            }),
            Some(false) => sort.push(SortSpec {
                column: column.to_string(),
                descending: true,
            }),
            Some(true) => {}
        }
    }
}

/// Two tiny stacked triangles shown on unsorted headers: the permanent hint
/// that a click sorts by that column.
fn draw_sort_affordance(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    for (offset, up) in [(-3.0, true), (3.0, false)] {
        let c = egui::pos2(center.x, center.y + offset);
        let half = 2.6;
        let points = if up {
            vec![
                egui::pos2(c.x, c.y - half),
                egui::pos2(c.x - half, c.y + half),
                egui::pos2(c.x + half, c.y + half),
            ]
        } else {
            vec![
                egui::pos2(c.x, c.y + half),
                egui::pos2(c.x - half, c.y - half),
                egui::pos2(c.x + half, c.y - half),
            ]
        };
        painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
    }
}

/// Small filled triangle for the header sort indicator. Drawing it instead of
/// relying on a font glyph keeps it visible with every bundled font.
fn draw_sort_triangle(painter: &egui::Painter, center: egui::Pos2, up: bool, color: Color32) {
    let half_h = 3.5;
    let half_w = 3.0;
    let points = if up {
        vec![
            egui::pos2(center.x, center.y - half_h),
            egui::pos2(center.x - half_w, center.y + half_h),
            egui::pos2(center.x + half_w, center.y + half_h),
        ]
    } else {
        vec![
            egui::pos2(center.x, center.y + half_h),
            egui::pos2(center.x - half_w, center.y - half_h),
            egui::pos2(center.x + half_w, center.y - half_h),
        ]
    };
    painter.add(egui::Shape::convex_polygon(points, color, Stroke::NONE));
}

/// Rows that intersect the scroll area viewport, with one row of overscan.
/// `viewport` and `rows_top` are in the content's coordinate space.
fn visible_row_range(
    viewport: Rect,
    rows_top: f32,
    row_height: f32,
    rows_len: usize,
) -> std::ops::Range<usize> {
    if rows_len == 0 || row_height <= 0.0 {
        return 0..0;
    }
    let first = ((((viewport.min.y - rows_top) / row_height).floor() as isize) - 1)
        .max(0) as usize;
    let first = first.min(rows_len - 1);
    let last = ((((viewport.max.y - rows_top) / row_height).ceil() as isize) + 1)
        .max(0) as usize;
    let last = last.min(rows_len).max(first + 1);
    first..last
}

fn measure_text_width(ui: &Ui, text: &str, size: f32) -> f32 {
    ui.fonts(|fonts| {
        fonts
            .layout_no_wrap(
                text.to_owned(),
                FontId::proportional(size),
                Color32::WHITE,
            )
            .size()
            .x
    })
}

/// Width for one column: the widest of the header and a sample of the loaded
/// rows, measured with the real font so resizing and auto-fit agree with what
/// is painted.
fn measure_column_width(
    ui: &Ui,
    columns: &[String],
    rows: &[Vec<Option<String>>],
    index: usize,
    font_size: f32,
) -> f32 {
    let mut max = columns
        .get(index)
        .map(|name| measure_text_width(ui, name, (font_size + 0.7).max(10.0)) + 34.0)
        .unwrap_or(72.0);
    for row in rows.iter().take(120) {
        if let Some(Some(value)) = row.get(index) {
            let first_line = value.lines().next().unwrap_or("");
            let width = measure_text_width(ui, first_line, font_size) + 28.0;
            max = max.max(width.min(520.0));
        }
    }
    max.clamp(64.0, 520.0)
}

fn measure_column_widths(
    ui: &Ui,
    columns: &[String],
    rows: &[Vec<Option<String>>],
    font_size: f32,
) -> Vec<f32> {
    (0..columns.len())
        .map(|index| measure_column_width(ui, columns, rows, index, font_size))
        .collect()
}

/// Fits text into a cell using the real font: the full value is measured once
/// and, when it overflows, the longest prefix that fits with an ellipsis is
/// found by binary search. This keeps emails and long identifiers from being
/// cut short by a hard-coded characters-per-pixel estimate.
fn fit_text_measured(ui: &Ui, text: &str, width: f32, font_size: f32) -> String {
    let single = text.replace(['\n', '\t', '\r'], " ");
    if width <= 8.0 || single.is_empty() {
        return single;
    }
    let measure = |value: &str| -> f32 {
        ui.fonts(|fonts| {
            fonts
                .layout_no_wrap(
                    value.to_owned(),
                    FontId::proportional(font_size),
                    Color32::WHITE,
                )
                .size()
                .x
        })
    };
    if measure(&single) <= width {
        return single;
    }
    let chars: Vec<char> = single.chars().collect();
    let mut low = 1usize;
    let mut high = chars.len();
    let mut best = 1usize;
    while low <= high {
        let mid = (low + high) / 2;
        let candidate: String = chars[..mid].iter().collect::<String>() + "…";
        if measure(&candidate) <= width {
            best = mid;
            low = mid + 1;
        } else {
            high = mid.saturating_sub(1);
        }
    }
    let mut out: String = chars[..best.min(chars.len())].iter().collect();
    out.push('…');
    out
}

fn fit_text(text: &str, width: f32) -> String {
    let max = ((width / 6.9).floor() as usize).max(2);
    if text.chars().count() <= max {
        return text.replace(['\n', '\t', '\r'], " ");
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out.replace(['\n', '\t', '\r'], " ")
}

// ----------------------------------------------------------------------
// SQL highlighting
// ----------------------------------------------------------------------

const KEYWORDS: &[&str] = &[
    "SELECT",
    "FROM",
    "WHERE",
    "GROUP",
    "BY",
    "ORDER",
    "HAVING",
    "LIMIT",
    "OFFSET",
    "JOIN",
    "LEFT",
    "RIGHT",
    "INNER",
    "OUTER",
    "FULL",
    "CROSS",
    "ON",
    "AS",
    "AND",
    "OR",
    "NOT",
    "IN",
    "IS",
    "NULL",
    "LIKE",
    "GLOB",
    "BETWEEN",
    "CASE",
    "WHEN",
    "THEN",
    "ELSE",
    "END",
    "DISTINCT",
    "UNION",
    "ALL",
    "INSERT",
    "INTO",
    "VALUES",
    "UPDATE",
    "SET",
    "DELETE",
    "CREATE",
    "TABLE",
    "VIEW",
    "INDEX",
    "DROP",
    "ALTER",
    "ADD",
    "COLUMN",
    "PRIMARY",
    "KEY",
    "FOREIGN",
    "REFERENCES",
    "UNIQUE",
    "DEFAULT",
    "CHECK",
    "PRAGMA",
    "BEGIN",
    "COMMIT",
    "ROLLBACK",
    "TRANSACTION",
    "EXPLAIN",
    "WITH",
    "RECURSIVE",
    "ASC",
    "DESC",
    "EXISTS",
    "COUNT",
    "SUM",
    "AVG",
    "MIN",
    "MAX",
    "ROUND",
    "CAST",
    "COALESCE",
    "IFNULL",
    "TRUE",
    "FALSE",
];

pub(crate) fn highlight_sql(text: &str, p: &Palette) -> egui::text::LayoutJob {
    use egui::text::{LayoutJob, TextFormat};

    let font = FontId::monospace(theme::SQL_FONT_SIZE);
    let normal = TextFormat {
        font_id: font.clone(),
        line_height: Some(theme::EDITOR_LINE_HEIGHT),
        color: p.text,
        ..Default::default()
    };
    let keyword = TextFormat {
        font_id: font.clone(),
        line_height: Some(theme::EDITOR_LINE_HEIGHT),
        color: p.accent,
        ..Default::default()
    };
    let function = TextFormat {
        font_id: font.clone(),
        line_height: Some(theme::EDITOR_LINE_HEIGHT),
        color: p.purple,
        ..Default::default()
    };
    let string = TextFormat {
        font_id: font.clone(),
        line_height: Some(theme::EDITOR_LINE_HEIGHT),
        color: p.code,
        ..Default::default()
    };
    let number = TextFormat {
        font_id: font.clone(),
        line_height: Some(theme::EDITOR_LINE_HEIGHT),
        color: p.amber,
        ..Default::default()
    };
    let comment = TextFormat {
        font_id: font.clone(),
        line_height: Some(theme::EDITOR_LINE_HEIGHT),
        color: p.faint,
        italics: true,
        ..Default::default()
    };

    let mut job = LayoutJob::default();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        if c == '-' && next == Some('-') {
            let start = i;
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            job.append(&s, 0.0, comment.clone());
            continue;
        }
        if c == '/' && next == Some('*') {
            let start = i;
            i += 2;
            while i < chars.len() {
                if chars[i] == '*' && chars.get(i + 1) == Some(&'/') {
                    i += 2;
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i.min(chars.len())].iter().collect();
            job.append(&s, 0.0, comment.clone());
            continue;
        }
        if c == '\'' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '\'' {
                    if chars.get(i + 1) == Some(&'\'') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i.min(chars.len())].iter().collect();
            job.append(&s, 0.0, string.clone());
            continue;
        }
        if c == '"' {
            let start = i;
            i += 1;
            while i < chars.len() {
                if chars[i] == '"' {
                    if chars.get(i + 1) == Some(&'"') {
                        i += 2;
                        continue;
                    }
                    i += 1;
                    break;
                }
                i += 1;
            }
            let s: String = chars[start..i.min(chars.len())].iter().collect();
            job.append(&s, 0.0, function.clone());
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            while i < chars.len()
                && (chars[i].is_ascii_digit()
                    || chars[i] == '.'
                    || chars[i] == 'e'
                    || chars[i] == 'E')
            {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            job.append(&s, 0.0, number.clone());
            continue;
        }
        if c.is_alphabetic() || c == '_' {
            let start = i;
            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_') {
                i += 1;
            }
            let word: String = chars[start..i].iter().collect();
            let upper = word.to_ascii_uppercase();
            if KEYWORDS.contains(&upper.as_str()) {
                let fmt = if matches!(
                    upper.as_str(),
                    "COUNT"
                        | "SUM"
                        | "AVG"
                        | "MIN"
                        | "MAX"
                        | "ROUND"
                        | "CAST"
                        | "COALESCE"
                        | "IFNULL"
                ) {
                    function.clone()
                } else {
                    keyword.clone()
                };
                job.append(&word, 0.0, fmt);
            } else {
                job.append(&word, 0.0, normal.clone());
            }
            continue;
        }
        if c.is_whitespace() {
            let start = i;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            let s: String = chars[start..i].iter().collect();
            job.append(&s, 0.0, normal.clone());
            continue;
        }
        let mut buf = [0u8; 4];
        job.append(c.encode_utf8(&mut buf), 0.0, normal.clone());
        i += 1;
    }
    job
}

/// Pretty-prints JSON for the value window; invalid JSON is returned as-is so
/// the user can still edit it.
pub(crate) fn pretty_json(text: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(text) {
        Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_else(|_| text.to_string()),
        Err(_) => text.to_string(),
    }
}

/// Truthiness for boolean columns across engines: SQLite stores 1/0,
/// PostgreSQL accepts t/f, true/false, yes/no and on/off.
pub(crate) fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "t" | "true" | "yes" | "on" | "-1"
    )
}

/// Decodes a blob cell back into bytes. SQLite blobs arrive as `0x…` hex;
/// other drivers hand over text that may or may not be binary, so its UTF-8
/// bytes are the best available preview.
pub(crate) fn blob_bytes(text: &str) -> Vec<u8> {
    let text = text.trim();
    if let Some(hex) = text.strip_prefix("0x") {
        let hex: String = hex.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
        let mut out = Vec::with_capacity(hex.len() / 2);
        let chars: Vec<char> = hex.chars().collect();
        for pair in chars.chunks(2) {
            if pair.len() == 2 {
                let byte = u8::from_str_radix(&format!("{}{}", pair[0], pair[1]), 16);
                if let Ok(byte) = byte {
                    out.push(byte);
                }
            }
        }
        return out;
    }
    text.as_bytes().to_vec()
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// `00000000  48 65 6c 6c 6f 2c 20 77  6f 72 6c 64 21           |Hello, world!|`
pub(crate) fn hex_dump(bytes: &[u8]) -> String {
    let mut out = String::new();
    for (row, chunk) in bytes.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", row * 16));
        for i in 0..16 {
            if let Some(byte) = chunk.get(i) {
                out.push_str(&format!("{byte:02x} "));
            } else {
                out.push_str("   ");
            }
            if i == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        for byte in chunk {
            let c = *byte as char;
            out.push(if c.is_ascii_graphic() || c == ' ' { c } else { '.' });
        }
        out.push_str("|\n");
    }
    out
}

pub(crate) fn blob_text_preview(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out: String = text
        .chars()
        .map(|c| if c.is_control() && c != '\n' && c != '\t' { '.' } else { c })
        .take(2000)
        .collect();
    if text.chars().count() > 2000 {
        out.push('…');
    }
    out
}

fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year } as i64;
    let era = (if year >= 0 { year } else { year - 399 }) / 400;
    let yoe = year - era * 400;
    let month = month as i64;
    let doy = (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Parses the date part of `YYYY-MM-DD` or `YYYY-MM-DD HH:MM:SS`.
pub(crate) fn parse_date(text: &str) -> Option<(i32, u32, u32)> {
    let date = text.trim().split(' ').next()?;
    let mut parts = date.split('-');
    let year = parts.next()?.parse::<i32>().ok()?;
    let month = parts.next()?.parse::<u32>().ok()?;
    let day = parts.next()?.parse::<u32>().ok()?;
    if (1..=12).contains(&month) && (1..=31).contains(&day) {
        Some((year, month, day))
    } else {
        None
    }
}

/// Small month calendar. Clicking a day writes `YYYY-MM-DD` into `text`,
/// keeping the time part when the field already had one.
pub(crate) fn calendar_ui(ui: &mut Ui, p: &Palette, text: &mut String) -> bool {
    let today = crate::db::now_parts();
    let (parsed_year, parsed_month, parsed_day) =
        parse_date(text).unwrap_or((today.0, today.1, today.2));
    // The visible month is UI state so the arrows work before a date is picked.
    let state_id = ui.id().with("calendar-month");
    let stored: i32 = ui
        .data(|data| data.get_temp(state_id))
        .unwrap_or(parsed_year * 12 + parsed_month as i32 - 1);
    let mut index = stored;
    let mut picked: Option<(i32, u32, u32)> = None;

    ui.horizontal(|ui| {
        if ui.small_button("‹").clicked() {
            index -= 1;
        }
        let year = index.div_euclid(12);
        let month = (index.rem_euclid(12) + 1) as u32;
        ui.add_space(4.0);
        ui.label(
            RichText::new(format!("{} {}", month_name(month), year))
                .size(12.0)
                .strong()
                .color(p.text),
        );
        ui.add_space(4.0);
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button("›").clicked() {
                index += 1;
            }
        });
    });
    ui.data_mut(|data| data.insert_temp(state_id, index));

    let year = index.div_euclid(12);
    let month = (index.rem_euclid(12) + 1) as u32;
    egui::Grid::new(ui.id().with("calendar-grid"))
        .num_columns(7)
        .spacing(Vec2::new(2.0, 2.0))
        .show(ui, |ui| {
            for name in ["Mo", "Tu", "We", "Th", "Fr", "Sa", "Su"] {
                ui.label(RichText::new(name).size(9.6).color(p.faint));
            }
            ui.end_row();
            let first = days_from_civil(year, month, 1);
            // Monday-first weekday index.
            let lead = ((first + 3).rem_euclid(7)) as u32;
            let mut column = 0;
            for _ in 0..lead {
                ui.label("");
                column += 1;
            }
            for day in 1..=days_in_month(year, month) {
                let selected = year == parsed_year && month == parsed_month && day == parsed_day;
                let is_today = year == today.0 && month == today.1 && day == today.2;
                let button = egui::Button::new(
                    RichText::new(day.to_string()).size(10.8).color(if selected {
                        p.accent
                    } else if is_today {
                        p.green
                    } else {
                        p.text
                    }),
                )
                .fill(if selected { p.accent_soft } else { Color32::TRANSPARENT })
                .stroke(Stroke::NONE)
                .min_size(Vec2::splat(22.0));
                if ui.add(button).clicked() {
                    picked = Some((year, month, day));
                }
                column += 1;
                if column % 7 == 0 {
                    ui.end_row();
                }
            }
        });
    ui.add_space(2.0);
    if ui.small_button("Today").clicked() {
        picked = Some((today.0, today.1, today.2));
    }
    let mut changed = false;
    if let Some((year, month, day)) = picked {
        let time = text
            .split_once(' ')
            .map(|(_, time)| time.to_string())
            .filter(|time| !time.is_empty());
        let next = match time {
            Some(time) => format!("{year:04}-{month:02}-{day:02} {time}"),
            None => format!("{year:04}-{month:02}-{day:02}"),
        };
        if next != *text {
            *text = next;
            changed = true;
        }
    }
    changed
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        _ => "December",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    fn raw_input(events: Vec<egui::Event>) -> egui::RawInput {
        raw_input_with(events, egui::Modifiers::NONE)
    }

    fn raw_input_with(
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> egui::RawInput {
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(900.0, 500.0),
            )),
            modifiers,
            events,
            ..Default::default()
        }
    }

    fn run_grid(
        ctx: &egui::Context,
        selection: &mut GridSelection,
        sort: &mut Vec<SortSpec>,
    ) -> GridAction {
        let p = crate::theme::Palette::light();
        let columns = vec!["id".to_string(), "name".to_string()];
        let rows = vec![
            vec![Some("1".to_string()), Some("A".to_string())],
            vec![Some("2".to_string()), Some("B".to_string())],
        ];
        let mut action = GridAction::None;
        let mut widths = Vec::new();
        egui::CentralPanel::default().show(ctx, |ui| {
                ui.set_min_size(egui::vec2(600.0, 400.0));
                action = data_grid(
                    ui,
                    &p,
                    egui::Id::new("another-grid"),
                    &columns,
                    &rows,
                    selection,
                    0,
                    "empty",
                    None,
                    true,
                    &[true, true],
                    sort,
                    true,
                    GridStyle {
                        row_height: theme::GRID_ROW_HEIGHT,
                        font_size: theme::DATA_FONT_SIZE,
                        alternate_rows: true,
                        show_row_numbers: true,
                    },
                    &mut widths,
                    &[],
                    &mut 0.0,
                    None,
                );
            });
        action
    }

    fn pointer(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        }
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }

    fn key_up(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers,
        }
    }

    fn click(ctx: &egui::Context, selection: &mut GridSelection, sort: &mut Vec<SortSpec>, pos: egui::Pos2) {
        let _ = ctx.run(raw_input(vec![egui::Event::PointerMoved(pos)]), |ctx| {
            let _ = run_grid(ctx, selection, sort);
        });
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos), pointer(pos, true)]),
            |ctx| {
                let _ = run_grid(ctx, selection, sort);
            },
        );
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos), pointer(pos, false)]),
            |ctx| {
                let _ = run_grid(ctx, selection, sort);
            },
        );
    }

    fn run_grid_rows(
        ctx: &egui::Context,
        selection: &mut GridSelection,
        sort: &mut Vec<SortSpec>,
        rows_len: usize,
        height: f32,
    ) -> GridAction {
        let p = crate::theme::Palette::light();
        let columns = vec!["id".to_string(), "name".to_string()];
        let rows: Vec<Vec<Option<String>>> = (0..rows_len)
            .map(|index| {
                vec![
                    Some(index.to_string()),
                    Some(format!("row {index}")),
                ]
            })
            .collect();
        let mut action = GridAction::None;
        let mut widths = Vec::new();
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(600.0, height),
                egui::Layout::top_down(egui::Align::Min),
                |ui| {
                    action = data_grid(
                        ui,
                        &p,
                        egui::Id::new("scroll-grid"),
                        &columns,
                        &rows,
                        selection,
                        0,
                        "empty",
                        None,
                        true,
                        &[true, true],
                        sort,
                        true,
                        GridStyle {
                            row_height: theme::GRID_ROW_HEIGHT,
                            font_size: theme::DATA_FONT_SIZE,
                            alternate_rows: true,
                            show_row_numbers: true,
                        },
                        &mut widths,
                        &[],
                        &mut 0.0,
                        None,
                    );
                },
            );
        });
        action
    }

    #[test]
    fn boolean_truthiness_matches_every_engine() {
        for value in ["1", "t", "TRUE", "yes", "on"] {
            assert!(is_truthy(value), "{value}");
        }
        for value in ["0", "f", "false", "", "NULL"] {
            assert!(!is_truthy(value), "{value}");
        }
    }

    #[test]
    fn blob_hex_roundtrip_and_previews() {
        let bytes = b"Hello, world!";
        let hex = format!("0x{}", hex_encode(bytes));
        assert_eq!(blob_bytes(&hex), bytes);
        let dump = hex_dump(bytes);
        assert!(dump.starts_with("00000000  48 65 6c 6c 6f"));
        assert!(dump.contains("|Hello, world!|"));
        assert_eq!(blob_text_preview(bytes), "Hello, world!");
        // Plain text blobs from MySQL are previewed as their own bytes.
        assert_eq!(blob_bytes("plain"), b"plain");
    }

    #[test]
    fn json_pretty_print_keeps_invalid_input() {
        assert_eq!(pretty_json(r#"{"a":1}"#), "{\n  \"a\": 1\n}");
        assert_eq!(pretty_json("not json"), "not json");
    }

    #[test]
    fn calendar_parses_and_keeps_time() {
        assert_eq!(parse_date("2024-02-29 10:11:12"), Some((2024, 2, 29)));
        assert_eq!(parse_date("2024-13-01"), None);
        assert_eq!(parse_date("not a date"), None);
        assert_eq!(days_in_month(2024, 2), 29);
        assert_eq!(days_in_month(2023, 2), 28);
    }

    #[test]
    fn scrolling_reaches_rows_beyond_the_first_page() {
        let ctx = egui::Context::default();
        let mut selection = GridSelection::default();
        let mut sort = Vec::new();

        // Small viewport: only about four rows are visible at a time.
        let _ = ctx.run(raw_input(vec![]), |ctx| {
            let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
        });
        let pos = egui::pos2(70.0, 60.0);
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos)]),
            |ctx| {
                let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
            },
        );
        let _ = ctx.run(
            raw_input(vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, 400.0),
                    modifiers: egui::Modifiers::default(),
                },
            ]),
            |ctx| {
                let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
            },
        );
        // Let the smooth scroll settle.
        for _ in 0..12 {
            let _ = ctx.run(raw_input(vec![]), |ctx| {
                let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
            });
        }
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos)]),
            |ctx| {
                let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
            },
        );
        let _ = ctx.run(
            raw_input(vec![
                egui::Event::PointerMoved(pos),
                pointer(pos, true),
            ]),
            |ctx| {
                let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
            },
        );
        let _ = ctx.run(
            raw_input(vec![
                egui::Event::PointerMoved(pos),
                pointer(pos, false),
            ]),
            |ctx| {
                let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
            },
        );
        let row = selection.cursor().map(|(row, _)| row).unwrap_or(0);
        assert!(row > 0, "the wheel did not scroll the virtualized grid");

        // The focus holder is scrolled out of view, but clicking a visible
        // row must still leave the grid keyboard-navigable, and moving past
        // the bottom must scroll the next rows into view.
        for _ in 0..12 {
            let _ = ctx.run(
                raw_input(vec![key_up(egui::Key::ArrowDown, egui::Modifiers::NONE)]),
                |ctx| {
                    let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
                },
            );
            let _ = ctx.run(
                raw_input(vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)]),
                |ctx| {
                    let _ = run_grid_rows(ctx, &mut selection, &mut sort, 100, 120.0);
                },
            );
        }
        assert_eq!(selection.cursor().map(|(row, _)| row), Some(row + 12));
    }

    #[test]
    fn virtualization_limits_painting_to_the_viewport() {
        let viewport = egui::Rect::from_min_max(
            egui::pos2(0.0, 0.0),
            egui::pos2(600.0, 400.0),
        );
        let range = visible_row_range(viewport, 30.0, 22.0, 100_000);
        assert_eq!(range.start, 0);
        assert!((16..=20).contains(&range.end), "{range:?}");

        let middle = egui::Rect::from_min_max(
            egui::pos2(0.0, 5_000.0),
            egui::pos2(600.0, 5_400.0),
        );
        let range = visible_row_range(middle, 30.0, 22.0, 100_000);
        assert!((200..230).contains(&range.start), "{range:?}");
        assert!(range.end <= 250, "{range:?}");
        assert!(range.len() <= 22, "{range:?}");

        let bottom = egui::Rect::from_min_max(
            egui::pos2(0.0, 2_199_700.0),
            egui::pos2(600.0, 2_200_030.0),
        );
        let range = visible_row_range(bottom, 30.0, 22.0, 100_000);
        assert_eq!(range.end, 100_000);
        assert!(range.start >= 99_980, "{range:?}");

        assert_eq!(
            visible_row_range(viewport, 30.0, 22.0, 0),
            0..0,
            "empty grids must not paint a row"
        );
    }

    #[test]
    fn grid_selection_tsv_quotes_control_characters_and_nulls() {
        let mut selection = GridSelection::default();
        selection.set_cell(0, 0);
        let columns = vec!["value".to_string()];
        let rows = vec![vec![Some("a\tb".to_string())]];
        assert_eq!(
            selection.to_tsv(&columns, &rows),
            Some("\"a\tb\"".to_string())
        );

        let mut all = GridSelection::default();
        all.select_all(2, 2);
        let columns = vec!["a".to_string(), "b".to_string()];
        let rows = vec![
            vec![Some("1".to_string()), None],
            vec![Some("x".to_string()), Some("y".to_string())],
        ];
        assert_eq!(all.to_tsv(&columns, &rows), Some("1\t\nx\ty".to_string()));
    }

    #[test]
    fn grid_click_and_arrow_keys_move_the_selection() {
        let ctx = egui::Context::default();
        let mut selection = GridSelection::default();
        let mut sort = Vec::new();

        click(&ctx, &mut selection, &mut sort, egui::pos2(70.0, 50.0));
        assert_eq!(selection.cursor(), Some((0, 0)));

        let _ = ctx.run(
            raw_input(vec![key(egui::Key::ArrowDown, egui::Modifiers::NONE)]),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        assert_eq!(selection.cursor(), Some((1, 0)));

        // Click the first cell again, then extend the range with Shift+Down.
        click(&ctx, &mut selection, &mut sort, egui::pos2(70.0, 50.0));
        let _ = ctx.run(
            raw_input_with(
                vec![key(egui::Key::ArrowDown, egui::Modifiers::SHIFT)],
                egui::Modifiers::SHIFT,
            ),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        assert_eq!(selection.normalized(), Some(((0, 0), (1, 0))));
    }

    #[test]
    fn right_click_inside_a_range_keeps_the_range() {
        let ctx = egui::Context::default();
        let mut selection = GridSelection::default();
        let mut sort = Vec::new();

        click(&ctx, &mut selection, &mut sort, egui::pos2(70.0, 50.0));
        let _ = ctx.run(
            raw_input_with(
                vec![key(egui::Key::ArrowDown, egui::Modifiers::SHIFT)],
                egui::Modifiers::SHIFT,
            ),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        assert_eq!(selection.normalized(), Some(((0, 0), (1, 0))));

        let pos = egui::pos2(70.0, 50.0);
        let secondary = |pressed: bool| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Secondary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos)]),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos), secondary(true)]),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(pos), secondary(false)]),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        assert_eq!(selection.normalized(), Some(((0, 0), (1, 0))));
    }

    #[test]
    fn grid_header_click_cycles_sort_and_shift_adds_a_column() {
        let ctx = egui::Context::default();
        let mut selection = GridSelection::default();
        let mut sort = Vec::new();

        // Second column header ("name"), at x 134..206.
        click(&ctx, &mut selection, &mut sort, egui::pos2(145.0, 20.0));
        assert_eq!(
            sort,
            vec![SortSpec {
                column: "name".to_string(),
                descending: false,
            }]
        );

        click(&ctx, &mut selection, &mut sort, egui::pos2(145.0, 20.0));
        assert!(sort[0].descending);

        click(&ctx, &mut selection, &mut sort, egui::pos2(145.0, 20.0));
        assert!(sort.is_empty());

        // Shift-click the first column after sorting the second one: both
        // remain, in click order.
        click(&ctx, &mut selection, &mut sort, egui::pos2(70.0, 20.0));
        let shift = egui::Event::PointerButton {
            pos: egui::pos2(145.0, 20.0),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::SHIFT,
        };
        let shift_release = egui::Event::PointerButton {
            pos: egui::pos2(145.0, 20.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::SHIFT,
        };
        let shift_pos = egui::pos2(145.0, 20.0);
        let _ = ctx.run(
            raw_input(vec![egui::Event::PointerMoved(shift_pos)]),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        let _ = ctx.run(
            raw_input_with(
                vec![egui::Event::PointerMoved(shift_pos), shift],
                egui::Modifiers::SHIFT,
            ),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        let _ = ctx.run(
            raw_input_with(
                vec![egui::Event::PointerMoved(shift_pos), shift_release],
                egui::Modifiers::SHIFT,
            ),
            |ctx| {
                let _ = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        assert_eq!(sort.len(), 2);
        assert_eq!(sort[0].column, "id");
        assert_eq!(sort[1].column, "name");
    }

    #[test]
    fn grid_gutter_click_copies_the_whole_row_with_cmd_c() {
        let ctx = egui::Context::default();
        let mut selection = GridSelection::default();
        let mut sort = Vec::new();

        click(&ctx, &mut selection, &mut sort, egui::pos2(20.0, 50.0));
        assert_eq!(selection.summary(), Some((1, 2)));

        let mut action = GridAction::None;
        let _ = ctx.run(
            raw_input(vec![key(egui::Key::C, egui::Modifiers::COMMAND)]),
            |ctx| {
                action = run_grid(ctx, &mut selection, &mut sort);
            },
        );
        match action {
            GridAction::CopyText(text) => assert_eq!(text, "1\tA"),
            _ => panic!("Cmd+C did not return the selected row"),
        }
    }
}
