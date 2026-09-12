//! Barre de commande rapide (F10), superposée à l'image émulée par
//! `renderer.rs` (Plan V2.md, jalon M2) : une ligne de saisie et, au plus,
//! une ligne de retour — pour une commande ponctuelle sans occuper l'écran.
//! La console complète, avec tout l'historique défilant, c'est
//! `console_window.rs` (F11).
//!
//! Ce module ne connaît rien à wgpu ni à SDL2 : il construit juste l'UI
//! egui et pousse les commandes saisies sur le même canal `MonitorCmd` que
//! la console complète — `Machine` ne voit donc aucune différence entre les
//! deux façades.

use crate::console_log::ConsoleLog;
use bytebox_core::monitor::{MonitorMessage, parse_command};
use std::sync::mpsc::Sender;

pub struct QuickCommandBar {
    input: String,
    /// Le focus doit être redemandé à chaque ouverture (F10) : `egui` ne le
    /// retient pas d'une trame sur l'autre quand le panneau est reconstruit.
    request_focus: bool,
}

impl QuickCommandBar {
    pub fn new() -> Self {
        Self {
            input: String::new(),
            request_focus: true,
        }
    }

    pub fn request_focus(&mut self) {
        self.request_focus = true;
    }

    /// `window_size` : taille réelle de la fenêtre CPC (`sdl.rs`), pour
    /// grossir police/espacements avec le zoom (F1-F4) — même mécanisme que
    /// `ConfigPanel` (F6), voir `crate::ui_scale`. Un `TopBottomPanel`
    /// recalcule sa hauteur à chaque trame (contrairement à un
    /// `egui::Window`, dont `default_width`/`default_pos` ne s'appliquent
    /// qu'à la toute première apparition) : pas besoin ici d'un id changeant
    /// à chaque redimensionnement pour rester à jour.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        cmd_sender: &Sender<MonitorMessage>,
        log: &mut ConsoleLog,
        window_size: egui::Vec2,
    ) {
        let scale = crate::ui_scale::content_scale(window_size);
        egui::TopBottomPanel::bottom("quick_command_bar")
            .resizable(false)
            .exact_height(if log.last_line().is_some() { 56.0 } else { 34.0 } * scale)
            .frame(
                egui::Frame::default()
                    .fill(egui::Color32::from_rgba_unmultiplied(15, 15, 25, 235))
                    .inner_margin(6.0 * scale),
            )
            .show(ctx, |ui| {
                ui.set_style(crate::ui_scale::scaled_style(ui.style(), scale));
                // Jamais plus d'une ligne de retour : c'est ce qui distingue
                // cette barre de la console complète (F11).
                if let Some(last) = log.last_line() {
                    ui.label(
                        egui::RichText::new(last)
                            .monospace()
                            .color(egui::Color32::from_rgb(180, 180, 190)),
                    );
                }
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new(">")
                            .monospace()
                            .color(egui::Color32::from_rgb(220, 220, 225)),
                    );
                    let response = ui.add(
                        egui::TextEdit::singleline(&mut self.input)
                            .desired_width(f32::INFINITY)
                            .font(egui::TextStyle::Monospace)
                            .hint_text("quick command — F11 for the full console"),
                    );
                    if self.request_focus {
                        response.request_focus();
                        self.request_focus = false;
                    }
                    let submitted =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if submitted {
                        let line = std::mem::take(&mut self.input);
                        if !line.trim().is_empty() {
                            log.push_command(&line);
                            let _ = cmd_sender.send(parse_command(&line));
                        }
                        // Redemande le focus : sans ça, ENTRÉE le fait perdre
                        // et la commande suivante exigerait un clic.
                        self.request_focus = true;
                    }
                });
            });
    }
}
