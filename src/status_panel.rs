//! Fenêtre "machine status" (F12), en egui au-dessus de wgpu (Plan V2.md,
//! jalon M1) : remplace le texte dessiné à la main avec `SDL_ttf` par un
//! panneau egui, à périmètre fonctionnel identique. Contexte GPU via
//! `egui_gpu.rs`, partagé avec la console complète (F11).

use crate::egui_gpu::EguiGpu;
use sdl2::video::Window;

pub struct StatusPanel {
    gpu: EguiGpu,
    start: std::time::Instant,
    /// # Sécurité
    ///
    /// Voir `egui_gpu::EguiGpu::new` : la surface qu'elle détient n'a aucun
    /// lien de durée de vie formel avec `window`, qui doit donc rester en
    /// vie au moins aussi longtemps — garanti ici en la possédant, et en la
    /// déclarant après `gpu` (Rust détruit les champs dans leur ordre de
    /// déclaration).
    window: Window,
}

impl StatusPanel {
    pub fn new(window: Window) -> Result<Self, String> {
        let gpu = EguiGpu::new(&window, "bytebox status panel device")?;
        Ok(Self {
            gpu,
            start: std::time::Instant::now(),
            window,
        })
    }

    pub fn window_mut(&mut self) -> &mut Window {
        &mut self.window
    }

    pub fn handle_event(&mut self, event: &sdl2::event::Event) {
        self.gpu.handle_event(&self.window, event);
    }

    pub fn resize(&mut self) {
        self.gpu.resize(&self.window);
    }

    /// Dessine le panneau à partir des mêmes chaînes que produisait
    /// l'ancien rendu SDL_ttf (`Machine::get_registers_string`,
    /// `Machine::get_hardware_string`) : texte monospace clair sur fond bleu
    /// nuit, avec le marqueur "accès disque en cours" (point rouge en fin de
    /// ligne) rendu à part, comme avant.
    pub fn render(&mut self, registers: &str, hardware: &str) {
        let mut text = String::with_capacity(registers.len() + 1 + hardware.len());
        text.push_str(registers);
        text.push('\n');
        text.push_str(hardware);

        self.gpu.present(&self.window, self.start, |ctx| {
            egui::CentralPanel::default()
                .frame(
                    egui::Frame::default()
                        .fill(egui::Color32::from_rgb(15, 15, 25))
                        .inner_margin(10.0),
                )
                .show(ctx, |ui| {
                    // Le contenu (surtout la matrice clavier, activable en
                    // config) peut dépasser la hauteur de la fenêtre : plutôt
                    // que de rogner silencieusement les dernières lignes,
                    // elles restent accessibles par défilement.
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        for line in text.lines() {
                            let line = line.replace('\t', "    ");
                            let (text_part, dot) = match line.strip_suffix('\u{25CF}') {
                                Some(prefix) => (prefix, true),
                                None => (line.as_str(), false),
                            };
                            ui.horizontal(|ui| {
                                ui.spacing_mut().item_spacing.x = 0.0;
                                if !text_part.is_empty() {
                                    ui.label(
                                        egui::RichText::new(text_part)
                                            .monospace()
                                            .color(egui::Color32::from_rgb(220, 220, 225)),
                                    );
                                }
                                if dot {
                                    ui.label(
                                        egui::RichText::new("\u{25CF}")
                                            .monospace()
                                            .color(egui::Color32::from_rgb(220, 40, 40)),
                                    );
                                }
                            });
                        }
                    });
                });
        });
    }
}
