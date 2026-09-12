//! Fenêtre "console" (F11), sur le même modèle que la fenêtre "machine
//! status" (F12, `status_panel.rs`) : contexte wgpu indépendant (via
//! `egui_gpu.rs`), fenêtre SDL2 séparée, cachée par défaut.
//!
//! Remplace entièrement la console pilotée depuis le terminal qui a lancé
//! l'émulateur (Plan V2.md, jalon M2) : `console.rs`, qui lisait `stdin`,
//! a disparu avec elle. Tout l'historique y est visible et défilable, sans
//! jamais être tronqué — contrairement à la barre rapide (F10,
//! `console_panel.rs`, embarquée dans la fenêtre d'émulation), qui n'en
//! montre qu'une ligne.

use crate::console_log::ConsoleLog;
use crate::egui_gpu::EguiGpu;
use bytebox_core::monitor::{MonitorMessage, parse_command};
use sdl2::video::Window;
use std::sync::mpsc::Sender;

pub struct ConsoleWindow {
    gpu: EguiGpu,
    start: std::time::Instant,
    input: String,
    /// Le focus doit être redemandé à chaque ouverture (F11), comme pour la
    /// barre rapide.
    request_focus: bool,
    window: Window,
}

impl ConsoleWindow {
    pub fn new(window: Window) -> Result<Self, String> {
        let gpu = EguiGpu::new(&window, "bytebox console window device")?;
        Ok(Self {
            gpu,
            start: std::time::Instant::now(),
            input: String::new(),
            request_focus: true,
            window,
        })
    }

    pub fn window_mut(&mut self) -> &mut Window {
        &mut self.window
    }

    pub fn request_focus(&mut self) {
        self.request_focus = true;
    }

    pub fn handle_event(&mut self, event: &sdl2::event::Event) {
        self.gpu.handle_event(&self.window, event);
    }

    pub fn resize(&mut self) {
        self.gpu.resize(&self.window);
    }

    pub fn render(&mut self, log: &mut ConsoleLog, cmd_sender: &Sender<MonitorMessage>) {
        let input = &mut self.input;
        let request_focus = &mut self.request_focus;

        let bg = egui::Color32::from_rgb(15, 15, 25);
        self.gpu.present(&self.window, self.start, |ctx| {
            // La ligne de saisie occupe le bas de la fenêtre en premier
            // (l'ordre d'ajout compte pour les TopBottomPanel) : c'est ce
            // qui reste ensuite qui devient la zone d'historique, plutôt
            // que l'inverse.
            egui::TopBottomPanel::bottom("console_input")
                .frame(egui::Frame::default().fill(bg).inner_margin(10.0))
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new(">")
                                .monospace()
                                .color(egui::Color32::from_rgb(220, 220, 225)),
                        );
                        let response = ui.add(
                            egui::TextEdit::singleline(input)
                                .desired_width(f32::INFINITY)
                                .font(egui::TextStyle::Monospace)
                                .hint_text("command (h for help)"),
                        );
                        if *request_focus {
                            response.request_focus();
                            *request_focus = false;
                        }
                        let submitted = response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if submitted {
                            let line = std::mem::take(input);
                            if !line.trim().is_empty() {
                                log.push_command(&line);
                                let _ = cmd_sender.send(parse_command(&line));
                            }
                            *request_focus = true;
                        }
                    });
                });
            egui::CentralPanel::default()
                .frame(egui::Frame::default().fill(bg).inner_margin(10.0))
                .show(ctx, |ui| {
                    // auto_shrink(false) : sans ça, la zone de défilement se
                    // réduit à la largeur du texte le plus long, et
                    // l'ascenseur se retrouve collé contre le texte plutôt
                    // que contre le bord droit de la fenêtre.
                    egui::ScrollArea::vertical()
                        .auto_shrink([false, false])
                        .stick_to_bottom(true)
                        .show(ui, |ui| {
                            for line in log.lines() {
                                ui.label(
                                    egui::RichText::new(line)
                                        .monospace()
                                        .color(egui::Color32::from_rgb(220, 220, 225)),
                                );
                            }
                        });
                });
        });
    }
}
