use eframe::egui;
use egui_phosphor::Variant;

/// Orange accent used for selection, links and the status bar.
pub const ACCENT: egui::Color32 = egui::Color32::from_rgb(234, 88, 12);

/// Panel background.
pub const PANEL_FILL: egui::Color32 = egui::Color32::from_rgb(22, 26, 36);
/// Status bar background: a darker orange than `ACCENT`, so white text reaches WCAG AA (>= 4.5:1).
pub const STATUS_BAR_FILL: egui::Color32 = egui::Color32::from_rgb(194, 65, 12);
/// Background of text inputs in the sidebar.
pub const INPUT_FILL: egui::Color32 = egui::Color32::from_gray(40);

/// Green of the "online" status dot and the "Connected" badge text.
pub const ONLINE: egui::Color32 = egui::Color32::from_rgb(34, 197, 94);
/// Background of the "Connected" badge (dark pill).
pub const BADGE_FILL: egui::Color32 = egui::Color32::from_rgb(15, 18, 26);

/// Font family of the filled Phosphor icons (folders in the explorer).
pub fn fill_family() -> egui::FontFamily {
    egui::FontFamily::Name("phosphor-fill".into())
}

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
    fonts.font_data.insert(
        "phosphor-fill".into(),
        egui::FontData::from_static(Variant::Fill.font_bytes()).into(),
    );
    // Fill family: the filled icons first, then everything Proportional has, so text outside the
    // icon codepoints still resolves. (The fill font maps only the icon range.)
    let mut chain = vec!["phosphor-fill".to_owned()];
    chain.extend(
        fonts
            .families
            .get(&egui::FontFamily::Proportional)
            .cloned()
            .unwrap_or_default(),
    );
    fonts.families.insert(fill_family(), chain);
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        let v = &mut s.visuals;
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = egui::Stroke::new(1.0, egui::Color32::WHITE);
        v.hyperlink_color = ACCENT;
        v.panel_fill = PANEL_FILL;
        v.window_fill = egui::Color32::from_rgb(28, 33, 45);
    });
}

#[cfg(test)]
pub(crate) fn luminance(c: egui::Color32) -> f32 {
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

/// WCAG contrast ratio of two colours.
#[cfg(test)]
pub(crate) fn contrast(a: egui::Color32, b: egui::Color32) -> f32 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_family_resolves_the_filled_folder_glyph() {
        // Arrange
        let font = egui::FontId::new(14.0, fill_family());
        let seen = std::cell::Cell::new((false, false));
        let mut h = egui_kittest::Harness::new_ui(|ui| {
            install_theme(ui.ctx());
            let r = ui
                .ctx()
                .fonts_mut(|f| (f.has_glyph(&font, '\u{E24A}'), f.has_glyph(&font, 'a')));
            seen.set(r);
        });

        // Act: fonts apply from the second pass on.
        h.run();
        h.run();

        // Assert
        assert_eq!(seen.get(), (true, true), "folder glyph, text fallback");
    }

    #[test]
    fn status_bar_fill_gives_white_text_wcag_aa() {
        // Act & Assert
        assert!(contrast(egui::Color32::WHITE, STATUS_BAR_FILL) >= 4.5);
    }

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

    #[test]
    fn subtle_selection_text_reaches_wcag_aa_on_dark_and_light() {
        for mut v in [egui::Visuals::dark(), egui::Visuals::light()] {
            // Arrange
            let panel = v.panel_fill;

            // Act
            subtle_selection(&mut v);

            // Assert
            let bg = panel.blend(v.selection.bg_fill);
            let ratio = contrast(v.selection.stroke.color, bg);
            assert!(v.selection.bg_fill.a() < 80, "not a solid pill");
            assert!(ratio >= 4.5, "contrast {ratio}");
        }
    }
}
