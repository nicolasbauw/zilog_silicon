//! Fenêtrage SDL2 : ouverture des fenêtres (écran principal et statut
//! machine), boucle d'événements (clavier, manette, raccourcis d'affichage),
//! rendu de la trame et régulation de la cadence. `main.rs` ne fait
//! qu'assembler une `Machine` prête à tourner et l'y confier via `run`.

use bytebox_core::app_log;
use bytebox_core::autotype::AutoTyper;
use bytebox_core::config::CrtConfig;
use crate::config_panel::{ConfigPanel, ZoomChoice};
use crate::console_log::ConsoleLog;
use crate::console_panel::QuickCommandBar;
use crate::console_window::ConsoleWindow;
use crate::keyboard_panel::{KeyboardPanel, KeyboardSettings};
use bytebox_core::machine::{self, Machine};
use crate::renderer::{CrtSettings, Renderer};
use bytebox_core::video;
use sdl2::event::Event;
use sdl2::mouse::MouseButton;
use sdl2::pixels::PixelFormatEnum;
use sdl2::surface::Surface;

/// Applique les valeurs enregistrées dans `config.toml` par-dessus les
/// valeurs par défaut du shader CRT, champ par champ : une section `[crt]`
/// partielle (ou absente) reste donc parfaitement valable. `CrtSettings`
/// elle-même (`renderer.rs`) ne connaît plus `CrtConfig` depuis que ce
/// fichier a été détaché de bytebox-core pour zilog silicon - cette
/// conversion vit ici plutôt, avec le reste du code encore spécifique au CPC.
pub(crate) fn crt_settings_from_config(crt: &CrtConfig) -> CrtSettings {
    let d = CrtSettings::default();
    CrtSettings {
        mask_cell_px: crt.mask_cell_px.unwrap_or(d.mask_cell_px),
        mask_min: crt.mask_min.unwrap_or(d.mask_min),
        mask_strength: crt.mask_strength.unwrap_or(d.mask_strength),
        scanline_beam: crt.scanline_beam.unwrap_or(d.scanline_beam),
        scanline_strength: crt.scanline_strength.unwrap_or(d.scanline_strength),
        beam_bloom: crt.beam_bloom.unwrap_or(d.beam_bloom),
        bright_boost: crt.bright_boost.unwrap_or(d.bright_boost),
        horizontal_blur: crt.horizontal_blur.unwrap_or(d.horizontal_blur),
    }
}

/// Réciproque de [`crt_settings_from_config`], pour l'enregistrement : tous
/// les champs sont renseignés, même ceux restés à leur valeur par défaut.
/// `enabled_at_startup` est hors du champ de `CrtSettings` (voir sa doc) :
/// laissé à la charge de l'appelant (`config_panel.rs`), qui le renseigne
/// depuis la case "Enable at startup" avant d'enregistrer.
pub(crate) fn crt_config_from_settings(settings: CrtSettings) -> CrtConfig {
    CrtConfig {
        mask_cell_px: Some(settings.mask_cell_px),
        mask_min: Some(settings.mask_min),
        mask_strength: Some(settings.mask_strength),
        scanline_beam: Some(settings.scanline_beam),
        scanline_strength: Some(settings.scanline_strength),
        beam_bloom: Some(settings.beam_bloom),
        bright_boost: Some(settings.bright_boost),
        horizontal_blur: Some(settings.horizontal_blur),
        enabled_at_startup: None,
    }
}

/// Pose `assets/bytebox_icon.png` comme icône de la fenêtre donnée.
/// Décodage en pur Rust via la crate `image` (plutôt que la feature
/// `"image"` de `sdl2`, qui dépend de la bibliothèque système
/// `libSDL2_image`) : pas de dépendance supplémentaire à installer sur la
/// machine qui compile ou qui exécute l'émulateur. SDL copie les pixels de
/// la surface dans la fenêtre dès `set_icon` : la surface elle-même n'a pas
/// besoin de survivre à cet appel.
///
/// Une icône n'est jamais affichée plus grande que quelques dizaines de
/// pixels (barre des tâches, alt-tab, titre de fenêtre) : `bytebox_icon.png`
/// (256×256) est une version pré-réduite de `bytebox.png` (1254×1254, gardé
/// pour d'autres usages éventuels — capture d'écran du README, etc.), pour
/// ne pas décoder une image ~24× plus grande que nécessaire, trois fois de
/// suite (une par fenêtre) à chaque lancement.
///
/// Une version antérieure distinguait ici les builds "officiels" (paquet
/// installé) des builds de développement, en recolorant en rouge le cadre
/// de l'icône pour ces derniers, selon une variable d'environnement
/// `BYTEBOX_PACKAGED_BUILD` positionnée par les recettes de packaging.
/// Abandonné : le mécanisme n'a jamais fonctionné de façon fiable en CI
/// (l'AppImage gardait le cadre rouge alors que la variable était bien vue
/// par le build script), pour un bénéfice qui ne valait pas cette
/// complexité. Qui veut distinguer son build local peut le faire
/// localement, dans son propre lanceur `.desktop`, sans que le code de
/// l'émulateur ait à en connaître quoi que ce soit.
fn set_window_icon(window: &mut sdl2::video::Window) -> Result<(), String> {
    // Sous macOS, SDL n'a pas de notion d'icône par fenêtre (NSWindow n'en
    // affiche pas dans sa barre de titre) : sa moulinette Cocoa fait donc
    // autre chose de cet appel — elle remplace l'icône du Dock/App Switcher
    // par cette surface brute, non masquée par le "squircle" arrondi
    // qu'applique macOS aux .icns des vraies applications (Finder,
    // Launchpad...). Résultat observé : l'icône passe d'arrondie (avant
    // lancement) à carrée et non lissée (une fois lancée). Ne rien faire ici
    // laisse macOS utiliser l'icône du bundle .app (Info.plist,
    // CFBundleIconFile) du début à la fin — la même partout, lancée ou non.
    if cfg!(target_os = "macos") {
        return Ok(());
    }

    // Embarquée dans le binaire à la compilation (`include_bytes!`), pas lue
    // sur le disque à l'exécution : un chemin relatif au répertoire courant
    // ("assets/bytebox_icon.png") ne pointe nulle part de fiable une fois
    // installée par un paquet (`Exec=bytebox` sans `Path=` particulier dans
    // le `.desktop`, ou lancée depuis n'importe où via `$PATH`) — et
    // contrairement aux ROMs, rien n'empêche de la distribuer telle quelle.
    let img = image::load_from_memory(include_bytes!("../assets/bytebox_icon.png"))
        .map_err(|e| e.to_string())?
        .into_rgba8();
    let (width, height) = img.dimensions();
    let mut pixels = img.into_raw();
    let pitch = width * 4;
    let surface = Surface::from_data(&mut pixels, width, height, pitch, PixelFormatEnum::RGBA32)
        .map_err(|e| e.to_string())?;
    window.set_icon(&surface);
    Ok(())
}

/// Niveau de zoom de la fenêtre d'affichage (touches F1-F4, ou
/// `default_zoom` dans config.toml).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum DisplayMode {
    Normal,
    X2,
    X3,
    Fullscreen,
}

impl DisplayMode {
    /// Lit la valeur de `default_zoom` (config.toml, section [display]).
    /// Une valeur absente ou non reconnue retombe silencieusement sur la
    /// taille normale (mais journalise un avertissement si elle est
    /// présente et non reconnue : mieux vaut le signaler qu'échouer
    /// silencieusement sur une faute de frappe dans le fichier).
    fn from_config(value: Option<&str>) -> Self {
        match value {
            None => DisplayMode::Normal,
            Some("x1") => DisplayMode::Normal,
            Some("x2") => DisplayMode::X2,
            Some("x3") => DisplayMode::X3,
            Some("fullscreen") => DisplayMode::Fullscreen,
            Some(other) => {
                app_log!(
                    "Config: display.default_zoom='{other}' not recognized (expected x1, x2, x3 or fullscreen), using x1."
                );
                DisplayMode::Normal
            }
        }
    }

    /// Vers `ZoomChoice` (`config_panel.rs`) : le panneau F6 affiche le zoom
    /// courant et propose de l'enregistrer comme défaut au démarrage, sans
    /// dupliquer ses propres boutons x1/x2/x3/Fullscreen pour ça.
    fn to_zoom_choice(self) -> ZoomChoice {
        match self {
            DisplayMode::Normal => ZoomChoice::X1,
            DisplayMode::X2 => ZoomChoice::X2,
            DisplayMode::X3 => ZoomChoice::X3,
            DisplayMode::Fullscreen => ZoomChoice::Fullscreen,
        }
    }
}

