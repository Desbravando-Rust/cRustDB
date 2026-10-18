use eframe::egui;
use egui_phosphor::Variant;

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
    ctx.global_style_mut(|s| s.spacing.item_spacing = egui::vec2(8.0, 6.0));
}
