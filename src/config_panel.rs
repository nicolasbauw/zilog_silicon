//! Panneau de configuration/médias (F6), superposé à l'image émulée par
//! `renderer.rs`, sur le même mécanisme que la barre de commande rapide
//! (F10, `console_panel.rs`) : les deux partagent le contexte wgpu de la
//! fenêtre principale (Plan V2.md, jalon M3).
//!
//! Reprend, derrière des champs qu'on peut cliquer plutôt que taper, ce qui
//! ne vivait jusqu'ici que dans `config.toml` ou les commandes console
//! (`disk`, `tape`, `blank`, `driveb`, `ram`, `vol`, `tapevol`). Chaque
//! champ pousse sur le même canal `MonitorCmd` que la console — `Machine`
//! ne voit aucune différence entre les deux façades.

use crate::keyboard_panel::KeyboardSettings;
use crate::renderer::CrtSettings;
use bytebox_core::app_log;
use bytebox_core::machine::Machine;
use bytebox_core::monitor::{MonitorCmd, MonitorMessage};
use std::path::Path;
use std::sync::mpsc::Sender;

/// Niveau de zoom demandé depuis le panneau. Le zoom est un état de
/// présentation (la fenêtre SDL2), pas un état de la machine émulée : il ne
/// passe donc pas par `MonitorCmd` comme le reste de ce panneau — c'est
/// `sdl::run` qui l'applique, exactement comme pour F1-F4.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ZoomChoice {
    X1,
    X2,
    X3,
    Fullscreen,
}

impl ZoomChoice {
    /// Forme attendue par `display.default_zoom` dans config.toml — voir
    /// `DisplayMode::from_config` côté `sdl.rs`, qui lit ces mêmes chaînes.
    fn as_config_str(self) -> &'static str {
        match self {
            ZoomChoice::X1 => "x1",
            ZoomChoice::X2 => "x2",
            ZoomChoice::X3 => "x3",
            ZoomChoice::Fullscreen => "fullscreen",
        }
    }
}

/// Onglet actif du panneau. Le shader CRT s'est retrouvé avec assez de
/// curseurs pour faire à lui seul déborder la fenêtre en x1 (voir Plan V2.md)
/// — le séparer du reste, qu'on ajuste rarement une fois réglé une bonne
/// fois, remet le panneau principal à une taille raisonnable sans rien
/// retirer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    General,
    Crt,
    Roms,
    Help,
}

/// Touches de fonction : sans équivalent côté machine émulée (contrairement
/// aux commandes ci-dessous), donc pas de constante partagée à réutiliser —
/// seul le README en gardait la liste jusqu'ici. À tenir en phase avec sa
/// section "Function keys" si l'une des deux change.
const FUNCTION_KEYS: &str = "\
F1 / F2 / F3   Window size x1 / x2 / x3
F4             Toggle fullscreen
F5             Toggle the CRT shader (RGB phosphor mask, scanlines)
F6             Toggle this configuration panel
F7             Toggle the virtual keyboard (clickable, overlaid on the
               emulator window)
F8             Step to next Z80 instruction
F9             Step to next video line
F10            Toggle the quick command bar (one input line, overlaid
               on the emulator window)
F11            Toggle the full console window (scrollable history,
               same commands as the quick command bar)
F12            Toggle the machine status window";

