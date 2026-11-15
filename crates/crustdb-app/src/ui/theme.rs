use eframe::egui;
use egui_phosphor::Variant;

/// Orange accent used for selection, links and the status bar.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(234, 88, 12);

/// Green of the "online" status dot and the "Connected" badge text.
pub const ONLINE: egui::Color32 = egui::Color32::from_rgb(34, 197, 94);
/// Background of the "Connected" badge.
pub const BADGE_FILL: egui::Color32 = egui::Color32::from_rgb(20, 60, 40);

/// Faint accent tint behind active rows and tabs.
pub fn subtle_fill() -> egui::Color32 {
    ACCENT.gamma_multiply(0.2)
}

/// Restyle `selection` for list rows and tabs: a faint accent tint instead of a solid pill, with
/// strong text colour (also the focus ring colour) so it stays readable in light and dark.
pub fn subtle_selection(v: &mut egui::Visuals) {
    v.selection.bg_fill = subtle_fill();
    v.selection.stroke = egui::Stroke::new(1.0, v.strong_text_color());
}

/// Fixed dark theme plus the Phosphor icon font.
pub fn install_theme(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    let mut fonts = egui::FontDefinitions::default();
    // egui-phosphor 0.14 targets egui 0.36, so only its font bytes and icon constants are used.
    fonts.font_data.insert(
        "phosphor".into(),
        egui::FontData::from_static(Variant::Regular.font_bytes()).into(),
    );
    if let Some(keys) = fonts.families.get_mut(&egui::FontFamily::Proportional) {
        keys.insert(keys.len().min(1), "phosphor".into());
    }
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        let v = &mut s.visuals;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        v.hyperlink_color = ACCENT;
        v.panel_fill = egui::Color32::from_rgb(22, 26, 36);
        v.window_fill = egui::Color32::from_rgb(28, 33, 45);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_uses_orange_accent() {
        // Arrange
        let ctx = egui::Context::default();

        // Act
        install_theme(&ctx);

        // Assert
        let v = ctx.global_style().visuals.clone();
        assert_eq!(v.selection.bg_fill, ACCENT);
        assert_eq!(v.hyperlink_color, ACCENT);
    }

    fn luminance(c: egui::Color32) -> f32 {
        let lin = |v: u8| {
            let v = f32::from(v) / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    }

    #[test]
    fn subtle_selection_text_reaches_wcag_aa_on_dark_and_light() {
        for mut v in [egui::Visuals::dark(), egui::Visuals::light()] {
            // Arrange
            let panel = v.panel_fill;

            // Act
            subtle_selection(&mut v);

            // Assert
            let bg = panel.blend(v.selection.bg_fill);
            let (a, b) = (luminance(v.selection.stroke.color), luminance(bg));
            let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
            assert!(v.selection.bg_fill.a() < 80, "not a solid pill");
            assert!(ratio >= 4.5, "contrast {ratio}");
        }
    }
}
