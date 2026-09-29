//! Présentation des fenêtres de réglages (F6), à la manière de WinUAE /
//! Amiberry : thème clair façon boîte de dialogue Windows, arborescence de
//! pages à gauche, cadres titrés (« group box ») à droite.
//!
//! Partagée plutôt que dupliquée dans chaque émulateur (MSXForge, bytebox) :
//! seul le contenu des pages leur est propre. S'applique au contexte egui
//! d'une fenêtre SDL2 séparée (`egui_gpu.rs`) — pas aux panneaux superposés
//! à la fenêtre principale, qui partagent le contexte du `Renderer` et
//! gardent le thème sombre d'egui.
//!
//! Disposition type d'une trame :
//!
//! ```ignore
//! egui::TopBottomPanel::bottom("buttons").frame(settings_theme::panel_frame(12, 10))...
//! egui::SidePanel::left("tree").exact_width(190.0).frame(settings_theme::panel_frame(10, 10))
//!     .show(ctx, |ui| settings_theme::page_list(ui, TREE, &mut page));
//! egui::CentralPanel::default().frame(settings_theme::panel_frame(12, 12))
//!     .show(ctx, |ui| settings_theme::group(ui, "Titre", |ui| { ... }));
//! ```

use egui::Color32;

// Couleurs façon boîte de dialogue Windows (WinUAE).
pub const BG: Color32 = Color32::from_rgb(240, 240, 240);
pub const LIST_BG: Color32 = Color32::WHITE;
pub const BORDER: Color32 = Color32::from_rgb(173, 173, 173);
pub const GROUP_BORDER: Color32 = Color32::from_rgb(208, 208, 208);
pub const SELECTION: Color32 = Color32::from_rgb(0, 120, 215);
pub const TEXT: Color32 = Color32::from_rgb(20, 20, 20);
pub const DIM: Color32 = Color32::from_rgb(90, 90, 90);
pub const BUTTON: Color32 = Color32::from_rgb(225, 225, 225);
pub const BUTTON_HOVER: Color32 = Color32::from_rgb(229, 241, 251);
const BUTTON_PRESSED: Color32 = Color32::from_rgb(204, 228, 247);

/// Pose le thème sur le contexte egui de la fenêtre de réglages ; une fois
/// pour toutes, à la création de la fenêtre.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.panel_fill = BG;
    visuals.window_fill = BG;
    visuals.extreme_bg_color = LIST_BG;
    visuals.override_text_color = Some(TEXT);
    visuals.selection.bg_fill = SELECTION;
    visuals.selection.stroke = egui::Stroke::new(1.0_f32, Color32::WHITE);
    let radius = egui::CornerRadius::same(2);
    for (w, fill, stroke) in [
        (&mut visuals.widgets.inactive, BUTTON, BORDER),
        (&mut visuals.widgets.hovered, BUTTON_HOVER, SELECTION),
        (&mut visuals.widgets.active, BUTTON_PRESSED, SELECTION),
    ] {
        w.bg_fill = fill;
        w.weak_bg_fill = fill;
        w.bg_stroke = egui::Stroke::new(1.0_f32, stroke);
        w.corner_radius = radius;
        w.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    }
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0_f32, GROUP_BORDER);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0_f32, TEXT);
    ctx.set_visuals(visuals);
    ctx.style_mut(|s| {
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(12.0, 4.0);
        s.spacing.slider_width = 220.0;
    });
}

/// Cadre d'un panneau (boutons du bas, arborescence, page) : fond de boîte
/// de dialogue, marges horizontale `x` et verticale `y`.
pub fn panel_frame(x: i8, y: i8) -> egui::Frame {
    egui::Frame::new()
        .fill(BG)
        .inner_margin(egui::Margin::symmetric(x, y))
}

/// Arborescence de gauche : titre « Settings », puis une liste blanche de
/// catégories et de leurs pages. `tree` : (catégorie, [(page, nom)]) ; une
/// catégorie vide place ses pages au premier niveau (« About », « Help »).
/// Toute la largeur d'une ligne est cliquable, fond bleu si sélectionnée.
pub fn page_list<P: Copy + PartialEq>(
    ui: &mut egui::Ui,
    tree: &[(&str, &[(P, &str)])],
    page: &mut P,
) {
    ui.label(egui::RichText::new("Settings").strong());
    egui::Frame::new()
        .fill(LIST_BG)
        .stroke(egui::Stroke::new(1.0_f32, BORDER))
        .inner_margin(egui::Margin::same(6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.set_min_height(ui.available_height());
            for (category, pages) in tree {
                if !category.is_empty() {
                    ui.label(egui::RichText::new(*category).strong());
                }
                for &(p, name) in *pages {
                    let indent = if category.is_empty() { 4.0 } else { 18.0 };
                    let height = ui.text_style_height(&egui::TextStyle::Body) + 6.0;
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), height),
                        egui::Sense::click(),
                    );
                    let selected = *page == p;
                    if selected {
                        ui.painter().rect_filled(rect, 0.0, SELECTION);
                    } else if response.hovered() {
                        ui.painter().rect_filled(rect, 0.0, BUTTON_HOVER);
                    }
                    ui.painter().text(
                        rect.left_center() + egui::vec2(indent, 0.0),
                        egui::Align2::LEFT_CENTER,
                        name,
                        egui::TextStyle::Body.resolve(ui.style()),
                        if selected { Color32::WHITE } else { TEXT },
                    );
                    if response.clicked() {
                        *page = p;
                    }
                }
                ui.add_space(4.0);
            }
        });
}

/// Cadre titré (« group box » des boîtes de dialogue Windows).
pub fn group(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    // Titre incrusté dans la bordure du cadre : on réserve la moitié de sa
    // hauteur au-dessus du cadre, puis on le peint sur un fond qui masque
    // la bordure à cet endroit.
    let font = egui::TextStyle::Body.resolve(ui.style());
    let galley = ui.painter().layout_no_wrap(title.to_string(), font, TEXT);
    let title_h = galley.size().y;
    ui.add_space(title_h / 2.0 + 2.0);
    let response = egui::Frame::new()
        .stroke(egui::Stroke::new(1.0_f32, GROUP_BORDER))
        .corner_radius(3.0)
        .inner_margin(egui::Margin {
            left: 10,
            right: 10,
            top: (title_h / 2.0 + 6.0) as i8,
            bottom: 10,
        })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add_contents(ui);
        })
        .response;
    let pos = egui::pos2(
        response.rect.left() + 10.0,
        response.rect.top() - title_h / 2.0,
    );
    let backdrop = egui::Rect::from_min_size(
        pos - egui::vec2(4.0, 0.0),
        galley.size() + egui::vec2(8.0, 0.0),
    );
    ui.painter().rect_filled(backdrop, 0.0, BG);
    ui.painter().galley(pos, galley, TEXT);
    ui.add_space(8.0);
}

/// Texte secondaire (remarque, état vide), en gris.
pub fn dim(ui: &mut egui::Ui, text: impl Into<String>) {
    ui.label(egui::RichText::new(text).color(DIM));
}