pub struct ConfigPanel {
    /// Nom de fichier pour une nouvelle disquette vierge ("blank") : pure
    /// saisie utilisateur, sans état côté machine dont le repartir — les
    /// autres champs de ce panneau (volume, banques RAM...) n'ont pas besoin
    /// d'un tel champ persistant, ils relisent l'état courant de `Machine`
    /// à chaque trame.
    blank_disk_name: String,
    blank_disk_drive_b: bool,
    tab: Tab,
    /// État de la case "Enable at startup" de l'onglet CRT — lu une fois à
    /// la construction depuis `config.toml` (`CrtConfig::enabled_at_startup`,
    /// voir `sdl::run`), puis réenregistré par le bouton "Save" avec le
    /// reste des réglages du shader. Ne fait pas partie de `CrtSettings` :
    /// contrairement aux curseurs, elle ne pilote rien côté GPU.
    crt_enabled_at_startup: bool,
    /// Téléchargement/installation des ROMs (onglet "ROMs") : voir
    /// `rom_install_panel.rs`. Vit ici, pas dans un état séparé côté
    /// `sdl.rs`, pour la même raison que `crt_enabled_at_startup` — propre à
    /// ce panneau, sans intérêt pour le reste de la boucle principale.
    roms: crate::rom_install_panel::RomInstallState,
}

impl ConfigPanel {
    pub fn new(crt_enabled_at_startup: bool) -> Self {
        Self {
            blank_disk_name: String::new(),
            blank_disk_drive_b: false,
            tab: Tab::General,
            crt_enabled_at_startup,
            roms: crate::rom_install_panel::RomInstallState::new(),
        }
    }

    /// Sélectionne l'onglet ROMs — appelé par `sdl.rs` au lancement si
    /// `Machine::load_roms` a échoué (voir `main.rs`) : plutôt que de
    /// laisser deviner où trouver l'écran d'installation, F6 s'ouvre
    /// directement dessus.
    pub fn open_on_roms_tab(&mut self) {
        self.tab = Tab::Roms;
    }

