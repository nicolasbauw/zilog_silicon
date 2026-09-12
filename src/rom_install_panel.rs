//! Contenu de l'onglet "ROMs" du panneau de configuration (F6) :
//! avertissement légal + case à cocher obligatoire + bouton "Install ROMs",
//! qui télécharge/installe via `bytebox_core::rom_installer` — voir
//! `doc/roms-installation.md` pour le contexte de cette décision.
//!
//! Le téléchargement bloque plusieurs secondes : jamais appelé depuis la
//! boucle de rendu egui elle-même, toujours dans un thread dédié
//! (`std::thread::spawn`), dont la progression remonte par un canal
//! `mpsc` — même principe que `MonitorCmd`, mais propre à cet écran.
//! `Machine` n'est consultée qu'en lecture (`rom_status`, pour savoir si le
//! formulaire a une raison de s'afficher) ; toute action dessus (recharger
//! les ROMs après une installation) passe par `MonitorCmd::PowerCycle` sur
//! le canal existant, jamais par un accès direct.

use bytebox_core::machine::{Machine, RomStatus};
use bytebox_core::monitor::{MonitorCmd, MonitorMessage};
use bytebox_core::rom_installer::InstalledFile;
use std::sync::mpsc::{Receiver, Sender};

enum InstallEvent {
    Progress(String),
    Done(Vec<InstalledFile>),
    Error(String),
}

enum Status {
    Idle,
    Running(Vec<String>),
    Done(Vec<InstalledFile>),
    Failed(String),
}

pub struct RomInstallState {
    /// Case "j'ai lu et j'accepte" — condition d'activation du bouton.
    accepted: bool,
    status: Status,
    events: Option<Receiver<InstallEvent>>,
}

impl RomInstallState {
    pub fn new() -> Self {
        Self {
            accepted: false,
            status: Status::Idle,
            events: None,
        }
    }

    /// Absorbe les évènements du thread d'installation en cours, s'il y en a
    /// un. Appelée à chaque trame (voir `ConfigPanel::ui`), pas seulement
    /// quand l'onglet ROMs est affiché : un changement d'onglet pendant un
    /// téléchargement ne doit pas geler sa progression ni son
    /// aboutissement (rechargement des ROMs dans `Machine`, plus bas).
    pub fn poll(&mut self, cmd_sender: &Sender<MonitorMessage>) {
        let Some(rx) = &self.events else { return };
        // `try_recv` en boucle : plusieurs évènements peuvent s'être
        // accumulés entre deux trames (le thread ne les fait pas attendre).
        loop {
            match rx.try_recv() {
                Ok(InstallEvent::Progress(msg)) => {
                    if let Status::Running(lines) = &mut self.status {
                        lines.push(msg);
                    }
                }
                Ok(InstallEvent::Done(installed)) => {
                    self.status = Status::Done(installed);
                    self.events = None;
                    // Recharge les ROMs fraîchement installées et redémarre
                    // à froid — même chemin que la commande console "pc" :
                    // `Machine::power_cycle` rappelle `load_roms` en interne,
                    // donc reprend là où `main.rs` avait échoué faute de
                    // ROMs.
                    let _ = cmd_sender.send((MonitorCmd::PowerCycle, String::new(), String::new()));
                    break;
                }
                Ok(InstallEvent::Error(e)) => {
                    self.status = Status::Failed(e);
                    self.events = None;
                    break;
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    // Le thread s'est arrêté sans envoyer Done/Error (panique) :
                    // ne doit pas laisser l'écran bloqué sur "en cours" indéfiniment.
                    if matches!(self.status, Status::Running(_)) {
                        self.status = Status::Failed(
                            "Installation interrupted unexpectedly.".to_string(),
                        );
                    }
                    self.events = None;
                    break;
                }
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, machine: &Machine) {
        // Rien à demander/montrer si tout est déjà en place et que
        // l'utilisateur n'est pas au milieu d'une action sur cet écran
        // (Running/Done/Failed doit rester visible tel quel — voir plus
        // bas). `rom_status` mire directement la configuration réelle
        // (`config.toml` [rom], personnalisée ou non) : une ROM d'ailleurs
        // qui charge très bien compte comme installée, pas seulement celles
        // que CE bouton a lui-même posées — voir son commentaire.
        if matches!(self.status, Status::Idle)
            && let RomStatus::Installed { diagnostic_present } = machine.rom_status()
        {
            ui.colored_text_ok("ROMs installed.");
            if !diagnostic_present {
                ui.label(
                    "(Diagnostic ROM not installed — optional, only used in diagnostic mode.)",
                );
            }
            return;
        }

        ui.label(
            "Amstrad has neither granted nor refused permission to redistribute \
             these ROM images. Their use for emulation is widely tolerated within \
             the retro-computing community, but their legal status has not been \
             formally clarified.",
        );
        ui.add_space(6.0);
        ui.checkbox(
            &mut self.accepted,
            "I understand and accept the above before installing these ROMs.",
        );
        ui.add_space(6.0);

        let running = matches!(self.status, Status::Running(_));
        ui.add_enabled_ui(self.accepted && !running, |ui| {
            if ui.button("Install ROMs").clicked() {
                self.start_install();
            }
        });

        ui.add_space(6.0);
        match &self.status {
            Status::Idle => {}
            Status::Running(lines) => {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(lines.last().map(String::as_str).unwrap_or("Starting..."));
                });
            }
            Status::Done(installed) => {
                ui.colored_text_ok("Installation complete — the machine has been power-cycled.");
                for file in installed {
                    let origin_note = match file.previous_crc32 {
                        Some(prev) if prev == file.crc32 => {
                            " (matches the file already there)".to_string()
                        }
                        Some(_) => " (replaced a DIFFERENT file that was already there)".to_string(),
                        None => String::new(),
                    };
                    ui.label(format!(
                        "  {} — {} bytes, CRC32 {:#010x}{origin_note}",
                        file.filename, file.bytes, file.crc32
                    ));
                }
            }
            Status::Failed(e) => {
                ui.colored_text_err(format!("Installation failed: {e}"));
            }
        }
    }

    fn start_install(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        self.events = Some(rx);
        self.status = Status::Running(Vec::new());
        std::thread::spawn(move || {
            let progress_tx = tx.clone();
            let result = bytebox_core::rom_installer::install_everything(move |msg| {
                let _ = progress_tx.send(InstallEvent::Progress(msg.to_string()));
            });
            let _ = match result {
                Ok(installed) => tx.send(InstallEvent::Done(installed)),
                Err(e) => tx.send(InstallEvent::Error(e)),
            };
        });
    }
}

/// Petites aides de couleur, pour ne pas répéter le même `RichText` deux
/// fois ci-dessus.
trait ColoredText {
    fn colored_text_ok(&mut self, text: impl Into<String>);
    fn colored_text_err(&mut self, text: impl Into<String>);
}

impl ColoredText for egui::Ui {
    fn colored_text_ok(&mut self, text: impl Into<String>) {
        self.label(egui::RichText::new(text.into()).color(egui::Color32::from_rgb(120, 220, 120)));
    }

    fn colored_text_err(&mut self, text: impl Into<String>) {
        self.label(egui::RichText::new(text.into()).color(egui::Color32::from_rgb(230, 90, 90)));
    }
}