/// Applique un niveau de zoom à la fenêtre d'affichage principale.
///
/// Le letterboxing/pillarboxing qui conserve le ratio d'aspect (4:3) n'est
/// plus automatique comme au temps de `Canvas::set_logical_size` : c'est
/// désormais `Renderer::present` qui recalcule un viewport à chaque trame
/// (voir `renderer.rs`). Cette fonction ne fait donc plus que choisir la
/// taille de fenêtre ou le plein écran.
fn apply_display_mode(window: &mut sdl2::video::Window, mode: DisplayMode) {
    match mode {
        DisplayMode::Fullscreen => {
            let _ = window.set_fullscreen(sdl2::video::FullscreenType::Desktop);
            return;
        }
        DisplayMode::Normal | DisplayMode::X2 | DisplayMode::X3 => {
            let _ = window.set_fullscreen(sdl2::video::FullscreenType::Off);
        }
    }
    let factor = match mode {
        DisplayMode::Normal => 1,
        DisplayMode::X2 => 2,
        DisplayMode::X3 => 3,
        DisplayMode::Fullscreen => unreachable!("traite plus haut, avec un retour anticipe"),
    };
    let _ = window.set_size(
        video::SCREEN_WIDTH as u32 * factor,
        video::SCREEN_HEIGHT as u32 * factor,
    );
    window.set_position(
        sdl2::video::WindowPos::Centered,
        sdl2::video::WindowPos::Centered,
    );
}

/// Traduit un bouton SDL2 vers l'index attendu par `Mouse::set_button`
/// (0 = gauche, 1 = droit, 2 = milieu — voir `bytebox_core::mouse`). Le clic
/// gauche/milieu qui capture ou relâche la souris n'atteint jamais cette
/// fonction (interceptés avant, voir la boucle d'évènements) ; ce qui reste
/// couvre le clic droit et les boutons latéraux, ignorés faute d'équivalent
/// dans le protocole à 3 boutons.
fn mouse_button_index(button: MouseButton) -> Option<u8> {
    match button {
        MouseButton::Left => Some(0),
        MouseButton::Right => Some(1),
        MouseButton::Middle => Some(2),
        _ => None,
    }
}

/// Traduit un `Keycode` SDL2 vers celui, propre à `core`, que
/// `Psg::set_key_state` attend (voir `bytebox_core::keys`). `core` ne
/// connaît pas SDL2 : c'est à cette façade de faire cette traduction, pas
/// à `core` de l'accepter tel quel. `None` pour tout ce que la matrice
/// clavier du CPC ne mappe pas — la très large majorité des touches SDL2.
fn to_core_keycode(kc: sdl2::keyboard::Keycode) -> Option<bytebox_core::keys::Keycode> {
    use bytebox_core::keys::Keycode as K;
    use sdl2::keyboard::Keycode as SdlK;
    Some(match kc {
        SdlK::A => K::A,
        SdlK::B => K::B,
        SdlK::C => K::C,
        SdlK::D => K::D,
        SdlK::E => K::E,
        SdlK::F => K::F,
        SdlK::G => K::G,
        SdlK::H => K::H,
        SdlK::I => K::I,
        SdlK::J => K::J,
        SdlK::K => K::K,
        SdlK::L => K::L,
        SdlK::M => K::M,
        SdlK::N => K::N,
        SdlK::O => K::O,
        SdlK::P => K::P,
        SdlK::Q => K::Q,
        SdlK::R => K::R,
        SdlK::S => K::S,
        SdlK::T => K::T,
        SdlK::U => K::U,
        SdlK::V => K::V,
        SdlK::W => K::W,
        SdlK::X => K::X,
        SdlK::Y => K::Y,
        SdlK::Z => K::Z,
        SdlK::Num0 => K::Num0,
        SdlK::Num1 => K::Num1,
        SdlK::Num2 => K::Num2,
        SdlK::Num3 => K::Num3,
        SdlK::Num4 => K::Num4,
        SdlK::Num5 => K::Num5,
        SdlK::Num6 => K::Num6,
        SdlK::Num7 => K::Num7,
        SdlK::Num8 => K::Num8,
        // Dedicated "!" key on a standard PC keyboard (distinct from the
        // physical "8" key, already mapped above): same CPC target, since
        // "8"/"!" share one CPC key whose unshifted legend is "!" — mirrors
        // bytebox-web/src/keyboard.rs's own `"!" => K::Num8` mapping.
        SdlK::Exclaim => K::Num8,
        SdlK::Num9 => K::Num9,
        SdlK::Up => K::Up,
        SdlK::Down => K::Down,
        SdlK::Left => K::Left,
        SdlK::Right => K::Right,
        SdlK::Return => K::Return,
        SdlK::Backspace => K::Backspace,
        SdlK::Delete => K::Delete,
        SdlK::Escape => K::Escape,
        SdlK::Tab => K::Tab,
        SdlK::Space => K::Space,
        SdlK::CapsLock => K::CapsLock,
        SdlK::LShift => K::LShift,
        SdlK::RShift => K::RShift,
        SdlK::LCtrl => K::LCtrl,
        SdlK::RCtrl => K::RCtrl,
        SdlK::LAlt => K::LAlt,
        SdlK::RAlt => K::RAlt,
        SdlK::Minus => K::Minus,
        SdlK::Equals => K::Equals,
        SdlK::Plus => K::Plus,
        SdlK::Colon => K::Colon,
        SdlK::Slash => K::Slash,
        SdlK::Percent => K::Percent,
        SdlK::Comma => K::Comma,
        SdlK::Period => K::Period,
        SdlK::Semicolon => K::Semicolon,
        SdlK::RightParen => K::RightParen,
        SdlK::Kp0 => K::Kp0,
        SdlK::Kp1 => K::Kp1,
        SdlK::Kp2 => K::Kp2,
        SdlK::Kp3 => K::Kp3,
        SdlK::Kp4 => K::Kp4,
        SdlK::Kp5 => K::Kp5,
        SdlK::Kp6 => K::Kp6,
        SdlK::Kp7 => K::Kp7,
        SdlK::Kp8 => K::Kp8,
        SdlK::Kp9 => K::Kp9,
        SdlK::KpEnter => K::KpEnter,
        SdlK::KpPeriod => K::KpPeriod,
        SdlK::KpMultiply => K::KpMultiply,
        SdlK::KpDivide => K::KpDivide,
        SdlK::KpEquals => K::KpEquals,
        SdlK::KpPlus => K::KpPlus,
        SdlK::KpMinus => K::KpMinus,
        _ => return None,
    })
}

/// Même principe que `to_core_keycode`, pour les quelques touches gérées
/// par position physique plutôt que par caractère (voir la doc de
/// `Psg::set_key_state_scancode`).
fn to_core_scancode(sc: sdl2::keyboard::Scancode) -> Option<bytebox_core::keys::Scancode> {
    use bytebox_core::keys::Scancode as S;
    use sdl2::keyboard::Scancode as SdlS;
    Some(match sc {
        SdlS::Apostrophe => S::Apostrophe,
        SdlS::LeftBracket => S::LeftBracket,
        SdlS::Grave => S::Grave,
        SdlS::RightBracket => S::RightBracket,
        SdlS::NonUsBackslash => S::NonUsBackslash,
        SdlS::Num8 => S::Num8,
        _ => return None,
    })
}

