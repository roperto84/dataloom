use eframe::egui::{self, Color32, FontFamily, FontId, Rounding, Stroke, TextStyle, Visuals};

// Brand mark: an indigo-to-sky tile with a white database cylinder and a mint
// spark. `tools/make_icon.py` renders the same shapes into the app icon.
pub const LOGO_TOP: Color32 = Color32::from_rgb(99, 102, 241);
pub const LOGO_BOTTOM: Color32 = Color32::from_rgb(14, 165, 233);
pub const LOGO_GLYPH: Color32 = Color32::from_rgb(248, 250, 252);
pub const LOGO_SPARK: Color32 = Color32::from_rgb(52, 211, 153);
pub const LOGO_BAND: Color32 = Color32::from_rgb(67, 56, 202);

// Values recovered from the original's `ViewSetting` dictionary in
// `com.tinyapp.TablePlus` and from the decompiled settings code
// (`sub_1005a7988`, `sub_1005ab940`, `sub_1008168c0`).
pub const DATA_FONT_SIZE: f32 = 11.0;
/// `TableDataController.nib`, object 106 (`NSClassSwapper`):
/// `NSRowHeight = 22.0` and `NSIntercellSpacingHeight = 0.0`. The row height
/// is stored by the nib, not derived from font size + row padding.
pub const GRID_ROW_HEIGHT: f32 = 22.0;
pub const SIDEBAR_FONT_SIZE: f32 = 13.0;
pub const SQL_FONT_SIZE: f32 = 11.0;
pub const LINE_HEIGHT: f32 = 1.2;
pub const EDITOR_LINE_HEIGHT: f32 = SQL_FONT_SIZE * LINE_HEIGHT;
/// `sub_1008168c0` maps the four indent-width choices to these values.
pub const TAB_WIDTH_OPTIONS: [usize; 4] = [2, 4, 6, 8];
/// The stored default is choice 1, i.e. 4 spaces.
pub const TAB_WIDTH: usize = TAB_WIDTH_OPTIONS[1];

/// The palette every launch starts in. Dataloom opens light even when macOS is
/// in Dark Mode; the toolbar toggle switches to dark for the rest of the
/// session. (The original follows the system through `ThemeID` =
/// "builtin:auto"; this is a recreated app's own default.)
pub const DEFAULT_DARK: bool = false;

#[derive(Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub sidebar: Color32,
    pub topbar: Color32,
    pub tabbar: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted: Color32,
    pub faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub green: Color32,
    pub red: Color32,
    pub amber: Color32,
    pub purple: Color32,
    pub editor: Color32,
    pub grid_header: Color32,
    pub row_alt: Color32,
    pub row_hover: Color32,
    pub selection: Color32,
    pub card: Color32,
    pub code: Color32,
}

impl Palette {
    pub fn dark() -> Self {
        Self {
            dark: true,
            bg: Color32::from_rgb(20, 21, 24),
            panel: Color32::from_rgb(25, 27, 31),
            sidebar: Color32::from_rgb(22, 24, 27),
            topbar: Color32::from_rgb(27, 29, 34),
            tabbar: Color32::from_rgb(23, 25, 29),
            border: Color32::from_rgb(40, 43, 49),
            text: Color32::from_rgb(217, 221, 229),
            muted: Color32::from_rgb(139, 146, 158),
            faint: Color32::from_rgb(95, 102, 114),
            accent: Color32::from_rgb(79, 140, 255),
            accent_soft: Color32::from_rgba_unmultiplied(79, 140, 255, 34),
            green: Color32::from_rgb(78, 201, 164),
            red: Color32::from_rgb(255, 107, 107),
            amber: Color32::from_rgb(230, 181, 102),
            purple: Color32::from_rgb(179, 157, 219),
            editor: Color32::from_rgb(18, 19, 23),
            grid_header: Color32::from_rgb(32, 35, 42),
            row_alt: Color32::from_rgba_unmultiplied(255, 255, 255, 6),
            row_hover: Color32::from_rgba_unmultiplied(255, 255, 255, 12),
            selection: Color32::from_rgb(42, 74, 122),
            card: Color32::from_rgb(30, 32, 38),
            code: Color32::from_rgb(153, 199, 148),
        }
    }

