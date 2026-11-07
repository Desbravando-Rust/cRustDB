use eframe::egui;
use egui_phosphor::Variant;

/// Orange accent used for selection, links and the status bar.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(234, 88, 12);

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
}