/// Ouvre les fenêtres SDL2 et fait tourner la machine jusqu'à la fermeture.
/// `machine` doit déjà avoir ses ROMs (et, le cas échéant, sa disquette)
/// chargées ; `autotyper`, s'il y en a un, tape sa commande au clavier
/// émulé au fil des trames.
pub fn run(
    mut machine: Machine,
    mut autotyper: Option<AutoTyper>,
    roms_missing: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    // 3. Initialisation de SDL2
    let sdl_context = sdl2::init()?;
    let video_subsystem = sdl_context.video()?;
    let controller_subsystem = sdl_context.game_controller()?;
    let joystick_subsystem = sdl_context.joystick()?;

    // Tentative d'ouverture de la première manette disponible. Réutilisée
    // aussi bien ici, au démarrage, que dans la boucle d'événements sur
    // branchement à chaud (voir `Event::ControllerDeviceAdded` plus bas) :
    // une seule manette active à la fois, comme au démarrage.
    let open_first_controller =
        |subsystem: &sdl2::GameControllerSubsystem| -> Option<sdl2::controller::GameController> {
            let num_joysticks = subsystem.num_joysticks().unwrap_or(0);
            for i in 0..num_joysticks {
                if subsystem.is_game_controller(i) {
                    match subsystem.open(i) {
                        Ok(c) => {
                            app_log!("Controller opened: {}", c.name());
                            return Some(c);
                        }
                        Err(e) => app_log!("Failed to open controller {}: {}", i, e),
                    }
                }
            }
            None
        };
    let mut active_controller = open_first_controller(&controller_subsystem);

    // Repli sur l'API Joystick brute : `is_game_controller` ne dit vrai que
    // pour les manettes présentes dans la base de mappings intégrée à SDL2
    // (SDL_GameControllerDB), qui ne couvre que les modèles les plus
    // répandus. Une manette USB "générique" ou simplement absente de cette
    // base (constaté avec une Logitech Precision Gamepad : vue par SDL comme
    // joystick à 2 axes/10 boutons, mais `is_game_controller` renvoie false)
    // n'était donc jamais ouverte, silencieusement — le symptôme rapporté
    // ("une manette USB n'est pas détectée").
    //
    // Sans mapping, impossible de connaître la disposition réelle des
    // boutons (pas de nom "A"/"B"/DPad comme sur l'API Controller) : schéma
    // générique adapté à l'usage CPC (croix directionnelle + 1-3 boutons de
    // tir), suffisant pour la plupart des manettes de jeu simples. Une
    // manette dotée d'un vrai stick analogique fonctionnera aussi (axes 0/1
    // pris comme abscisse/ordonnée), juste sans les autres axes/boutons.
    let open_first_joystick = |controller_subsystem: &sdl2::GameControllerSubsystem,
                                joystick_subsystem: &sdl2::JoystickSubsystem|
     -> Option<sdl2::joystick::Joystick> {
        let num_joysticks = joystick_subsystem.num_joysticks().unwrap_or(0);
        for i in 0..num_joysticks {
            // Déjà pris en charge par l'API Controller : pas de double emploi.
            if controller_subsystem.is_game_controller(i) {
                continue;
            }
            match joystick_subsystem.open(i) {
                Ok(j) => {
                    app_log!("Joystick opened (generic mapping): {}", j.name());
                    return Some(j);
                }
                Err(e) => app_log!("Failed to open joystick {}: {}", i, e),
            }
        }
        None
    };
    let mut active_joystick = if active_controller.is_none() {
        open_first_joystick(&controller_subsystem, &joystick_subsystem)
    } else {
        None
    };

    // 4. Ouverture de la sortie audio. Une machine sans carte son utilisable
    // ne doit pas empêcher l'émulateur de démarrer : on continue en silence.
    let mut audio = match crate::audio::Audio::new(&sdl_context) {
        Ok(a) => Some(a),
        Err(e) => {
            app_log!("Audio disabled: {e}");
            None
        }
    };

    // 5. Fenêtre "machine status" (F12), cachée par défaut. Son contenu est
    // un panneau egui (voir status_panel.rs, Plan V2.md jalon M1) : plus de
    // police à charger depuis le disque, egui embarque la sienne.
    let mut debug_visible = false;
    let mut debug_window = video_subsystem
        .window("Amstrad CPC 6128 - Machine Status", 900, 1250)
        .position_centered()
        .hidden()
        .resizable()
        .metal_view()
        // Reste visible par-dessus la fenêtre principale même en plein
        // écran (F4) : sans ça, recliquer sur l'émulateur repasse cette
        // fenêtre derrière, alors qu'elle sert justement à être consultée
        // en même temps que lui.
        .always_on_top()
        .build()?;
    // Même icône que la fenêtre principale : sans cet appel, le gestionnaire
    // de fenêtres retombe sur son icône par défaut (le logo Wayland sous
    // KDE/Wayland) pour cette fenêtre-ci uniquement.
    if let Err(e) = set_window_icon(&mut debug_window) {
        app_log!("Can't set debug window icon: {e}");
    }
    let debug_window_id = debug_window.id();
    let mut status_panel =
        crate::status_panel::StatusPanel::new(debug_window, "bytebox status panel device")?;

    // Console complète (F11), cachée par défaut, sur le même modèle que la
    // fenêtre de statut ci-dessus : elle remplace entièrement la console
    // pilotée depuis le terminal qui a lancé l'émulateur (voir
    // console_window.rs, Plan V2.md jalon M2) — il n'y en a plus d'autre.
    let mut console_window_visible = false;
    let mut console_win = video_subsystem
        .window("Amstrad CPC 6128 - Console", 900, 700)
        .position_centered()
        .hidden()
        .resizable()
        .metal_view()
        // Voir le commentaire équivalent sur la fenêtre de statut ci-dessus.
        .always_on_top()
        .build()?;
    if let Err(e) = set_window_icon(&mut console_win) {
        app_log!("Can't set console window icon: {e}");
    }
    let console_window_id = console_win.id();
    let mut console_window = ConsoleWindow::new(console_win)?;

    // Identifiant de build (build.rs) : le hash court du commit quand il est
    // disponible (dépôt cloné — `cargo build` local, packaging/PKGBUILD-git,
    // ou nos jobs CI, qui clonent via `actions/checkout`), sinon le numéro
    // de version de Cargo.toml (archive d'une release taguée, dont le
    // tarball n'embarque pas `.git` — packaging/PKGBUILD, Homebrew).
    let build_id = option_env!("BYTEBOX_GIT_HASH").unwrap_or(env!("CARGO_PKG_VERSION"));
    let window_title = if machine.diagnostic_mode {
        format!("ByteBox - {build_id} - Diag ROM")
    } else {
        format!("ByteBox - {build_id}")
    };

    let mut window = video_subsystem
        .window(
            &window_title,
            video::SCREEN_WIDTH as u32,
            video::SCREEN_HEIGHT as u32,
        )
        .position_centered()
        // Sans effet hors macOS ; là-bas, condition requise pour que wgpu
        // puisse créer une surface à partir du handle de cette fenêtre.
        .metal_view()
        // Sans .resizable(), le bouton vert "plein écran" de la barre de
        // titre macOS reste grisé (NSWindow doit pouvoir se redimensionner
        // pour que le bureau propose une transition plein écran) — F1-F4
        // redimensionnent déjà la fenêtre par le code, et `renderer.resize`
        // (voir plus bas, sur SizeChanged/Resized) recalcule le letterboxing
        // pour n'importe quelle taille de surface, pas seulement les
        // quatre zooms prédéfinis : un redimensionnement à la souris marche
        // donc aussi bien qu'un F1-F4, sans code supplémentaire.
        .resizable()
        .build()?;
    // Non bloquant : une icône manquante ou illisible ne doit pas empêcher
    // l'émulateur de démarrer, la fenêtre garde alors l'icône par défaut du
    // système de fenêtrage.
    if let Err(e) = set_window_icon(&mut window) {
        app_log!("Can't set window icon: {e}");
    }
    let mut renderer = Renderer::new(
        window,
        video::SCREEN_WIDTH,
        video::SCREEN_HEIGHT,
        video::PIXELS_PER_SCANLINE as f32,
    )?;
    // Une section [crt] dans config.toml (écrite par le bouton du panneau F6)
    // outrepasse les valeurs par défaut du shader, champ par champ.
    renderer.set_crt_settings(crt_settings_from_config(machine.crt_config()));
    if machine.crt_config().enabled_at_startup.unwrap_or(false) {
        renderer.set_crt_enabled(true);
    }
    let mut current_zoom = DisplayMode::from_config(machine.default_zoom());
    apply_display_mode(renderer.window_mut(), current_zoom);
    // Point rouge superposé à l'écran pendant un accès disque (panneau F6,
    // config_panel.rs) : lu une fois au démarrage depuis config.toml, puis
    // modifiable en direct sans redémarrage — contrairement au zoom/CRT, pas
    // besoin de le faire transiter par le petit ballet Option/tuple de
    // `ConfigPanel::ui` (voir son commentaire) : rien d'autre n'emprunte
    // cette variable ailleurs dans la trame.
    let mut disk_indicator_enabled = machine.show_disk_access_indicator();
    let main_window_id = renderer.window().id();
    let mut event_pump = sdl_context.event_pump()?;

    // Message d'information éphémère (osd.rs) : manette connectée, shader
    // CRT (F5) activé/désactivé... `active_controller` a été résolue plus
    // haut, avant que `renderer` (et donc un contexte egui où afficher quoi
    // que ce soit) n'existe — le message correspondant n'est déclenché
    // qu'ici, une fois `osd` construite.
    let mut osd = crate::osd::Osd::new();
    if let Some(controller) = &active_controller {
        osd.show(format!("Controller connected: {}", controller.name()));
    } else if let Some(joystick) = &active_joystick {
        osd.show(format!("Controller connected: {}", joystick.name()));
    }

    // Barre de commande rapide (F10, console_panel.rs) et console complète
    // (F11, console_window.rs) : deux vues du même historique
    // (Plan V2.md, jalon M2), alimentées via le même canal MonitorCmd.
    let mut quick_bar_visible = false;
    let mut quick_bar = QuickCommandBar::new();
    let mut console_log = ConsoleLog::new();
    let cmd_sender = machine.command_sender();

    // Panneau de configuration/médias (F6, config_panel.rs, Plan V2.md
    // jalon M3) : superposé à la fenêtre principale comme la barre rapide
    // ci-dessus, même canal MonitorCmd. `config_panel_generation` change à
    // chaque redimensionnement confirmé de la fenêtre CPC pendant qu'il est
    // ouvert (même mécanisme que `keyboard_panel_generation` plus bas, voir
    // son commentaire) : sans ça, sa taille — proportionnelle au zoom —
    // resterait celle calculée pour l'ancienne taille de fenêtre.
    // `roms_missing` (voir `main.rs`) : le panneau s'ouvre directement sur
    // son onglet ROMs plutôt que de laisser deviner où trouver l'écran
    // d'installation — c'est le seul cas où F6 s'ouvre de lui-même, sans
    // appui sur la touche.
    let mut config_panel_visible = roms_missing;
    let mut config_panel =
        ConfigPanel::new(machine.crt_config().enabled_at_startup.unwrap_or(false));
    if roms_missing {
        config_panel.open_on_roms_tab();
    }
    let mut config_panel_generation: u64 = 0;

    // Clavier virtuel (F7, keyboard_panel.rs, Plan V2.md jalon M5) : même
    // mécanisme d'overlay que les deux panneaux ci-dessus. `pressed_keys`
    // retient l'ensemble PSG appliqué à la trame précédente, pour ne
    // presser/relâcher que ce qui change d'une trame à l'autre plutôt que de
    // rejouer tout l'état à chaque fois. `keyboard_panel_generation` change à
    // chaque réouverture (F7) : voir le commentaire de `KeyboardPanel::ui`
    // sur pourquoi elle ne peut pas se suivre elle-même.
    let mut keyboard_panel_visible = false;
    let mut keyboard_panel = KeyboardPanel::new();
    let mut keyboard_panel_generation: u64 = 0;
    let mut pressed_keys: std::collections::HashSet<(usize, u8)> = std::collections::HashSet::new();
    let mut keyboard_settings = KeyboardSettings::from_config(machine.keyboard_config());
    // Voir le commentaire sur les évènements Enter/Leave plus bas.
    let mut mouse_over_main_window = false;
    // Capture souris de la souris logicielle ByteBox (voir `bytebox_core::
    // mouse`) : clic gauche sur la fenêtre principale pour capturer (mode
    // relatif SDL2, curseur système déjà masqué au survol — voir plus bas),
    // clic milieu pour relâcher. Indépendant de `mouse_over_main_window`,
    // qui ne sert qu'à masquer le curseur : une fois capturée, la souris
    // n'émet plus d'évènements Enter/Leave fiables (le pointeur ne "sort"
    // jamais vraiment de la fenêtre en mode relatif), donc cet état a besoin
    // de son propre booléen plutôt que de se déduire de mouse_over_main_window.
    let mut mouse_captured = false;
    // Tout ce qui a été journalisé avant l'ouverture des fenêtres (bannière
    // de démarrage, config invalide, --disk/--tape en ligne de commande...)
    // attendait dans la file globale (voir applog.rs) : on le récupère ici,
    // rien n'est perdu.
    for line in bytebox_core::applog::drain() {
        console_log.push_output(&line);
    }

    let mut frame_buffer = vec![0u8; video::SCREEN_WIDTH * video::SCREEN_HEIGHT * 3];
    // Deux fois la durée d'une trame standard (312 lignes de 256 ticks) : simple
    // garde-fou pour ne pas bloquer la boucle SDL si le VSYNC n'arrive jamais.
    let max_ticks_per_frame: u32 = 2 * 79_872;
    let mut running = true;

    // Cadence d'exécution. On vise une échéance absolue plutôt que de dormir une
    // durée fixe : avec une attente fixe, la période réelle vaut "temps de calcul
    // + attente" et la machine émulée tourne durablement trop lentement.
    // La durée d'une trame est celle du temps émulé qu'elle a réellement
    // consommé (voir machine::emulated_duration) : un logiciel qui reprogramme
    // le CRTC en cours de trame change bien la durée de trame, mais ses
    // registres ne la décrivent plus une fois la trame finie.
    let mut next_frame = std::time::Instant::now();
    // Trame arrêtée (émulation en pause) : cadence de repli pour continuer à
    // rafraîchir l'écran et lire la console sans tourner à vide.
    const PAUSED_FRAME: std::time::Duration = std::time::Duration::from_millis(20);
    let mut measure_start = std::time::Instant::now();
    let mut measured_ticks: u64 = 0;
    let mut late_frames: u32 = 0;

    while running {
        for event in event_pump.poll_iter() {
            // `egui_sdl2_event::translate_virtual_key_code` ne connaît que
            // `Keycode::Return`, pas `Keycode::KpEnter` (absent de sa table
            // de traduction, silencieusement ignoré) : la touche Entrée du
            // pavé numérique ne validait donc jamais un champ de saisie
            // egui (F10/F11/F12), gênant pour une saisie massive au pavé
            // numérique. Corrigé ici plutôt qu'en patchant la dépendance :
            // un événement réécrit, seulement pour les panneaux egui
            // ci-dessous — surtout PAS pour le passthrough clavier CPC plus
            // bas dans cette boucle, qui doit continuer à distinguer
            // KpEnter (une vraie touche CPC à part, (0,6) dans la matrice)
            // de Return.
            let egui_event = match &event {
                Event::KeyDown {
                    timestamp,
                    window_id,
                    keycode: Some(sdl2::keyboard::Keycode::KpEnter),
                    scancode,
                    keymod,
                    repeat,
                } => Event::KeyDown {
                    timestamp: *timestamp,
                    window_id: *window_id,
                    keycode: Some(sdl2::keyboard::Keycode::Return),
                    scancode: *scancode,
                    keymod: *keymod,
                    repeat: *repeat,
                },
                Event::KeyUp {
                    timestamp,
                    window_id,
                    keycode: Some(sdl2::keyboard::Keycode::KpEnter),
                    scancode,
                    keymod,
                    repeat,
                } => Event::KeyUp {
                    timestamp: *timestamp,
                    window_id: *window_id,
                    keycode: Some(sdl2::keyboard::Keycode::Return),
                    scancode: *scancode,
                    keymod: *keymod,
                    repeat: *repeat,
                },
                _ => event.clone(),
            };

            // Alimente egui même quand le panneau est caché : sinon la
            // première trame après un F12 le retrouverait avec un état
            // d'entrée périmé (position de souris, modificateurs...). Sans
            // risque de fuite vers le CPC : `status_panel` et
            // `console_window` sont des fenêtres SDL2 séparées, et
            // `EguiSDL2State::sdl2_input_to_egui` filtre les événements sur
            // l'identifiant de fenêtre — une frappe faite dans la fenêtre
            // principale (jouer, taper au BASIC...) ne les atteint jamais.
            status_panel.handle_event(&egui_event);
            console_window.handle_event(&egui_event);
            // La barre rapide F10, le panneau de configuration F6 et le
            // clavier virtuel F7, eux, sont superposés à la fenêtre
            // PRINCIPALE (même wgpu, voir renderer.rs) : ce filtre par
            // fenêtre ne les protège donc de rien, toute frappe faite en
            // jouant a le même identifiant de fenêtre qu'eux. Sans la
            // condition ci-dessous, ces frappes s'accumulaient en silence
            // dans la file d'entrée d'egui tant qu'aucun des trois n'était
            // affiché (rien ne la vidait, `Renderer::present` ne traitant
            // cette file que lorsqu'un overlay est actif) et se
            // déversaient d'un coup, à l'ouverture, dans le premier champ
            // de saisie venu.
            if quick_bar_visible || config_panel_visible || keyboard_panel_visible {
                renderer.handle_event(&egui_event);
            }
            match event {
                Event::Quit { .. } => {
                    running = false;
                }
                Event::Window {
                    win_event: sdl2::event::WindowEvent::Close,
                    window_id,
                    ..
                } => {
                    if window_id == debug_window_id {
                        debug_visible = false;
                        status_panel.window_mut().hide();
                    } else if window_id == console_window_id {
                        console_window_visible = false;
                        console_window.window_mut().hide();
                    } else if window_id == main_window_id {
                        running = false;
                    }
                }
                // wgpu ne suit pas tout seul le redimensionnement de la
                // fenêtre (contrairement à `Canvas::set_logical_size`,
                // disparu avec lui) : il faut reconfigurer la surface à sa
                // nouvelle taille, sous peine de la dessiner étirée ou
                // tronquée jusqu'à la trame suivante.
                Event::Window {
                    win_event:
                        sdl2::event::WindowEvent::SizeChanged(..)
                        | sdl2::event::WindowEvent::Resized(..),
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    renderer.resize();
                    // Le clavier virtuel (F7), s'il est ouvert, calcule sa
                    // position/taille par défaut une seule fois par
                    // ouverture (voir `KeyboardPanel::ui`) : sans le faire
                    // suivre ici, un changement d'échelle en cours de
                    // session le laisserait à la taille calculée pour
                    // l'ancienne fenêtre. Faire changer
                    // `keyboard_panel_generation` force egui à le traiter
                    // comme une réouverture fraîche (nouvel id, donc
                    // nouveau calcul de position/taille) — l'équivalent
                    // visuel d'un F7/F7 mais sans le clignotement d'un vrai
                    // cycle fermé/rouvert.
                    //
                    // Fait ICI plutôt que dans les gestionnaires F1-F4 :
                    // `window.set_size()` (appelé par `apply_display_mode`)
                    // ne garantit pas que le redimensionnement SDL/OS soit
                    // déjà effectif au retour de l'appel — le lire
                    // immédiatement après pouvait encore donner l'ancienne
                    // taille pendant une trame, plaçant le clavier comme si
                    // la fenêtre n'avait pas changé. Cet évènement, lui, ne
                    // se déclenche qu'une fois le redimensionnement réel
                    // confirmé.
                    if keyboard_panel_visible {
                        keyboard_panel_generation += 1;
                    }
                    // Même raisonnement pour le panneau de configuration
                    // (F6), dont la largeur par défaut suit elle aussi le
                    // zoom — voir `ConfigPanel::ui`.
                    if config_panel_visible {
                        config_panel_generation += 1;
                    }
                }
                Event::Window {
                    win_event:
                        sdl2::event::WindowEvent::SizeChanged(..)
                        | sdl2::event::WindowEvent::Resized(..),
                    window_id,
                    ..
                } if window_id == debug_window_id => {
                    status_panel.resize();
                }
                Event::Window {
                    win_event:
                        sdl2::event::WindowEvent::SizeChanged(..)
                        | sdl2::event::WindowEvent::Resized(..),
                    window_id,
                    ..
                } if window_id == console_window_id => {
                    console_window.resize();
                }
                // Le pointeur système n'a aucun rôle dans l'émulation elle-
                // même (le clavier et la manette suffisent) : il ne fait que
                // masquer l'image quand il traîne dessus. Mais F6/F10 posent
                // des boutons et des curseurs qu'il faut bien pouvoir viser :
                // ce simple booléen ne décide donc pas seul, il est combiné
                // chaque trame avec la visibilité des overlays (voir plus
                // bas, juste avant le rendu) pour obtenir la visibilité
                // réelle du pointeur.
                Event::Window {
                    win_event: sdl2::event::WindowEvent::Enter,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    mouse_over_main_window = true;
                }
                Event::Window {
                    win_event: sdl2::event::WindowEvent::Leave,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    mouse_over_main_window = false;
                }
                // Perte de focus (alt-tab, clic sur une autre fenêtre...) :
                // filet de sécurité pour ne jamais laisser la souris captée
                // — sans curseur visible ni fenêtre au premier plan — alors
                // que l'utilisateur est parti faire autre chose.
                Event::Window {
                    win_event: sdl2::event::WindowEvent::FocusLost,
                    window_id,
                    ..
                } if window_id == main_window_id && mouse_captured => {
                    sdl_context.mouse().set_relative_mouse_mode(false);
                    mouse_captured = false;
                }
                // Clic gauche pour capturer la souris (mode relatif SDL2) :
                // seulement hors overlay (sinon ce clic vise un bouton F6/F10
                // et ne doit pas en plus capturer), sur la fenêtre
                // principale, et seulement si la souris logicielle est
                // activée (config.toml [mouse] ou panneau F6) — capturer
                // pour un périphérique qu'aucun logiciel ne lira n'aurait
                // aucun sens et ne ferait que piéger le curseur sans raison.
                // Ce premier clic ne doit PAS remonter comme un bouton
                // enfoncé côté CPC : voir plus bas, le `MouseButtonDown`
                // générique est dans un bras `if` distinct qui l'exclut.
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Left,
                    window_id,
                    ..
                } if window_id == main_window_id
                    && !mouse_captured
                    && !(quick_bar_visible || config_panel_visible || keyboard_panel_visible)
                    && machine.bus.mouse.borrow().enabled =>
                {
                    sdl_context.mouse().set_relative_mouse_mode(true);
                    mouse_captured = true;
                }
                // Clic milieu pour relâcher — ne remonte pas non plus côté
                // CPC, même logique que la capture ci-dessus.
                Event::MouseButtonDown {
                    mouse_btn: MouseButton::Middle,
                    window_id,
                    ..
                } if window_id == main_window_id && mouse_captured => {
                    sdl_context.mouse().set_relative_mouse_mode(false);
                    mouse_captured = false;
                }
                // Mouvement relatif : en mode capturé, SDL2 rend `xrel`/`yrel`
                // au lieu d'une position absolue recentrée chaque trame — pas
                // besoin de calculer nous-mêmes un delta depuis la position
                // précédente.
                Event::MouseMotion {
                    window_id,
                    xrel,
                    yrel,
                    ..
                } if window_id == main_window_id && mouse_captured => {
                    machine.bus.mouse.borrow_mut().on_motion(xrel, yrel);
                }
                // Boutons transmis tels quels pendant la capture (hors clic
                // gauche/milieu de capture/relâche eux-mêmes, déjà consommés
                // par les bras ci-dessus qui les interceptent en premier —
                // l'ordre des bras `match` compte ici).
                Event::MouseButtonDown {
                    mouse_btn,
                    window_id,
                    ..
                } if window_id == main_window_id && mouse_captured => {
                    if let Some(button) = mouse_button_index(mouse_btn) {
                        machine.bus.mouse.borrow_mut().set_button(button, true);
                    }
                }
                Event::MouseButtonUp {
                    mouse_btn,
                    window_id,
                    ..
                } if window_id == main_window_id && mouse_captured => {
                    if let Some(button) = mouse_button_index(mouse_btn) {
                        machine.bus.mouse.borrow_mut().set_button(button, false);
                    }
                }
                // Glisser-déposer d'un .SNA/.DSK/.CDT sur la fenêtre
                // principale : même chargement que les commandes console
                // "snapload"/"disk"/"tape", choisi par extension
                // (insensible à la casse). Le lecteur A est le seul choix
                // possible pour un .dsk déposé — pas d'interface pour
                // viser le lecteur B ici, comme le glisser-déposer web
                // (bytebox-web/src/dropfile.rs, même principe).
                Event::DropFile {
                    window_id,
                    filename,
                    ..
                } if window_id == main_window_id => {
                    let lower = filename.to_ascii_lowercase();
                    let result = if lower.ends_with(".sna") {
                        machine
                            .load_snapshot(&filename)
                            .map(|()| format!("Snapshot loaded: {filename}"))
                    } else if lower.ends_with(".dsk") {
                        machine
                            .load_disk(&filename)
                            .map(|()| format!("Disk inserted: {filename}"))
                    } else if lower.ends_with(".cdt") {
                        machine
                            .load_tape(&filename)
                            .map(|()| format!("Tape inserted: {filename}"))
                    } else {
                        Err(format!("Unrecognized file type: {filename}"))
                    };
                    match result {
                        Ok(msg) => {
                            app_log!("{msg}");
                            osd.show(msg);
                        }
                        Err(e) => {
                            app_log!("Drag-and-drop load failed: {e}");
                            osd.show(e);
                        }
                    }
                }
                // Taille d'affichage : F1 normale, F2 x2, F3 x3, F4 plein
                // écran. Repasser par F1/F2/F3 quitte aussi le plein écran,
                // pour ne jamais y rester coincé sans savoir comment en
                // sortir.
                //
                // `repeat: false` sur celle-ci et sur toutes les touches de
                // fonction ci-dessous (F1-F12) : ce sont des bascules, pas du
                // texte à taper — sans ce filtre, un appui tenu un peu trop
                // longtemps envoie plusieurs `KeyDown` de répétition du
                // système (le délai avant la première répétition varie
                // d'un système à l'autre, plus court sur certaines
                // configurations Linux que sur macOS) et la touche
                // bascule deux fois de suite, quasi instantanément — visible
                // sur F5 comme un message OSD qui semble clignoter entre
                // l'ancien et le nouvel état. Sans rapport avec la frappe
                // normale (dernier `Event::KeyDown` de cette liste, le
                // passthrough clavier CPC), qui doit au contraire répéter
                // tant que la touche reste enfoncée.
                //
                // `window_id == main_window_id` sur celle-ci et sur les onze
                // autres touches de fonction ci-dessous : sans ce filtre,
                // n'importe laquelle se déclenchait aussi depuis la console
                // (F11) ou la fenêtre de statut (F12), qui ont leur propre
                // focus — repérée en creusant un signalement de
                // clignotement OSD (finalement pas lié à ceci, mais
                // découverte au passage). Même principe que le passthrough
                // clavier CPC un peu plus bas, ou les gestionnaires de
                // redimensionnement par fenêtre juste au-dessus.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F1),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    current_zoom = DisplayMode::Normal;
                    apply_display_mode(renderer.window_mut(), current_zoom);
                }
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F2),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    current_zoom = DisplayMode::X2;
                    apply_display_mode(renderer.window_mut(), current_zoom);
                }
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F3),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    current_zoom = DisplayMode::X3;
                    apply_display_mode(renderer.window_mut(), current_zoom);
                }
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F4),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    // Bascule : F4 quitte le plein écran s'il est deja actif
                    // (par F4 ou par default_zoom = "fullscreen").
                    let currently_fullscreen = renderer.window().fullscreen_state()
                        != sdl2::video::FullscreenType::Off;
                    current_zoom = if currently_fullscreen {
                        DisplayMode::Normal
                    } else {
                        DisplayMode::Fullscreen
                    };
                    apply_display_mode(renderer.window_mut(), current_zoom);
                }
                // Shader CRT (F5, Plan V2.md jalon M4) : scanlines +
                // aperture arrondie des pixels, voir renderer_crt.wgsl.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F5),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    renderer.toggle_crt();
                    let state = if renderer.crt_enabled() {
                        "enabled"
                    } else {
                        "disabled"
                    };
                    app_log!("CRT shader {state}");
                    osd.show(format!("CRT shader {state}"));
                }
                // Événements d'enfoncement de touches du clavier moderne PC
                //
                // Pas-à-pas CPU (F8) et pas-à-pas ligne (F9) : anciennement
                // F10/Shift+F10, déplacées pour lui laisser la barre de
                // commande rapide (voir plus bas).
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F8),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    app_log!(
                        "{}",
                        (zilog_z80::dasm::dasm(&machine.bus, machine.cpu.reg.pc)).0
                    );
                    machine.step();
                    machine.print_registers();
                }
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F9),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    let start_line = machine.current_line;
                    while machine.current_line == start_line {
                        let ticks = machine.step();
                        if ticks == 0 {
                            break;
                        }
                    }
                    app_log!(
                        "Stepped to next video line (Line {}).",
                        machine.current_line
                    );
                    machine.print_registers();
                }
                // Barre de commande rapide (F10, Plan V2.md jalon M2) :
                // superposée à la fenêtre principale plutôt que dans une
                // fenêtre séparée (contrairement à F11/F12), donc pas de
                // show()/hide() ici — son affichage tient entièrement à
                // `quick_bar_visible`, lu par la fermeture passée à
                // `renderer.present` plus bas. Reprend le focus à chaque
                // ouverture pour taper sans clic préalable.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F10),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    quick_bar_visible = !quick_bar_visible;
                    if quick_bar_visible {
                        quick_bar.request_focus();
                    }
                }
                // Panneau de configuration/médias (F6, Plan V2.md jalon M3) :
                // même mécanisme que la barre rapide F10 ci-dessus (overlay
                // sur la fenêtre principale, pas de fenêtre séparée).
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F6),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    config_panel_visible = !config_panel_visible;
                    // Couvre le cas "redimensionné pendant que F6 était
                    // fermé" : le gestionnaire de redimensionnement plus bas
                    // ne peut la faire changer que pendant qu'il est déjà
                    // visible — voir son commentaire, et celui du F7
                    // équivalent juste en dessous.
                    if config_panel_visible {
                        config_panel_generation += 1;
                    }
                }
                // Clavier virtuel (F7, Plan V2.md jalon M5) : même mécanisme
                // que F6 ci-dessus. La réouverture (pas la fermeture) fait
                // changer `keyboard_panel_generation` — voir le commentaire
                // de `KeyboardPanel::ui`.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F7),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id => {
                    keyboard_panel_visible = !keyboard_panel_visible;
                    if keyboard_panel_visible {
                        keyboard_panel_generation += 1;
                    }
                }
                // Console complète (F11) : fenêtre séparée, sur le même
                // modèle que le statut machine (F12). Accepté aussi depuis
                // `console_window_id` elle-même, pas seulement depuis la
                // fenêtre principale : l'ouvrir lui donne le focus
                // (`request_focus` ci-dessous), donc un réappui sur F11 pour
                // la refermer arrive avec ce `window_id`-là — le filtre
                // `main_window_id` seul (ajouté pour empêcher les AUTRES
                // touches de fonction de se déclencher depuis cette fenêtre,
                // voir le commentaire sur F1) l'aurait sinon silencieusement
                // avalé.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F11),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id || window_id == console_window_id => {
                    console_window_visible = !console_window_visible;
                    if console_window_visible {
                        console_window.window_mut().show();
                        console_window.request_focus();
                    } else {
                        console_window.window_mut().hide();
                    }
                }
                // Raccourci alternatif pour la console, en plus de F11 :
                // sous macOS, F11 est intercepté au niveau système par
                // "Afficher le bureau" (Mission Control) avant même
                // d'atteindre l'application — rien à faire côté SDL contre
                // ça. F11 reste la touche standard partout ailleurs ; Cmd+
                // Maj+C donne un second chemin qui marche toujours sur Mac.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::C),
                    repeat: false,
                    keymod,
                    window_id,
                    ..
                } if (window_id == main_window_id || window_id == console_window_id)
                    && keymod.intersects(
                        sdl2::keyboard::Mod::LGUIMOD | sdl2::keyboard::Mod::RGUIMOD,
                    )
                    && keymod.intersects(
                        sdl2::keyboard::Mod::LSHIFTMOD | sdl2::keyboard::Mod::RSHIFTMOD,
                    ) =>
                {
                    console_window_visible = !console_window_visible;
                    if console_window_visible {
                        console_window.window_mut().show();
                        console_window.request_focus();
                    } else {
                        console_window.window_mut().hide();
                    }
                }
                // Même raisonnement que F11 ci-dessus : la plupart des
                // gestionnaires de fenêtres donnent le focus à une fenêtre
                // qu'on vient de montrer, même sans `request_focus` explicite
                // ici — un réappui sur F12 pour refermer arrive donc avec
                // `debug_window_id`, pas `main_window_id`.
                Event::KeyDown {
                    keycode: Some(sdl2::keyboard::Keycode::F12),
                    repeat: false,
                    window_id,
                    ..
                } if window_id == main_window_id || window_id == debug_window_id => {
                    debug_visible = !debug_visible;
                    if debug_visible {
                        status_panel.window_mut().show();
                    } else {
                        status_panel.window_mut().hide();
                    }
                }
                // La barre rapide absorbe le clavier tant qu'elle est
                // ouverte(s), et cette entrée globale ne doit de toute façon
                // s'appliquer qu'à la fenêtre principale : sans le filtre
                // sur `window_id`, taper dans la console complète (F11,
                // fenêtre séparée) ou dans la fenêtre de statut (F12)
                // enverrait aussi chaque touche à la matrice clavier émulée.
                Event::KeyDown {
                    keycode: Some(kc),
                    scancode: Some(sc),
                    keymod,
                    window_id,
                    ..
                } if window_id == main_window_id
                    && !quick_bar_visible
                    && !config_panel_visible =>
                {
                    let shift_held = keymod.intersects(
                        sdl2::keyboard::Mod::LSHIFTMOD | sdl2::keyboard::Mod::RSHIFTMOD,
                    );
                    if let Some(sc) = to_core_scancode(sc) {
                        machine.bus.psg.set_key_state_scancode(sc, true, shift_held);
                    } else if let Some(kc) = to_core_keycode(kc) {
                        machine.bus.psg.set_key_state(kc, true);
                    }
                }
                Event::KeyUp {
                    keycode: Some(kc),
                    scancode: Some(sc),
                    keymod,
                    ..
                } => {
                    let shift_held = keymod.intersects(
                        sdl2::keyboard::Mod::LSHIFTMOD | sdl2::keyboard::Mod::RSHIFTMOD,
                    );
                    if let Some(sc) = to_core_scancode(sc) {
                        machine.bus.psg.set_key_state_scancode(sc, false, shift_held);
                    } else if let Some(kc) = to_core_keycode(kc) {
                        machine.bus.psg.set_key_state(kc, false);
                    }
                }
                Event::ControllerButtonDown { button, .. } => {
                    let btn_idx = match button {
                        sdl2::controller::Button::DPadUp => 0,
                        sdl2::controller::Button::DPadDown => 1,
                        sdl2::controller::Button::DPadLeft => 2,
                        sdl2::controller::Button::DPadRight => 3,
                        sdl2::controller::Button::A => 4, // Fire 1
                        sdl2::controller::Button::B => 5, // Fire 2
                        sdl2::controller::Button::X | sdl2::controller::Button::Y => 6, // Fire 3
                        _ => 99,
                    };
                    if btn_idx != 99 {
                        machine.bus.psg.set_controller_button(btn_idx, true);
                    }
                }
                Event::ControllerButtonUp { button, .. } => {
                    let btn_idx = match button {
                        sdl2::controller::Button::DPadUp => 0,
                        sdl2::controller::Button::DPadDown => 1,
                        sdl2::controller::Button::DPadLeft => 2,
                        sdl2::controller::Button::DPadRight => 3,
                        sdl2::controller::Button::A => 4, // Fire 1
                        sdl2::controller::Button::B => 5, // Fire 2
                        sdl2::controller::Button::X | sdl2::controller::Button::Y => 6, // Fire 3
                        _ => 99,
                    };
                    if btn_idx != 99 {
                        machine.bus.psg.set_controller_button(btn_idx, false);
                    }
                }
                Event::ControllerAxisMotion { axis, value, .. } => {
                    let threshold = 10000;
                    match axis {
                        sdl2::controller::Axis::LeftX => {
                            if value > threshold {
                                machine.bus.psg.set_controller_button(3, true); // Right
                                machine.bus.psg.set_controller_button(2, false);
                            } else if value < -threshold {
                                machine.bus.psg.set_controller_button(2, true); // Left
                                machine.bus.psg.set_controller_button(3, false);
                            } else {
                                machine.bus.psg.set_controller_button(2, false);
                                machine.bus.psg.set_controller_button(3, false);
                            }
                        }
                        sdl2::controller::Axis::LeftY => {
                            if value > threshold {
                                machine.bus.psg.set_controller_button(1, true); // Down
                                machine.bus.psg.set_controller_button(0, false);
                            } else if value < -threshold {
                                machine.bus.psg.set_controller_button(0, true); // Up
                                machine.bus.psg.set_controller_button(1, false);
                            } else {
                                machine.bus.psg.set_controller_button(0, false);
                                machine.bus.psg.set_controller_button(1, false);
                            }
                        }
                        _ => {}
                    }
                }
                // Branchement/débranchement à chaud : ces événements sont
                // générés nativement par SDL (relayés depuis udev/le pilote
                // système), pas par une scrutation active de notre part —
                // aucun coût CPU supplémentaire au repos, juste deux cas de
                // plus dans la boucle d'événements qui tourne déjà.
                Event::ControllerDeviceAdded { which, .. } => {
                    // Ne remplace pas une manette déjà active : la première
                    // branchée garde la main, comme au démarrage.
                    if active_controller.is_none() {
                        match controller_subsystem.open(which) {
                            Ok(c) => {
                                app_log!("Controller opened: {}", c.name());
                                osd.show(format!("Controller connected: {}", c.name()));
                                active_controller = Some(c);
                            }
                            Err(e) => app_log!("Failed to open controller {which}: {e}"),
                        }
                    }
                }
                Event::ControllerDeviceRemoved { which, .. } => {
                    let was_active = active_controller
                        .as_ref()
                        .is_some_and(|c| c.instance_id() == which);
                    if was_active {
                        app_log!("Controller disconnected");
                        osd.show("Controller disconnected");
                        active_controller = None;
                        // Une direction ou un tir resté "enfoncé" au moment
                        // du débranchement resterait sinon bloqué indéfiniment
                        // dans la matrice clavier émulée.
                        for button in 0..7 {
                            machine.bus.psg.set_controller_button(button, false);
                        }
                    }
                }
                // Repli manette non reconnue par l'API Controller (voir la
                // doc d'`open_first_joystick` plus haut) : mêmes gestes,
                // schéma générique faute de mapping. `which` désigne l'ID
                // d'instance pour ces trois événements (contrairement à
                // `JoyDeviceAdded` ci-dessous), donc comparable directement à
                // `instance_id()`.
                Event::JoyAxisMotion {
                    which,
                    axis_idx,
                    value,
                    ..
                } => {
                    if active_joystick
                        .as_ref()
                        .is_some_and(|j| j.instance_id() == which)
                    {
                        let threshold = 10000;
                        match axis_idx {
                            0 => {
                                if value > threshold {
                                    machine.bus.psg.set_controller_button(3, true); // Right
                                    machine.bus.psg.set_controller_button(2, false);
                                } else if value < -threshold {
                                    machine.bus.psg.set_controller_button(2, true); // Left
                                    machine.bus.psg.set_controller_button(3, false);
                                } else {
                                    machine.bus.psg.set_controller_button(2, false);
                                    machine.bus.psg.set_controller_button(3, false);
                                }
                            }
                            1 => {
                                if value > threshold {
                                    machine.bus.psg.set_controller_button(1, true); // Down
                                    machine.bus.psg.set_controller_button(0, false);
                                } else if value < -threshold {
                                    machine.bus.psg.set_controller_button(0, true); // Up
                                    machine.bus.psg.set_controller_button(1, false);
                                } else {
                                    machine.bus.psg.set_controller_button(0, false);
                                    machine.bus.psg.set_controller_button(1, false);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                Event::JoyButtonDown {
                    which, button_idx, ..
                } => {
                    if active_joystick
                        .as_ref()
                        .is_some_and(|j| j.instance_id() == which)
                    {
                        // Pas de nom de bouton sans mapping : les trois
                        // premiers de la manette servent de tir 1/2/3, comme
                        // A/B/X côté API Controller.
                        if let 0..=2 = button_idx {
                            machine
                                .bus
                                .psg
                                .set_controller_button(4 + button_idx as usize, true);
                        }
                    }
                }
                Event::JoyButtonUp {
                    which, button_idx, ..
                } => {
                    if active_joystick
                        .as_ref()
                        .is_some_and(|j| j.instance_id() == which)
                        && let 0..=2 = button_idx
                    {
                        machine
                            .bus
                            .psg
                            .set_controller_button(4 + button_idx as usize, false);
                    }
                }
                Event::JoyDeviceAdded { which, .. } => {
                    // Ici `which` est un INDEX de périphérique (comme pour
                    // `ControllerDeviceAdded`), pas un ID d'instance : cet
                    // événement est émis pour tout joystick qui apparaît, y
                    // compris ceux reconnus par l'API Controller — dont
                    // l'ouverture est déjà gérée par `ControllerDeviceAdded`.
                    if active_controller.is_none()
                        && active_joystick.is_none()
                        && !controller_subsystem.is_game_controller(which)
                    {
                        match joystick_subsystem.open(which) {
                            Ok(j) => {
                                app_log!("Joystick opened (generic mapping): {}", j.name());
                                osd.show(format!("Controller connected: {}", j.name()));
                                active_joystick = Some(j);
                            }
                            Err(e) => app_log!("Failed to open joystick {which}: {e}"),
                        }
                    }
                }
                Event::JoyDeviceRemoved { which, .. } => {
                    let was_active = active_joystick
                        .as_ref()
                        .is_some_and(|j| j.instance_id() == which);
                    if was_active {
                        app_log!("Controller disconnected");
                        osd.show("Controller disconnected");
                        active_joystick = None;
                        for button in 0..7 {
                            machine.bus.psg.set_controller_button(button, false);
                        }
                    }
                }
                _ => {}
            }
        }

        // Une trame s'achève sur le VSYNC généré par le CRTC, pas sur un nombre
        // de ticks figé : un logiciel qui reprogramme R4/R9 change la durée de
        // trame, et le rendu doit suivre. La borne en ticks reste comme garde-fou
        // si le CRTC est programmé sans jamais produire de VSYNC.
        machine.frame_ready = false;
        let mut frame_ticks: u32 = 0;

        while machine.is_running() && !machine.frame_ready && frame_ticks < max_ticks_per_frame {
            let ticks = machine.step();
            if let Some(typer) = autotyper.as_mut() {
                typer.advance(&mut machine.bus.psg, ticks);
                if typer.is_done() {
                    autotyper = None;
                }
            }
            frame_ticks += ticks;
        }

        // Les échantillons produits pendant la trame partent vers la carte
        // son. En pause, le PSG n'avance plus : rien n'est produit et SDL
        // enchaîne sur du silence.
        let samples = machine.bus.psg.sound.take_samples();
        if let Some(audio) = audio.as_mut() {
            audio.set_volume(machine.volume());
            audio.push(&samples);
        }

        // Traité avant le rendu, pour que la sortie d'une commande saisie
        // dans l'une des deux façades console apparaisse dès cette trame,
        // plutôt qu'avec un tour de boucle de retard. Il n'y a plus de
        // terminal à alimenter : la sortie va uniquement dans le journal
        // partagé, lu aussi bien par la barre rapide (F10, une ligne) que
        // par la console complète (F11, tout l'historique).
        if let Ok(output) = machine.console_handle()
            && !output.is_empty()
        {
            console_log.push_output(&output);
        }
        // Idem pour tout ce que le reste de l'application a journalisé cette
        // trame (voir applog.rs) : connexion de manette, disquette éjectée,
        // avertissement de régulation audio... Rien de tout cela ne doit
        // atteindre le terminal qui a lancé l'émulateur.
        for line in bytebox_core::applog::drain() {
            console_log.push_output(&line);
        }

        // Appel au module vidéo déporté pour le rendu VRAM
        video::render(&machine, &mut frame_buffer);

        // Le panneau F6 est un état de la machine émulée (MonitorCmd) sauf
        // pour le zoom, un état de présentation (la fenêtre SDL2, voir
        // `ZoomChoice`) : `ConfigPanel::ui` le renvoie plutôt que de
        // l'appliquer lui-même, pour rester composée dans la même fermeture
        // que la barre rapide sans avoir à connaître `Renderer`.
        // Le pointeur redevient visible dès qu'un overlay cliquable (F6, F7,
        // et par cohérence F10) est ouvert et que la souris est sur la
        // fenêtre principale : sinon les boutons du panneau de
        // configuration seraient quasiment impossibles à viser à l'aveugle.
        // Recalculé chaque trame plutôt qu'à chaque évènement individuel
        // (Enter/Leave, F6, F10...) : plus simple, et 60 fois par seconde
        // est largement assez réactif pour un simple show/hide de curseur.
        // Filet de sécurité, recalculé chaque trame comme le show/hide du
        // curseur juste en dessous : la capture doit se relâcher dès qu'un
        // overlay s'ouvre (F6/F7/F10, quelle que soit la touche qui l'a fait
        // — sinon un clic viserait un bouton du panneau tout en continuant à
        // piloter la souris émulée en douce) ou que la souris logicielle est
        // désactivée en cours de capture (ex. commande console "mouse off"
        // tapée dans F11 sans passer par F6).
        if mouse_captured
            && (quick_bar_visible
                || config_panel_visible
                || keyboard_panel_visible
                || !machine.bus.mouse.borrow().enabled)
        {
            sdl_context.mouse().set_relative_mouse_mode(false);
            mouse_captured = false;
        }

        sdl_context.mouse().show_cursor(
            !mouse_over_main_window
                || quick_bar_visible
                || config_panel_visible
                || keyboard_panel_visible,
        );

        let mut requested_zoom: Option<ZoomChoice> = None;
        let mut requested_crt_settings: Option<CrtSettings> = None;
        let mut requested_keyboard_settings: Option<KeyboardSettings> = None;
        let mut requested_keys: Option<std::collections::HashSet<(usize, u8)>> = None;
        // Lu avant de créer la fermeture ci-dessous, pour la même raison que
        // `show_overlay` juste en dessous : une fois la fermeture construite,
        // elle emprunte `config_panel_visible` en mutable, donc plus moyen de
        // relire quoi que ce soit de `renderer` (qui la borrow aussi via
        // `present`) entre les deux.
        let crt_settings = renderer.crt_settings();
        // Même raison : taille réelle de la fenêtre CPC, lue directement sur
        // SDL plutôt que sur l'état egui de cette trame — voir le
        // commentaire de `KeyboardPanel::ui` sur pourquoi ce dernier peut
        // encore accuser un train de retard juste après un changement de
        // zoom (F1-F4).
        let (window_w, window_h) = renderer.window().drawable_size();
        let window_size = egui::vec2(window_w as f32, window_h as f32);
        // Décidé avant de créer la fermeture ci-dessous : elle emprunte
        // `config_panel_visible` en mutable (`ConfigPanel::ui` peut la
        // remettre à faux via la croix de la fenêtre egui), la relire une
        // fois la fermeture construite serait rejeté par l'emprunteur.
        // L'OSD doit s'afficher même quand aucun des trois panneaux
        // ci-dessus n'est ouvert (il apparaît en pleine partie, pas
        // seulement pendant qu'on consulte F6/F7/F10) : sa propre visibilité
        // rejoint donc `show_overlay`, indépendamment des autres. Même
        // raisonnement pour le point rouge d'accès disque : il doit
        // apparaître en pleine partie, pas seulement quand un panneau est
        // déjà ouvert pour une autre raison.
        let disk_access = disk_indicator_enabled && machine.disk_access_in_progress();
        let show_overlay = quick_bar_visible
            || config_panel_visible
            || keyboard_panel_visible
            || osd.is_active()
            || disk_access;
        let mut draw_overlay = |ctx: &egui::Context| {
            if quick_bar_visible {
                quick_bar.ui(ctx, &cmd_sender, &mut console_log, window_size);
            }
            if config_panel_visible {
                let (zoom, crt, keyboard) = config_panel.ui(
                    ctx,
                    &machine,
                    &cmd_sender,
                    &mut config_panel_visible,
                    crt_settings,
                    keyboard_settings,
                    window_size,
                    config_panel_generation,
                    current_zoom.to_zoom_choice(),
                    &mut disk_indicator_enabled,
                );
                requested_zoom = zoom;
                requested_crt_settings = Some(crt);
                requested_keyboard_settings = Some(keyboard);
            }
            if keyboard_panel_visible {
                requested_keys = Some(keyboard_panel.ui(
                    ctx,
                    &mut keyboard_panel_visible,
                    keyboard_panel_generation,
                    keyboard_settings,
                    window_size,
                ));
            }
            osd.ui(ctx, window_size);
            if disk_access {
                let scale = crate::ui_scale::content_scale(window_size);
                egui::Area::new(egui::Id::new("disk_access_indicator"))
                    .anchor(egui::Align2::RIGHT_TOP, egui::vec2(-12.0, 12.0) * scale)
                    .interactable(false)
                    .show(ctx, |ui| {
                        // Rectangle plutôt qu'un rond : rappelle la forme de
                        // la LED d'un vrai lecteur de disquettes, pas un
                        // simple témoin générique.
                        let size = egui::vec2(16.0, 10.0) * scale;
                        let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
                        ui.painter().rect_filled(
                            rect,
                            2.0 * scale,
                            egui::Color32::from_rgb(220, 40, 40),
                        );
                    });
            }
        };
        let overlay: Option<&mut dyn FnMut(&egui::Context)> = if show_overlay {
            Some(&mut draw_overlay)
        } else {
            None
        };
        renderer.present(&frame_buffer, overlay);

        if let Some(zoom) = requested_zoom {
            current_zoom = match zoom {
                ZoomChoice::X1 => DisplayMode::Normal,
                ZoomChoice::X2 => DisplayMode::X2,
                ZoomChoice::X3 => DisplayMode::X3,
                ZoomChoice::Fullscreen => DisplayMode::Fullscreen,
            };
            apply_display_mode(renderer.window_mut(), current_zoom);
        }
        // Toujours réappliqué sans détection de changement : `crt_settings`
        // ci-dessus vaut déjà les réglages courants si l'utilisateur n'a
        // touché aucun curseur cette trame, donc réécrire les 32 octets du
        // tampon uniforme est un no-op fonctionnel, pas juste bon marché.
        if let Some(settings) = requested_crt_settings {
            renderer.set_crt_settings(settings);
        }
        // Même chose pour les réglages du clavier virtuel : simple
        // affectation, `keyboard_settings` vaut déjà la valeur courante si
        // rien n'a changé cette trame.
        if let Some(settings) = requested_keyboard_settings {
            keyboard_settings = settings;
        }
        // `requested_keys` vaut `None` aussi bien quand le panneau est fermé
        // que la trame où il vient tout juste de se refermer (croix de la
        // fenêtre, ou F7) : `unwrap_or_default` retombe alors sur l'ensemble
        // vide, ce qui relâche automatiquement toute touche encore tenue à
        // cet instant — sinon une touche cliquée puis le panneau refermé
        // sans relâcher le bouton resterait bloquée enfoncée côté CPC.
        let new_pressed_keys = requested_keys.unwrap_or_default();
        // `apply_matrix_diff` (pas deux boucles `set_matrix_bit`) : un
        // relâchement de loquet (SHIFT/CONTROL) et un appui peuvent tomber
        // dans la même trame ici (contrairement au clavier physique, un
        // évènement par appel) — voir son commentaire pour pourquoi ça
        // reproduit sinon le bug déjà connu sur "#/@" et consorts.
        machine.bus.psg.apply_matrix_diff(
            pressed_keys.difference(&new_pressed_keys).copied(),
            new_pressed_keys.difference(&pressed_keys).copied(),
        );
        pressed_keys = new_pressed_keys;

        if debug_visible {
            let registers = machine.get_registers_string();
            let hardware = machine.get_hardware_string(machine.show_keyboard_matrix());
            status_panel.render(&registers, &hardware);
        }

        if console_window_visible {
            console_window.render(&mut console_log, &cmd_sender);
        }

        // Mesure de la vitesse réelle, moyennée sur une seconde : c'est elle
        // qui dit si la machine tient la cadence, et donc si le son a de quoi
        // alimenter la carte son en continu.
        measured_ticks += frame_ticks as u64;
        if measure_start.elapsed() >= std::time::Duration::from_secs(1) {
            let emulated = machine::emulated_duration(measured_ticks as u32).as_secs_f32();
            machine.set_measured_timing(
                100.0 * emulated / measure_start.elapsed().as_secs_f32(),
                late_frames,
            );
            // La régulation audio ne doit rien avoir à faire en régime normal.
            // Le moindre remplissage insère du silence dans le flux et étire la
            // musique : on le signale, il ne se voit nulle part ailleurs.
            // Débrayable, car sur une machine trop juste le message se
            // répéterait à chaque seconde (config.toml, [debugger] audio).
            if let Some(audio) = audio.as_mut().filter(|_| machine.report_audio_regulation()) {
                let (refills, padded_ms, dropped) = audio.take_stats();
                if refills > 0 || dropped > 0 {
                    app_log!(
                        "Audio: {refills} refill(s) ({padded_ms:.0} ms of silence inserted), \
                         {dropped} frame(s) dropped, queue {} samples",
                        audio.queued_samples()
                    );
                }
            }
            measured_ticks = 0;
            late_frames = 0;
            measure_start = std::time::Instant::now();
        }

        let frame_duration = if frame_ticks > 0 {
            machine::emulated_duration(frame_ticks)
        } else {
            PAUSED_FRAME
        };
        let now = std::time::Instant::now();
        if now < next_frame {
            std::thread::sleep(next_frame - now);
            next_frame += frame_duration;
        } else {
            // Trame en retard : on repart de l'instant courant plutôt que de
            // tenter de rattraper, ce qui emballerait la machine émulée. Le
            // temps perdu ne se rattrape donc jamais : la carte son se
            // retrouve à sec et le silence qu'elle joue à la place étire la
            // musique d'autant. Un décrochage bref suffit à l'entendre.
            next_frame = now + frame_duration;
            late_frames += 1;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_mode_recognizes_the_four_config_values() {
        assert_eq!(DisplayMode::from_config(Some("x1")), DisplayMode::Normal);
        assert_eq!(DisplayMode::from_config(Some("x2")), DisplayMode::X2);
        assert_eq!(DisplayMode::from_config(Some("x3")), DisplayMode::X3);
        assert_eq!(
            DisplayMode::from_config(Some("fullscreen")),
            DisplayMode::Fullscreen
        );
    }

    /// Absent ou mal orthographié, on ne bloque pas le démarrage : la
    /// fenêtre s'ouvre en taille normale plutôt que de faire planter
    /// l'émulateur sur une faute de frappe dans config.toml.
    #[test]
    fn display_mode_falls_back_to_normal_when_absent_or_unrecognized() {
        assert_eq!(DisplayMode::from_config(None), DisplayMode::Normal);
        assert_eq!(DisplayMode::from_config(Some("XL")), DisplayMode::Normal);
    }
}