    pub fn light() -> Self {
        Self {
            dark: false,
            bg: Color32::from_rgb(247, 248, 250),
            panel: Color32::from_rgb(255, 255, 255),
            sidebar: Color32::from_rgb(241, 243, 246),
            topbar: Color32::from_rgb(255, 255, 255),
            tabbar: Color32::from_rgb(238, 240, 244),
            border: Color32::from_rgb(226, 229, 234),
            text: Color32::from_rgb(30, 34, 40),
            muted: Color32::from_rgb(102, 112, 133),
            faint: Color32::from_rgb(152, 162, 179),
            accent: Color32::from_rgb(37, 99, 235),
            accent_soft: Color32::from_rgba_unmultiplied(37, 99, 235, 26),
            green: Color32::from_rgb(18, 128, 92),
            red: Color32::from_rgb(217, 45, 32),
            amber: Color32::from_rgb(181, 71, 8),
            purple: Color32::from_rgb(105, 65, 198),
            editor: Color32::from_rgb(255, 255, 255),
            grid_header: Color32::from_rgb(242, 244, 247),
            row_alt: Color32::from_rgba_unmultiplied(15, 23, 42, 5),
            row_hover: Color32::from_rgba_unmultiplied(15, 23, 42, 12),
            selection: Color32::from_rgb(219, 234, 254),
            card: Color32::from_rgb(255, 255, 255),
            code: Color32::from_rgb(5, 116, 84),
        }
    }
}

pub fn apply(ctx: &egui::Context, dark: bool, data_font_size: f32, sql_font_size: f32) -> Palette {
    let p = if dark {
        Palette::dark()
    } else {
        Palette::light()
    };
    let mut visuals = if dark {
        Visuals::dark()
    } else {
        Visuals::light()
    };

    visuals.override_text_color = Some(p.text);
    visuals.panel_fill = p.bg;
    visuals.window_fill = p.panel;
    visuals.extreme_bg_color = p.editor;
    visuals.faint_bg_color = p.sidebar;
    visuals.code_bg_color = p.grid_header;
    visuals.selection.bg_fill = p.selection;
    visuals.selection.stroke = Stroke::new(1.0_f32, p.accent);
    visuals.hyperlink_color = p.accent;
    visuals.warn_fg_color = p.amber;
    visuals.error_fg_color = p.red;
    visuals.window_stroke = Stroke::new(1.0_f32, p.border);
    visuals.window_rounding = Rounding::same(10.0);
    visuals.menu_rounding = Rounding::same(8.0);

    visuals.widgets.noninteractive.bg_fill = p.panel;
    visuals.widgets.noninteractive.weak_bg_fill = p.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, p.text);

    visuals.widgets.inactive.bg_fill = if dark {
        Color32::from_rgb(36, 39, 46)
    } else {
        Color32::from_rgb(236, 239, 244)
    };
    visuals.widgets.inactive.weak_bg_fill = visuals.widgets.inactive.bg_fill;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.inactive.rounding = Rounding::same(6.0);

    visuals.widgets.hovered.bg_fill = if dark {
        Color32::from_rgb(45, 49, 58)
    } else {
        Color32::from_rgb(226, 230, 237)
    };
    visuals.widgets.hovered.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.hovered.rounding = Rounding::same(6.0);

    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.weak_bg_fill = p.accent;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0_f32, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0_f32, Color32::WHITE);
    visuals.widgets.active.rounding = Rounding::same(6.0);

    visuals.widgets.open.bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.open.weak_bg_fill = visuals.widgets.hovered.bg_fill;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0_f32, p.border);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0_f32, p.text);
    visuals.widgets.open.rounding = Rounding::same(6.0);

    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 26.0;
    style.spacing.scroll.bar_width = 9.0;
    style.spacing.scroll.floating = false;
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(19.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(13.5, FontFamily::Proportional)),
        (
            TextStyle::Monospace,
            FontId::new(sql_font_size, FontFamily::Monospace),
        ),
        (
            TextStyle::Name("Data".into()),
            FontId::new(data_font_size, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(13.5, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(11.2, FontFamily::Proportional),
        ),
    ]
    .into();
    ctx.set_style(style);

    p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recovered_typography_matches_tableplus_viewsetting() {
        // `defaults read com.tinyapp.TablePlus ViewSetting`
        assert_eq!(DATA_FONT_SIZE, 11.0);
        assert_eq!(SIDEBAR_FONT_SIZE, 13.0);
        assert_eq!(SQL_FONT_SIZE, 11.0);
        assert_eq!(LINE_HEIGHT, 1.2);
        // `TableDataController.nib`: NSRowHeight = 22.0.
        assert_eq!(GRID_ROW_HEIGHT, 22.0);
        assert!((EDITOR_LINE_HEIGHT - 13.2).abs() < 1e-5);
        assert_eq!(TAB_WIDTH, 4);
        // `sub_1008168c0` (0x1008168c0) maps the four indent choices.
        assert_eq!(TAB_WIDTH_OPTIONS, [2, 4, 6, 8]);
    }

    #[test]
    fn launches_in_the_light_palette() {
        // The app must not inherit macOS Dark Mode on startup.
        assert!(!DEFAULT_DARK);
        let p = Palette::light();
        assert!(!p.dark);
        let luma = |c: Color32| c.r() as u32 + c.g() as u32 + c.b() as u32;
        assert!(
            luma(p.bg) > luma(p.text),
            "light palette has light surfaces"
        );
    }
}