    /// Dessine le panneau ; `open` reflète et contrôle sa visibilité (la
    /// petite croix de la fenêtre egui peut la fermer, en plus de F6).
    /// `crt_settings`/`keyboard_settings` sont l'état courant du shader CRT
    /// et du clavier virtuel (relus à chaque trame depuis `Renderer`/
    /// `KeyboardPanel`, comme `machine` pour l'état de la machine) ; renvoie
    /// le zoom demandé cette trame s'il y en a un, et les réglages CRT/
    /// clavier à jour — inchangés si l'utilisateur n'a touché aucun curseur,
    /// donc toujours sûrs à réappliquer sans condition (voir `sdl.rs`).
    #[allow(clippy::too_many_arguments)]
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        machine: &Machine,
        cmd_sender: &Sender<MonitorMessage>,
        open: &mut bool,
        crt_settings: CrtSettings,
        keyboard_settings: KeyboardSettings,
        window_size: egui::Vec2,
        generation: u64,
        current_zoom: ZoomChoice,
        disk_indicator_enabled: &mut bool,
    ) -> (Option<ZoomChoice>, CrtSettings, KeyboardSettings) {
        let mut zoom = None;
        let mut crt_settings = crt_settings;
        let mut keyboard_settings = keyboard_settings;
        // Indépendant de l'onglet affiché : un changement d'onglet pendant
        // un téléchargement en cours ne doit pas en geler la progression ni
        // son aboutissement (voir `RomInstallState::poll`).
        self.roms.poll(cmd_sender);
        // Proportionnelle à la fenêtre CPC, pas juste une largeur : élargir
        // le panneau sans grossir police/boutons/curseurs ne sert à rien —
        // c'est justement leur petitesse en plein écran 4K qui gênait, pas
        // le manque de largeur. `scale` grossit tout le contenu (voir
        // `ui_scale::scaled_style`) ; `default_width` en découle, pour
        // rester cohérente avec un contenu qui a lui-même grossi.
        //
        // `generation` (incrémentée par `sdl.rs` sur confirmation d'un
        // redimensionnement réel — même mécanisme que `KeyboardPanel`, voir
        // son commentaire) fait partie de l'id de la fenêtre pour que ce
        // calcul se refasse à chaque changement de zoom, pas seulement à la
        // toute première ouverture.
        let scale = crate::ui_scale::content_scale(window_size);
        let default_width = 420.0 * scale;
        // Coin haut-gauche fixe : sans `default_pos`, egui place chaque
        // nouvelle fenêtre (un nouvel id à chaque changement de
        // `generation`, voir plus haut) via son algorithme de cascade
        // interne — d'où une position différente d'une ouverture à
        // l'autre au lieu de toujours la même, symptôme observé en
        // pratique.
        let margin = 8.0;
        egui::Window::new("Configuration")
            .id(egui::Id::new(("config_panel_window", generation)))
            .open(open)
            .resizable(true)
            .default_width(default_width)
            .default_pos(egui::pos2(margin, margin))
            .show(ctx, |ui| {
                ui.set_style(crate::ui_scale::scaled_style(ui.style(), scale));
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut self.tab, Tab::General, "General");
                    ui.selectable_value(&mut self.tab, Tab::Crt, "CRT Shader");
                    ui.selectable_value(&mut self.tab, Tab::Roms, "ROMs");
                    ui.selectable_value(&mut self.tab, Tab::Help, "Help");
                });
                ui.separator();
                match self.tab {
                    Tab::General => {
                        ui.heading("Media");
                        self.media_section(ui, machine, cmd_sender);
                        ui.separator();
                        ui.heading("Hardware");
                        Self::hardware_section(ui, machine, cmd_sender);
                        ui.separator();
                        ui.heading("Display");
                        Self::display_section(
                            ui,
                            &mut zoom,
                            &mut keyboard_settings,
                            current_zoom,
                            disk_indicator_enabled,
                        );
                        ui.separator();
                        ui.heading("Audio");
                        Self::audio_section(ui, machine, cmd_sender);
                        ui.separator();
                        Self::save_defaults_section(
                            ui,
                            machine,
                            current_zoom,
                            *disk_indicator_enabled,
                            &keyboard_settings,
                        );
                    }
                    Tab::Crt => {
                        Self::crt_section(ui, &mut crt_settings, &mut self.crt_enabled_at_startup)
                    }
                    Tab::Roms => self.roms.ui(ui, machine),
                    Tab::Help => Self::help_section(ui, scale),
                }
            });
        (zoom, crt_settings, keyboard_settings)
    }

    fn media_section(
        &mut self,
        ui: &mut egui::Ui,
        machine: &Machine,
        cmd_sender: &Sender<MonitorMessage>,
    ) {
        let dsk_path = machine.dsk_path();
        let fdc = machine.bus.fdc.borrow();
        Self::disk_drive_row(
            ui,
            "Drive A",
            &fdc.drive_a.current_filename,
            "",
            dsk_path,
            cmd_sender,
        );
        if fdc.drive_b_enabled {
            Self::disk_drive_row(
                ui,
                "Drive B",
                &fdc.drive_b.current_filename,
                "b",
                dsk_path,
                cmd_sender,
            );
        } else {
            ui.label("Drive B is disabled (see Hardware below).");
        }
        drop(fdc);

        ui.horizontal(|ui| {
            ui.label("New blank disk:");
            ui.text_edit_singleline(&mut self.blank_disk_name);
            ui.checkbox(&mut self.blank_disk_drive_b, "drive B");
            if ui
                .add_enabled(
                    !self.blank_disk_name.is_empty(),
                    egui::Button::new("Create"),
                )
                .clicked()
            {
                let arg2 = if self.blank_disk_drive_b { "b" } else { "" };
                let _ = cmd_sender.send((
                    MonitorCmd::Blank,
                    std::mem::take(&mut self.blank_disk_name),
                    arg2.to_string(),
                ));
            }
        });

        ui.separator();
        let tape = machine.bus.tape.borrow();
        let tape_label = tape
            .current_filename
            .as_deref()
            .map(Self::display_name)
            .unwrap_or("(empty)");
        ui.horizontal(|ui| {
            ui.label(format!("Tape: {tape_label}"));
            if ui.button("Insert…").clicked()
                && let Some(path) = Self::file_dialog(machine.cdt_path())
                    .add_filter("Tape image", &["cdt"])
                    .pick_file()
            {
                let _ = cmd_sender.send((
                    MonitorCmd::Tape,
                    path.to_string_lossy().into_owned(),
                    String::new(),
                ));
            }
            if ui
                .add_enabled(tape.current_filename.is_some(), egui::Button::new("Eject"))
                .clicked()
            {
                let _ = cmd_sender.send((MonitorCmd::Tape, "eject".to_string(), String::new()));
            }
        });
    }

    /// Une ligne "Drive A"/"Drive B" : nom du fichier inséré (ou vide),
    /// bouton d'insertion (sélecteur natif) et d'éjection. `drive_arg2` vaut
    /// "" pour le lecteur A, "b" pour le lecteur B — c'est le deuxième
    /// argument attendu par `MonitorCmd::Disk`.
    fn disk_drive_row(
        ui: &mut egui::Ui,
        label: &str,
        current_filename: &str,
        drive_arg2: &str,
        dsk_path: Option<&str>,
        cmd_sender: &Sender<MonitorMessage>,
    ) {
        let loaded = current_filename != "None";
        let shown = if loaded {
            Self::display_name(current_filename)
        } else {
            "(empty)"
        };
        ui.horizontal(|ui| {
            ui.label(format!("{label}: {shown}"));
            if ui.button("Insert…").clicked()
                && let Some(path) = Self::file_dialog(dsk_path)
                    .add_filter("Disk image", &["dsk"])
                    .pick_file()
            {
                let _ = cmd_sender.send((
                    MonitorCmd::Disk,
                    path.to_string_lossy().into_owned(),
                    drive_arg2.to_string(),
                ));
            }
            if ui.add_enabled(loaded, egui::Button::new("Eject")).clicked() {
                let _ = cmd_sender.send((
                    MonitorCmd::Disk,
                    "eject".to_string(),
                    drive_arg2.to_string(),
                ));
            }
        });
    }

    /// Nom affiché pour un chemin de disquette/cassette chargée : le seul
    /// nom de fichier, pas le chemin complet — trop long une fois résolu via
    /// `dsk_path`/`cdt_path` (`~/.bytebox/DSK/...`), et sans intérêt pour
    /// l'utilisateur ici, contrairement au sélecteur de fichier natif où le
    /// chemin complet reste affiché (rôle différent : y naviguer). Le chemin
    /// complet, lui, continue d'être ce que porte `current_filename` et ce
    /// qui est effectivement chargé — seul l'AFFICHAGE change.
    fn display_name(path: &str) -> &str {
        Path::new(path)
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or(path)
    }

    /// Sélecteur de fichier natif, ouvert par défaut dans `[file] dsk_path`
    /// (config.toml) plutôt que dans le répertoire courant du processus —
    /// c'est là que vivent les images disque/cassette la plupart du temps.
    /// Absent, `rfd` retombe sur son propre choix par défaut (généralement
    /// le répertoire courant, ou le dernier utilisé).
    fn file_dialog(dsk_path: Option<&str>) -> rfd::FileDialog {
        let dialog = rfd::FileDialog::new();
        match dsk_path {
            // `set_directory` attend un chemin réel : un `~` de tête n'y
            // serait pas plus compris que par `File::open`, voir
            // `bytebox_core::config::expand_tilde`.
            Some(dir) => dialog.set_directory(bytebox_core::config::expand_tilde(dir)),
            None => dialog,
        }
    }

    fn hardware_section(ui: &mut egui::Ui, machine: &Machine, cmd_sender: &Sender<MonitorMessage>) {
        let mut drive_b_enabled = machine.bus.fdc.borrow().drive_b_enabled;
        if ui
            .checkbox(&mut drive_b_enabled, "Enable drive B")
            .changed()
        {
            let arg = if drive_b_enabled { "on" } else { "off" };
            let _ = cmd_sender.send((MonitorCmd::DriveB, arg.to_string(), String::new()));
        }

        let mut mouse_enabled = machine.bus.mouse.borrow().enabled;
        if ui
            .checkbox(&mut mouse_enabled, "Enable mouse interface")
            .changed()
        {
            let arg = if mouse_enabled { "on" } else { "off" };
            let _ = cmd_sender.send((MonitorCmd::Mouse, arg.to_string(), String::new()));
        }

        let mut diagnostic_mode = machine.diagnostic_mode;
        if ui
            .checkbox(&mut diagnostic_mode, "Diagnostic ROM at slot 0F")
            .changed()
        {
            let arg = if diagnostic_mode { "on" } else { "off" };
            let _ = cmd_sender.send((MonitorCmd::DiagnosticMode, arg.to_string(), String::new()));
        }

        ui.horizontal(|ui| {
            ui.label("Extra RAM banks:");
            let mut banks = machine.extra_ram_banks();
            // 0 est la valeur normale d'un 6128 de base : les 128 Ko
            // standard (donc déjà une "banque haute") sont acquis d'office,
            // ce réglage ne compte que ce qu'une extension tierce
            // (Dk'tronics et consorts) ajouterait par-dessus — voir le
            // tooltip, pour la confusion venue une fois de "pourquoi 0 est
            // une valeur possible".
            let response = ui.add(egui::Slider::new(&mut banks, 0..=56)).on_hover_text(
                "On top of the 6128's standard 128 KB, which is already \
                     included and not affected by this setting. Only for a \
                     third-party expansion (Dk'tronics and the like) — 0, \
                     the default, is a plain, unexpanded 6128.",
            );
            if response.changed() {
                let _ =
                    cmd_sender.send((MonitorCmd::ExtraRamBanks, banks.to_string(), String::new()));
            }
        });

        ui.horizontal(|ui| {
            ui.label("Extra RAM banks and the Diagnostic ROM only apply at the next power cycle:");
            if ui.button("Power cycle now").clicked() {
                let _ = cmd_sender.send((MonitorCmd::PowerCycle, String::new(), String::new()));
            }
        });
    }

    fn display_section(
        ui: &mut egui::Ui,
        zoom: &mut Option<ZoomChoice>,
        keyboard_settings: &mut KeyboardSettings,
        current_zoom: ZoomChoice,
        disk_indicator_enabled: &mut bool,
    ) {
        ui.horizontal(|ui| {
            if ui.button("x1").clicked() {
                *zoom = Some(ZoomChoice::X1);
            }
            if ui.button("x2").clicked() {
                *zoom = Some(ZoomChoice::X2);
            }
            if ui.button("x3").clicked() {
                *zoom = Some(ZoomChoice::X3);
            }
            if ui.button("Fullscreen").clicked() {
                *zoom = Some(ZoomChoice::Fullscreen);
            }
        });

        ui.checkbox(
            disk_indicator_enabled,
            "Show a red dot on the emulator screen during disk access",
        );

        // Plus de bouton "Save as startup default" ici : le zoom courant
        // (avec l'indicateur ci-dessus) est maintenant enregistré par le
        // bouton "Save as defaults" tout en bas de cet onglet, avec le
        // reste — `current_zoom` reflète toujours l'état réel de la fenêtre
        // (`sdl.rs`), pas seulement ce qui a été choisi ici, donc ce label
        // seul suffit à savoir ce qui serait enregistré.
        ui.label(format!("Current zoom: {}", current_zoom.as_config_str()));

        // Taille par défaut du clavier virtuel (F7) : en fraction de la
        // hauteur de la fenêtre CPC, voir le commentaire de
        // `KeyboardPanel::ui` sur ce plafond de hauteur — trop grand en x1,
        // le clavier masque l'écran sur lequel on tape. Plus de bouton
        // "Save" ici non plus : enregistré par "Save as defaults", tout en
        // bas de l'onglet, comme le zoom ci-dessus.
        ui.horizontal(|ui| {
            let mut percent = keyboard_settings.default_size_percent * 100.0;
            let response = ui.add(egui::Slider::new(&mut percent, 10.0..=100.0).suffix(" %"));
            ui.label("Virtual keyboard (F7) default size");
            if response.changed() {
                keyboard_settings.default_size_percent = percent / 100.0;
            }
        });
    }

    /// Curseurs pour les six constantes de `renderer_crt.wgsl` — voir ses
    /// commentaires pour ce que chacune contrôle visuellement. Pas de
    /// `MonitorCmd` ici : ce sont des réglages de présentation, propres au
    /// `Renderer` de la fenêtre principale, au même titre que le zoom.
    fn crt_section(ui: &mut egui::Ui, settings: &mut CrtSettings, enabled_at_startup: &mut bool) {
        ui.add(
            egui::Slider::new(&mut settings.mask_cell_px, 0.5..=4.0).text("Mask cell size (px)"),
        );
        ui.add(egui::Slider::new(&mut settings.mask_strength, 0.0..=1.0).text("Mask strength"));
        ui.add(egui::Slider::new(&mut settings.mask_min, 0.0..=1.0).text("Mask min brightness"));
        // Plage étendue jusqu'à 24 : 10 était atteint en butée sans que les
        // scanlines paraissent encore assez marquées — c'était en fait le
        // symptôme d'un bug de période (voir `line_height` dans le shader),
        // mais la marge reste utile maintenant qu'il est corrigé.
        ui.add(
            egui::Slider::new(&mut settings.scanline_beam, 1.0..=24.0).text("Scanline beam width"),
        );
        ui.add(
            egui::Slider::new(&mut settings.scanline_strength, 0.0..=1.0).text("Scanline strength"),
        );
        ui.add(egui::Slider::new(&mut settings.beam_bloom, 0.05..=1.0).text("Beam bloom (bright)"));
        ui.add(egui::Slider::new(&mut settings.bright_boost, 1.0..=2.5).text("Brightness boost"));
        // Bornée à 1.0 : au-delà, le noyau à 5 colonnes du shader
        // (`BLUR_TAPS`) tronquerait visiblement la gaussienne. À 0, on
        // retrouve le pixel net d'origine.
        ui.add(
            egui::Slider::new(&mut settings.horizontal_blur, 0.0..=1.0)
                .text("Horizontal blur (px)"),
        );
        ui.checkbox(enabled_at_startup, "Enable at startup");
        ui.horizontal(|ui| {
            if ui.button("Reset to defaults").clicked() {
                *settings = CrtSettings::default();
            }
            // Enregistrés ensemble : les curseurs ci-dessus et la case
            // "Enable at startup" forment tous les deux `[crt]` dans
            // config.toml, juste par des chemins différents (`CrtSettings`
            // pour les curseurs, ce bool à part pour la case).
            if ui.button("Save").clicked() {
                let mut crt_config = settings.to_config();
                crt_config.enabled_at_startup = Some(*enabled_at_startup);
                match bytebox_core::config::save_crt_config(&crt_config) {
                    Ok(()) => app_log!("CRT settings saved to config.toml"),
                    Err(e) => app_log!("Could not save CRT settings: {e}"),
                }
            }
        });
    }

    /// Réunit ce que le README garde par ailleurs éclaté en plusieurs
    /// sections (touches de fonction, commandes console, commandes du
    /// moniteur) : pratique pour l'utilisateur, inutile de quitter
    /// l'émulateur — mais toujours du texte destiné à être lu, pas à être
    /// cliqué, donc pas sa place dans l'onglet "General".
    ///
    /// `bytebox_core::machine::HELP` fournit déjà les deux dernières
    /// sections telles quelles : c'est le même texte que la commande console
    /// `help` (F10/F11), une seule source pour les deux façades.
    fn help_section(ui: &mut egui::Ui, scale: f32) {
        // Non mis à l'échelle jusqu'ici, contrairement à `default_width`
        // (voir `ConfigPanel::ui`) : à fort zoom, le texte grossit (police
        // via `ui_scale::scaled_style`) mais la fenêtre de défilement
        // gardait la même hauteur en pixels, donc affichait nettement moins
        // de lignes à la fois — l'aide semblait "réduite de moitié".
        egui::ScrollArea::vertical()
            .max_height(420.0 * scale)
            .show(ui, |ui| {
                ui.heading("Function keys");
                ui.label(egui::RichText::new(FUNCTION_KEYS).monospace());
                ui.separator();
                ui.label(egui::RichText::new(bytebox_core::machine::HELP.trim_start()).monospace());
            });
    }

    fn audio_section(ui: &mut egui::Ui, machine: &Machine, cmd_sender: &Sender<MonitorMessage>) {
        let mut volume_pct = (machine.volume() * 100.0).round();
        ui.horizontal(|ui| {
            ui.label("Volume:");
            let response = ui.add(egui::Slider::new(&mut volume_pct, 0.0..=100.0).suffix(" %"));
            if response.changed() {
                let _ =
                    cmd_sender.send((MonitorCmd::Volume, volume_pct.to_string(), String::new()));
            }
        });

        let mut tape_pct = (machine.bus.psg.sound.tape_amplitude() * 100.0).round();
        ui.horizontal(|ui| {
            ui.label("Tape signal in mix:");
            let response = ui.add(egui::Slider::new(&mut tape_pct, 0.0..=100.0).suffix(" %"));
            if response.changed() {
                let _ = cmd_sender.send((
                    MonitorCmd::TapeAmplitude,
                    tape_pct.to_string(),
                    String::new(),
                ));
            }
        });
    }

    /// Un seul bouton pour tout l'onglet "General", tout en bas — remplace
    /// les anciens "Save as startup default" (zoom) et "Save" (taille du
    /// clavier virtuel), chacun limité à sa propre section : lecteur B,
    /// souris, RAM étendue, volume/cassette, zoom/indicateur disque et
    /// taille du clavier virtuel partagent maintenant un seul geste plutôt
    /// que plusieurs boutons "Save" épars dans chaque sous-section. Chaque
    /// section reste un fichier TOML à part (voir `write_config_section`
    /// côté core) — six écritures séquentielles, pas une transaction, mais
    /// chacune correcte indépendamment même si une autre échouait.
    fn save_defaults_section(
        ui: &mut egui::Ui,
        machine: &Machine,
        current_zoom: ZoomChoice,
        disk_indicator_enabled: bool,
        keyboard_settings: &KeyboardSettings,
    ) {
        if ui.button("Save as defaults").clicked() {
            let drives = bytebox_core::config::DriveConfig {
                drive_b: machine.bus.fdc.borrow().drive_b_enabled,
            };
            let mouse = bytebox_core::config::MouseConfig {
                enabled: machine.bus.mouse.borrow().enabled,
            };
            let memory = bytebox_core::config::MemoryConfig {
                extra_ram_banks: machine.extra_ram_banks(),
            };
            let audio = bytebox_core::config::AudioConfig {
                volume: Some(machine.volume()),
                tape_amplitude: Some(machine.bus.psg.sound.tape_amplitude()),
            };
            let display = bytebox_core::config::DisplayConfig {
                default_zoom: Some(current_zoom.as_config_str().to_string()),
                show_disk_access_indicator: Some(disk_indicator_enabled),
            };
            let keyboard = keyboard_settings.to_config();

            let results = [
                bytebox_core::config::save_drive_config(&drives),
                bytebox_core::config::save_mouse_config(&mouse),
                bytebox_core::config::save_memory_config(&memory),
                bytebox_core::config::save_audio_config(&audio),
                bytebox_core::config::save_display_config(&display),
                bytebox_core::config::save_keyboard_config(&keyboard),
            ];
            match results.into_iter().find_map(Result::err) {
                None => app_log!("General settings saved to config.toml"),
                Some(e) => app_log!("Could not save general settings: {e}"),
            }
        }
    }
}
