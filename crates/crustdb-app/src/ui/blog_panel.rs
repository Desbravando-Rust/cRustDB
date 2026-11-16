//! "From the Blog" panel: featured book card plus the latest post excerpt (ADR 0012 addendum).
use eframe::egui;

use crate::news::News;
use egui_phosphor::regular as icon;

const SITE_URL: &str = "https://desbravandorust.com.br/";
/// Hide the panel when the central area is too small to leave room for the content above it.
const MIN_SIZE: egui::Vec2 = egui::vec2(640.0, 380.0);

pub fn show(ui: &mut egui::Ui, news: &News) {
    if ui.available_width() < MIN_SIZE.x || ui.available_height() < MIN_SIZE.y {
        return;
    }
    egui::Panel::bottom("blog_panel")
        .resizable(false)
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                book_card(ui);
                ui.vertical(|ui| post(ui, news));
            });
        });
}

fn book_card(ui: &mut egui::Ui) {
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.set_width(300.0);
        ui.horizontal_top(|ui| {
            ui.add(
                egui::Image::new(egui::include_image!(
                    "../../../../assets/imgs/book-cover.png"
                ))
                .fit_to_exact_size(egui::vec2(80.0, 122.0))
                .alt_text("Desbravando Rust book cover"),
            );
            ui.vertical(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    egui::RichText::new("DESTAQUE").strong(),
                );
                ui.strong("Desbravando Rust");
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(
                            "Um guia prático para pythonistas explorarem novos horizontes",
                        )
                        .italics()
                        .small(),
                    )
                    .wrap(),
                );
                ui.weak("Por: José Luis da Cruz Junior");
                ui.hyperlink_to(format!("SAIBA MAIS {}", icon::ARROW_RIGHT), SITE_URL);
            });
        });
    });
}

fn post(ui: &mut egui::Ui, news: &News) {
    ui.horizontal_wrapped(|ui| {
        ui.strong("From the Blog:");
        match news.post.get() {
            Some(p) => ui.hyperlink_to(&p.title, &p.url),
            // Offline, opted out or empty feed: static line.
            None => ui.hyperlink_to("desbravandorust.com.br", SITE_URL),
        };
    });
    if let Some(p) = news.post.get().filter(|p| !p.excerpt.is_empty()) {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(egui::Label::new(egui::RichText::new(&p.excerpt).italics().weak()).wrap());
        });
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used)]
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;

    use super::*;
    use crate::news::Post;

    /// Fonts apply on the next pass and image loaders must exist before the cover renders.
    fn ready(ui: &egui::Ui, done: &mut bool) -> bool {
        if std::mem::replace(done, true) {
            return true;
        }
        crate::ui::theme::install_theme(ui.ctx());
        egui_extras::install_image_loaders(ui.ctx());
        false
    }

    fn render(news: &News) -> Harness<'_> {
        let mut done = false;
        let mut h = Harness::builder()
            .with_size(egui::vec2(900.0, 700.0))
            .build_ui(move |ui| {
                if ready(ui, &mut done) {
                    show(ui, news);
                }
            });
        h.run();
        h
    }

    #[test]
    fn shows_book_card_post_link_and_excerpt() {
        // Arrange
        let news = News::default();
        let _ = news.post.set(Post {
            title: "Why Indexing Matters".into(),
            url: "https://x/post".into(),
            excerpt: "Many developers treat databases as black boxes".into(),
        });

        // Act
        let h = render(&news);

        // Assert
        h.get_by_label("DESTAQUE");
        h.get_by_label("Desbravando Rust");
        h.get_by_label(&format!("SAIBA MAIS {}", icon::ARROW_RIGHT));
        h.get_by_label("From the Blog:");
        h.get_by_role_and_label(egui::accesskit::Role::Link, "Why Indexing Matters");
        h.get_by_label_contains("black boxes");
    }

    #[test]
    fn falls_back_to_site_link_without_post() {
        // Arrange
        let news = News::default();

        // Act
        let h = render(&news);

        // Assert
        h.get_by_role_and_label(egui::accesskit::Role::Link, "desbravandorust.com.br");
        h.get_by_label("DESTAQUE");
    }

    #[test]
    fn hidden_when_area_is_small() {
        // Arrange
        let news = News::default();
        let mut h = Harness::builder()
            .with_size(egui::vec2(300.0, 200.0))
            .build_ui(|ui| show(ui, &news));

        // Act
        h.run();

        // Assert
        assert!(h.query_by_label("DESTAQUE").is_none());
    }
}
