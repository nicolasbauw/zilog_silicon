//! Message d'information éphémère affiché en surimpression de l'écran émulé
//! (pas dans la console) : connexion d'une manette, bascule du shader CRT
//! (F5)... Toujours le même texte que celui déjà envoyé à la console (voir
//! les points d'appel dans `sdl.rs`), juste répété là où le regard est déjà
//! posé pendant qu'on joue — la console (F10/F11) ne l'est pas forcément.
//!
//! Contrairement aux panneaux F6/F7/F10, il doit pouvoir apparaître même
//! quand aucun d'eux n'est ouvert : c'est `sdl.rs` qui décide d'inclure la
//! passe egui rien que pour lui dans ce cas (`Osd::is_active`, combiné à
//! `show_overlay`).

use std::time::{Duration, Instant};

/// Deux secondes, comme demandé — voir TODO.txt.
const DURATION: Duration = Duration::from_secs(2);

pub struct Osd {
    /// Le texte et l'instant où il doit disparaître. `None` la plupart du
    /// temps : la vue courante ne coûte donc qu'un `Option` vide à tester,
    /// pas une structure à parcourir.
    message: Option<(String, Instant)>,
}

impl Osd {
    pub fn new() -> Self {
        Self { message: None }
    }

    pub fn show(&mut self, text: impl Into<String>) {
        self.message = Some((text.into(), Instant::now() + DURATION));
    }

    /// À tester avant de construire l'overlay de la trame (voir `sdl.rs`,
    /// `show_overlay`) : contrairement à `ui()`, ne fait pas expirer le
    /// message — seul `ui()`, appelé au plus une fois par trame, doit
    /// décider de son expiration, pour ne jamais la faire deux fois.
    pub fn is_active(&self) -> bool {
        self.message.as_ref().is_some_and(|(_, expires)| Instant::now() < *expires)
    }

    /// `window_size` : même mécanisme que les autres panneaux superposés à
    /// la fenêtre principale (voir `crate::ui_scale`) — un message minuscule
    /// en plein écran 4K serait aussi peu utile que l'était le panneau F6
    /// avant ce correctif.
    pub fn ui(&mut self, ctx: &egui::Context, window_size: egui::Vec2) {
        let Some((text, expires)) = &self.message else {
            return;
        };
        if Instant::now() >= *expires {
            self.message = None;
            return;
        }
        let scale = crate::ui_scale::content_scale(window_size);
        let text = text.clone();
        egui::Area::new(egui::Id::new("osd"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, 24.0 * scale))
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_style(crate::ui_scale::scaled_style(ui.style(), scale));
                egui::Frame::default()
                    .fill(egui::Color32::from_rgba_unmultiplied(15, 15, 25, 235))
                    .corner_radius(6.0 * scale)
                    .inner_margin(egui::vec2(14.0, 8.0) * scale)
                    .show(ui, |ui| {
                        // Sans ça, le label se voit attribuer la largeur
                        // disponible de l'`Area` (bien plus étroite que le
                        // texte une fois replié sur son contenu) et enroule
                        // le texte sur plusieurs lignes au lieu de rester sur
                        // une seule.
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(text)
                                    .monospace()
                                    .color(egui::Color32::from_rgb(220, 220, 225)),
                            )
                            .wrap_mode(egui::TextWrapMode::Extend),
                        );
                    });
            });
    }
}
