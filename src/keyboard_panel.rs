//! Clavier virtuel (F7, Plan V2.md jalon M5) : l'illustration stylisée du
//! clavier 6128 AZERTY (`assets/keyboard.png`), avec une zone cliquable par
//! touche mappée **directement** sur une position `(ligne, bit)` de la
//! matrice PSG (`psg::Psg::set_matrix_bit`) — aucune couche de keymap
//! supplémentaire, exactement comme le clavier physique lui-même
//! (`sdl.rs`, `machine.bus.psg.set_key_state`/`set_key_state_scancode`).
//!
//! Les rectangles ci-dessous ont été mesurés une fois pour toutes sur
//! l'image source (1801x873), par détection de bord (profil de luminosité
//! par colonne/ligne, voir l'historique de ce fichier) plutôt qu'à l'œil :
//! chaque valeur correspond au bord exact d'une touche dans le PNG. Le
//! panneau les remet à l'échelle de la taille d'affichage courante à chaque
//! trame (`scale`, calculé depuis la largeur réellement disponible) — image
//! et zones cliquables grossissent/rétrécissent ensemble, aucune des deux
//! n'est jamais mesurée en pixels d'écran.
//!
//! Position de chaque touche vérifiée contre la table déjà éprouvée de
//! `Psg::set_key_state` (clavier physique) : les lettres/chiffres/touches de
//! contrôle en reprennent directement les positions. Les symboles ambigus
//! (`# > < * $ @ % ù`) ont été recoupés avec `doc/clavier-mac-azerty.md`, qui
//! documente déjà leurs positions CPC exactes suite à l'investigation sur
//! clavier réel.
//!
//! Une seule souris ne peut pas maintenir SHIFT/CONTROL enfoncés tout en
//! cliquant une autre touche : ces deux-là sont donc des loquets (un clic
//! les enfonce), relâchés automatiquement dès qu'une autre touche (non
//! modificatrice) est cliquée — inutile de recliquer dessus à chaque fois,
//! il suffit de les poser puis de taper la touche à combiner. Toutes les
//! autres touches (CAPS LOCK compris, voir plus bas) suivent le bouton de la
//! souris (enfoncées tant qu'il est tenu au-dessus d'elles), comme une
//! touche physique classique.
//!
//! CAPS LOCK est un cas à part : une PREMIÈRE tentative le traitait comme un
//! loquet électrique persistant (bit posé en continu tant qu'"actif"), sur
//! l'hypothèse qu'il s'agit d'une vraie touche à verrouillage mécanique sur
//! le clavier physique du CPC. Constaté à l'usage : ça désynchronise l'état
//! réel du firmware de l'affichage (le voyant du clavier virtuel bascule à
//! chaque clic, mais il fallait cliquer deux fois pour que le CPC bascule
//! réellement) — signe que le firmware réagit au FRONT du bit, pas à son
//! niveau continu, et qu'un maintien introduit un front de trop. Corrigé en
//! envoyant une impulsion brève (une seule trame) à chaque clic, exactement
//! comme n'importe quelle autre touche tapée rapidement, plutôt qu'un
//! maintien : `caps_lock_display` ci-dessous n'est donc plus qu'une
//! estimation d'affichage (bascule à chaque clic), sans lien avec l'état
//! électrique envoyé au PSG — cet émulateur n'a de toute façon aucun moyen
//! de relire l'état verrouillage-majuscules interne du firmware pour
//! vérifier.

use std::collections::HashSet;

/// Position `(ligne, bit)` d'une touche dans la matrice PSG.
type Position = (usize, u8);

const SHIFT: Position = (2, 5);
const CONTROL: Position = (2, 7);
const CAPS_LOCK: Position = (8, 6);
/// Seule touche à deux rectangles qui soit une seule et même touche
/// visuelle en L (SHIFT, l'autre cas à deux rectangles, ce sont deux
/// touches séparées aux deux bouts du clavier) — voir `l_shape_outline`
/// plus bas, pour un contour de surbrillance qui suit cette forme au lieu
/// de deux rectangles disjoints qui la font paraître coupée en deux.
const RETURN: Position = (2, 2);

/// Loquets relâchés automatiquement dès qu'une autre touche est cliquée —
/// voir le commentaire d'en-tête. CAPS LOCK n'en est PAS un : impulsion
/// d'une trame par clic, traité séparément dans `ui()`.
const ONE_SHOT_LATCHES: [Position; 2] = [SHIFT, CONTROL];

/// Réglages du panneau F7 sauvegardables dans `config.toml`, sur le même
/// modèle que `CrtSettings`/`CrtConfig` (`renderer.rs`) : un champ absent de
/// la section `[keyboard]` du fichier laisse la valeur par défaut ci-dessous,
/// un champ présent l'outrepasse.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct KeyboardSettings {
    /// Taille par défaut à l'ouverture, en fraction de la hauteur de la
    /// fenêtre CPC (0.0..=1.0) — voir le commentaire sur le plafond de
    /// hauteur dans `ui()`.
    pub default_size_percent: f32,
}

impl Default for KeyboardSettings {
    fn default() -> Self {
        Self {
            default_size_percent: 0.33,
        }
    }
}

impl KeyboardSettings {
    pub fn from_config(cfg: &bytebox_core::config::KeyboardConfig) -> Self {
        let d = Self::default();
        Self {
            default_size_percent: cfg.default_size_percent.unwrap_or(d.default_size_percent),
        }
    }

    pub fn to_config(self) -> bytebox_core::config::KeyboardConfig {
        bytebox_core::config::KeyboardConfig {
            default_size_percent: Some(self.default_size_percent),
        }
    }
}

/// Taille de l'image source, en pixels — référence de tous les rectangles
/// ci-dessous, indépendante de la taille d'affichage courante.
const IMAGE_SIZE: egui::Vec2 = egui::vec2(1801.0, 873.0);

struct VirtualKey {
    /// Rectangle en pixels de l'image SOURCE (voir `IMAGE_SIZE`), pas de
    /// l'écran : remis à l'échelle à chaque trame dans `ui()`.
    rect: egui::Rect,
    position: Position,
}

const fn k(x0: f32, y0: f32, x1: f32, y1: f32, line: usize, bit: u8) -> VirtualKey {
    VirtualKey {
        rect: egui::Rect {
            min: egui::pos2(x0, y0),
            max: egui::pos2(x1, y1),
        },
        position: (line, bit),
    }
}

/// Les 8 sommets du contour en L de deux rectangles empilés verticalement
/// et en contact (`top.max.y == bottom.min.y`) — RETURN, la seule touche
/// dans ce cas (voir la constante `RETURN`). Ne suppose pas lequel des
/// deux est le plus large : suit simplement l'union réelle des deux
/// largeurs, dans l'ordre où `top`/`bottom` sont donnés.
fn l_shape_outline(top: egui::Rect, bottom: egui::Rect) -> Vec<egui::Pos2> {
    use egui::pos2;
    vec![
        pos2(top.min.x, top.min.y),
        pos2(top.max.x, top.min.y),
        pos2(top.max.x, top.max.y),
        pos2(bottom.max.x, top.max.y),
        pos2(bottom.max.x, bottom.max.y),
        pos2(bottom.min.x, bottom.max.y),
        pos2(bottom.min.x, top.max.y),
        pos2(top.min.x, top.max.y),
    ]
}

/// Une touche à cheval sur deux rangées (RETURN, en L) ou dédoublée de
/// chaque côté du clavier (SHIFT) apparaît deux fois ici, avec la même
/// position : deux rectangles, une seule touche PSG.
#[rustfmt::skip]
const KEYS: &[VirtualKey] = &[
    // Rangée 1 (y 251-337) : ESC, chiffres, CLR/DEL, f7-f9.
    k(47.0, 251.0, 136.0, 337.0, 8, 2),    // ESC
    k(144.0, 251.0, 233.0, 337.0, 8, 0),   // 1 &
    k(240.0, 251.0, 329.0, 337.0, 8, 1),   // 2 é
    k(335.0, 251.0, 425.0, 337.0, 7, 1),   // 3 "
    k(432.0, 251.0, 520.0, 337.0, 7, 0),   // 4 '
    k(527.0, 251.0, 617.0, 337.0, 6, 1),   // 5 (
    k(624.0, 251.0, 712.0, 337.0, 6, 0),   // 6 ]
    k(718.0, 251.0, 807.0, 337.0, 5, 1),   // 7 è
    k(814.0, 251.0, 903.0, 337.0, 5, 0),   // 8 !
    k(910.0, 251.0, 998.0, 337.0, 4, 1),   // 9 ç
    k(1005.0, 251.0, 1092.0, 337.0, 4, 0), // 0 à
    k(1099.0, 251.0, 1187.0, 337.0, 3, 1), // [ )
    k(1194.0, 251.0, 1283.0, 337.0, 3, 0), // - _
    k(1290.0, 251.0, 1378.0, 337.0, 2, 0), // CLR
    k(1385.0, 251.0, 1474.0, 337.0, 9, 7), // DEL
    k(1481.0, 251.0, 1571.0, 337.0, 1, 2), // f7
    k(1578.0, 251.0, 1667.0, 337.0, 1, 3), // f8
    k(1674.0, 251.0, 1762.0, 337.0, 0, 3), // f9

    // Rangée 2 (y 348-435) : TAB, AZERTY, RETURN (partie haute), f4-f6.
    k(48.0, 348.0, 179.0, 435.0, 8, 4),    // TAB
    k(192.0, 348.0, 276.0, 435.0, 8, 3),   // A
    k(288.0, 348.0, 374.0, 435.0, 7, 3),   // Z
    k(386.0, 348.0, 471.0, 435.0, 7, 2),   // E
    k(483.0, 348.0, 567.0, 435.0, 6, 2),   // R
    k(580.0, 348.0, 663.0, 435.0, 6, 3),   // T
    k(675.0, 348.0, 759.0, 435.0, 5, 3),   // Y
    k(770.0, 348.0, 853.0, 435.0, 5, 2),   // U
    k(866.0, 348.0, 950.0, 435.0, 4, 3),   // I
    k(961.0, 348.0, 1044.0, 435.0, 4, 2),  // O
    k(1055.0, 348.0, 1140.0, 435.0, 3, 3), // P
    k(1151.0, 348.0, 1237.0, 435.0, 3, 2), // ¨ ^
    k(1246.0, 348.0, 1332.0, 435.0, 2, 1), // < *
    // RETURN (haut/bas) : les deux rectangles se rejoignent exactement à
    // y=439.5 (le milieu de l'écart de 9px entre rangées 2 et 3 ailleurs
    // sur le clavier) — dans l'image source, RETURN est dessinée comme une
    // seule touche en L, sans coupure visible à cet endroit ; un vrai écart
    // ici créerait une zone morte au clic en plein milieu de cette touche.
    k(1342.0, 348.0, 1474.0, 439.5, 2, 2), // RETURN (haut)
    k(1484.0, 348.0, 1571.0, 435.0, 2, 4), // f4
    k(1580.0, 348.0, 1667.0, 435.0, 1, 4), // f5
    k(1677.0, 348.0, 1761.0, 435.0, 0, 4), // f6

    // Rangée 3 (y 444-530) : CAPS LOCK, QSDFG..., RETURN (partie basse), f1-f3.
    k(46.0, 444.0, 205.0, 530.0, 8, 6),    // CAPS LOCK
    k(216.0, 444.0, 301.0, 530.0, 8, 5),   // Q
    k(313.0, 444.0, 399.0, 530.0, 7, 4),   // S
    k(409.0, 444.0, 493.0, 530.0, 7, 5),   // D
    k(505.0, 444.0, 589.0, 530.0, 6, 5),   // F
    k(601.0, 444.0, 685.0, 530.0, 6, 4),   // G
    k(698.0, 444.0, 781.0, 530.0, 5, 4),   // H
    k(793.0, 444.0, 875.0, 530.0, 5, 5),   // J
    k(889.0, 444.0, 970.0, 530.0, 4, 5),   // K
    k(983.0, 444.0, 1067.0, 530.0, 4, 4),  // L
    k(1079.0, 444.0, 1165.0, 530.0, 3, 5), // M
    k(1174.0, 444.0, 1262.0, 530.0, 3, 4), // % ù
    k(1272.0, 444.0, 1360.0, 530.0, 2, 3), // > #
    k(1370.0, 439.5, 1469.0, 530.0, 2, 2), // RETURN (bas)
    k(1485.0, 444.0, 1569.0, 530.0, 1, 5), // f1
    k(1580.0, 444.0, 1667.0, 530.0, 1, 6), // f2
    k(1676.0, 444.0, 1762.0, 530.0, 0, 5), // f3

    // Rangée 4 (y 540-626) : SHIFT, WXCVBN..., SHIFT, f0, flèche haut, ".".
    k(46.0, 540.0, 250.0, 626.0, 2, 5),    // SHIFT (gauche)
    k(262.0, 540.0, 348.0, 626.0, 8, 7),   // W
    k(359.0, 540.0, 445.0, 626.0, 7, 7),   // X
    k(456.0, 540.0, 541.0, 626.0, 7, 6),   // C
    k(552.0, 540.0, 637.0, 626.0, 6, 7),   // V
    k(648.0, 540.0, 732.0, 626.0, 6, 6),   // B
    k(743.0, 540.0, 827.0, 626.0, 5, 6),   // N
    k(840.0, 540.0, 922.0, 626.0, 4, 6),   // ? ,
    k(934.0, 540.0, 1017.0, 626.0, 4, 7),  // : ;
    k(1029.0, 540.0, 1112.0, 626.0, 3, 7), // / :
    k(1124.0, 540.0, 1210.0, 626.0, 3, 6), // + =
    k(1219.0, 540.0, 1304.0, 626.0, 2, 6), // @ $
    k(1316.0, 540.0, 1475.0, 626.0, 2, 5), // SHIFT (droite)
    k(1483.0, 540.0, 1571.0, 626.0, 1, 7), // f0
    k(1580.0, 540.0, 1667.0, 626.0, 0, 0), // ↑
    k(1676.0, 540.0, 1760.0, 626.0, 0, 7), // ·

    // Rangée 5 (y 636-721) : CONTROL, COPY, SPACE, ENTER, flèches.
    k(46.0, 636.0, 249.0, 721.0, 2, 7),    // CONTROL
    k(260.0, 636.0, 422.0, 721.0, 1, 1),   // COPY
    k(431.0, 636.0, 1183.0, 721.0, 5, 7),  // SPACE
    k(1196.0, 636.0, 1474.0, 721.0, 0, 6), // ENTER
    k(1484.0, 636.0, 1571.0, 721.0, 1, 0), // ←
    k(1581.0, 636.0, 1667.0, 721.0, 0, 2), // ↓
    k(1677.0, 636.0, 1761.0, 721.0, 0, 1), // →
];

pub struct KeyboardPanel {
    /// Chargée paresseusement au premier `ui()` : `egui::Context` (donc
    /// `load_texture`) n'existe pas encore à la construction du panneau
    /// (`sdl::run`, avant la boucle d'évènements). `None` de façon durable
    /// si le fichier est illisible — le panneau reste alors une fenêtre
    /// vide plutôt qu'un plantage, avec un message dans la console.
    texture: Option<egui::TextureHandle>,
    /// Touches actuellement verrouillées (SHIFT/CONTROL) — voir le
    /// commentaire d'en-tête.
    latched: HashSet<Position>,
    /// Bascule d'affichage uniquement pour CAPS LOCK, sans lien avec ce qui
    /// est envoyé au PSG (impulsion, pas un maintien) — voir le commentaire
    /// d'en-tête pour pourquoi.
    caps_lock_display: bool,
}

impl KeyboardPanel {
    pub fn new() -> Self {
        Self {
            texture: None,
            latched: HashSet::new(),
            caps_lock_display: false,
        }
    }

    /// Dessine le panneau ; `open` reflète et contrôle sa visibilité, comme
    /// `ConfigPanel::ui`. `generation` doit changer à chaque réouverture (F7) —
    /// il fait partie de l'id de la fenêtre egui, donc de la clé sous
    /// laquelle sa position/taille est mémorisée : sans ça, elles ne
    /// seraient calculées qu'une seule fois pour toute la session (egui
    /// retient position et taille par id, y compris pendant que la fenêtre
    /// est masquée) et resteraient celles du tout premier affichage même
    /// après un changement de zoom entre deux ouvertures. Suivi côté
    /// `sdl.rs`, pas ici : ce panneau n'est construit/appelé QUE pendant
    /// qu'il est visible, il ne peut donc pas détecter lui-même sa propre
    /// fermeture.
    ///
    /// Renvoie l'ensemble des positions PSG qui doivent être enfoncées CETTE
    /// trame (loquets + éventuelle touche tenue par la souris) — à
    /// l'appelant de comparer avec la trame précédente et de n'appliquer que
    /// ce qui a changé sur `machine.bus.psg` (voir `sdl.rs`) : ce panneau ne
    /// touche jamais la machine lui-même, il n'en a pas connaissance.
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        open: &mut bool,
        generation: u64,
        settings: KeyboardSettings,
        window_size: egui::Vec2,
    ) -> HashSet<Position> {
        if self.texture.is_none() {
            self.texture = Self::load_texture(ctx);
        }

        // Taille par défaut à l'ouverture : bornée à la fois en largeur (90%
        // de la fenêtre CPC, fixe) ET en hauteur (`default_size_percent` de
        // cette même fenêtre, réglable depuis F6 — Plan V2.md jalon M5 :
        // sans ce plafond, le clavier masquait l'écran qu'on est justement
        // en train de regarder en tapant, surtout sensible en x1). La plus
        // contraignante des deux gagne ; en plein écran haute résolution,
        // c'est en général la limite de hauteur qui l'emporte, donnant un
        // clavier nettement plus grand qu'en fenêtre x1 pour le même
        // pourcentage — sans valeur spéciale "x2 en 4K", juste la même règle
        // appliquée à une fenêtre plus grande.
        //
        // `window_size` vient de `renderer.window().drawable_size()`
        // (`sdl.rs`), pas de `ctx.content_rect()` : ce dernier reflète l'état
        // qu'`egui_sdl2_event` a construit pour CETTE trame à partir des
        // évènements SDL déjà traités, qui peut encore accuser un train de
        // retard juste après un changement de zoom (F1-F4) — la fenêtre
        // physique a déjà changé de taille, mais l'entrée egui de cette même
        // trame peut encore décrire l'ancienne. Symptôme observé : la
        // position par défaut, calculée sur une taille de fenêtre périmée,
        // atterrissait hors de la zone visible en x1 (la plus petite, donc
        // celle où l'écart se voit). Interroger SDL directement à chaque
        // appel élimine ce décalage, quelle que soit sa cause exacte.
        let avail = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), window_size);
        let aspect = IMAGE_SIZE.y / IMAGE_SIZE.x;
        let width_from_width_cap = avail.width() * 0.9;
        let width_from_height_cap =
            (avail.height() * settings.default_size_percent.clamp(0.0, 1.0)) / aspect;
        let default_width = width_from_width_cap.min(width_from_height_cap);
        // Coin bas-DROIT de la fenêtre CPC : hors du champ de vision
        // immédiat pendant qu'on joue/tape, contrairement au centre où egui
        // ouvrirait sinon une fenêtre sans position explicite. Le point visé
        // est directement CE coin, combiné à `.pivot(RIGHT_BOTTOM)` plus bas
        // — pas une estimation de la position du coin haut-gauche à partir
        // d'une hauteur de fenêtre devinée (`default_pos` positionne alors
        // le pivot LEFT_TOP par défaut, qu'il aurait fallu compenser
        // manuellement par une hauteur de titre estimée ; imprécis, et
        // seule la marge de calcul le devient encore plus dans un petit
        // clavier x1).
        let margin = 8.0;
        let default_pos = egui::pos2(avail.right() - margin, avail.bottom() - margin);

        let mut active = self.latched.clone();
        // Relâché après la boucle de touches ci-dessous plutôt que pendant :
        // savoir SI une touche momentanée a été cliquée cette trame n'est
        // connu qu'une fois toutes parcourues.
        let mut release_one_shot_latches = false;
        egui::Window::new("Virtual keyboard")
            .id(egui::Id::new(("keyboard_panel_window", generation)))
            .open(open)
            .resizable(true)
            .default_width(default_width)
            .pivot(egui::Align2::RIGHT_BOTTOM)
            .default_pos(default_pos)
            .show(ctx, |ui| {
                let Some(texture) = &self.texture else {
                    ui.label("Couldn't decode the embedded keyboard image — see the console.");
                    return;
                };
                // Largeur réellement disponible dans la fenêtre (bornée par
                // son redimensionnement courant) : l'image ET les zones
                // cliquables sont recalculées à partir d'elle, jamais fixées
                // en pixels d'écran — voir le commentaire d'en-tête du
                // fichier.
                let width = ui.available_width().max(1.0);
                let scale = width / IMAGE_SIZE.x;
                let display_size = IMAGE_SIZE * scale;
                let (image_rect, _) =
                    ui.allocate_exact_size(display_size, egui::Sense::hover());
                ui.painter().image(
                    texture.id(),
                    image_rect,
                    egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );

                // Deux passes plutôt qu'une : SHIFT et RETURN ont chacun
                // deux rectangles pour une seule position PSG (voir plus
                // bas), et un survol/appui sur l'un des deux doit allumer
                // les DEUX — sinon RETURN, dessinée dans l'image source
                // comme une seule touche en L, a l'air de deux touches
                // distinctes dont une seule réagit. Impossible à savoir
                // avant d'avoir appelé `ui.interact` sur tous les
                // rectangles, d'où les deux passes : la première interagit
                // et agrège par position, la seconde dessine avec l'état
                // agrégé.
                let mut rects = Vec::with_capacity(KEYS.len());
                let mut lit_by_position: std::collections::HashMap<Position, (bool, bool)> =
                    std::collections::HashMap::new();

                for (index, key) in KEYS.iter().enumerate() {
                    let screen_rect = egui::Rect::from_min_max(
                        image_rect.min + key.rect.min.to_vec2() * scale,
                        image_rect.min + key.rect.max.to_vec2() * scale,
                    );
                    // Indexé par position de la touche dans `KEYS`, pas par
                    // sa position PSG : SHIFT et RETURN ont chacun DEUX
                    // rectangles qui partagent la même position PSG, et deux
                    // widgets avec le même id egui se marchent dessus (un
                    // seul reçoit effectivement les évènements) — c'est ce
                    // qui rendait SHIFT inopérant avant ce correctif.
                    let id = ui.id().with(("virtual_key", index));
                    let response = ui.interact(screen_rect, id, egui::Sense::click());

                    let pressed = if key.position == CAPS_LOCK {
                        // Impulsion d'une trame, pas un maintien — voir le
                        // commentaire d'en-tête sur pourquoi un vrai loquet
                        // désynchronisait l'état réel du firmware.
                        if response.clicked() {
                            self.caps_lock_display = !self.caps_lock_display;
                            active.insert(key.position);
                        }
                        self.caps_lock_display
                    } else if ONE_SHOT_LATCHES.contains(&key.position) {
                        if response.clicked() && !self.latched.remove(&key.position) {
                            self.latched.insert(key.position);
                        }
                        self.latched.contains(&key.position)
                    } else {
                        if response.clicked() {
                            release_one_shot_latches = true;
                        }
                        let held = response.is_pointer_button_down_on();
                        if held {
                            active.insert(key.position);
                        }
                        held
                    };

                    let entry = lit_by_position.entry(key.position).or_default();
                    entry.0 |= pressed;
                    entry.1 |= response.hovered();
                    rects.push((screen_rect, key.position));
                }

                let stroke_for = |pressed: bool| {
                    egui::Stroke::new(
                        2.0_f32,
                        if pressed {
                            egui::Color32::YELLOW
                        } else {
                            egui::Color32::from_white_alpha(120)
                        },
                    )
                };

                // RETURN : un seul contour en L plutôt que deux rectangles
                // disjoints, qui la font paraître coupée en deux même une
                // fois éclairés ensemble (voir `l_shape_outline`). SHIFT a
                // aussi deux rectangles pour une seule position, mais ce
                // sont deux vraies touches séparées aux deux bouts du
                // clavier — pour elle, deux contours distincts restent
                // corrects.
                let mut return_rects = Vec::with_capacity(2);
                for (screen_rect, position) in rects {
                    if position == RETURN {
                        return_rects.push(screen_rect);
                        continue;
                    }
                    let (pressed, hovered) = lit_by_position[&position];
                    if pressed || hovered {
                        ui.painter().rect_stroke(
                            screen_rect,
                            4.0,
                            stroke_for(pressed),
                            egui::StrokeKind::Inside,
                        );
                    }
                }
                if let [top, bottom] = return_rects[..] {
                    let (pressed, hovered) = lit_by_position[&RETURN];
                    if pressed || hovered {
                        ui.painter().add(egui::Shape::closed_line(
                            l_shape_outline(top, bottom),
                            stroke_for(pressed),
                        ));
                    }
                }
            });

        if release_one_shot_latches {
            for position in ONE_SHOT_LATCHES {
                self.latched.remove(&position);
                active.remove(&position);
            }
        }
        active
    }

    fn load_texture(ctx: &egui::Context) -> Option<egui::TextureHandle> {
        // Embarquée dans le binaire à la compilation (`include_bytes!`), pas
        // lue sur le disque à l'exécution : contrairement aux ROMs (droits
        // non tranchés, jamais distribuées avec l'émulateur — voir
        // doc/roms-installation.md), cette illustration est notre propre
        // travail, rien n'empêche de la distribuer telle quelle. Évite un
        // ancien piège à deux volets : un chemin relatif au répertoire
        // courant ne pointe nulle part de fiable une fois installée par un
        // paquet, et ~/.bytebox/assets/ (l'ancienne convention) n'était créé
        // ni peuplé par personne — même défaut que les ROMs avant leur
        // installeur (F6), mais sans raison de le reproduire ici.
        match image::load_from_memory(include_bytes!("../assets/keyboard.png")) {
            Ok(img) => {
                let img = img.into_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                let color_image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                Some(ctx.load_texture(
                    "virtual_keyboard",
                    color_image,
                    egui::TextureOptions::LINEAR,
                ))
            }
            Err(e) => {
                bytebox_core::app_log!("Can't load the embedded keyboard image: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Le contour en L doit suivre exactement RETURN, insets asymétriques
    /// inclus (28px à gauche, 5px à droite dans l'artwork réel) — pas une
    /// approximation qui supposerait un L symétrique.
    #[test]
    fn l_shape_outline_follows_returns_actual_asymmetric_insets() {
        let top = egui::Rect::from_min_max(egui::pos2(1342.0, 348.0), egui::pos2(1474.0, 439.5));
        let bottom =
            egui::Rect::from_min_max(egui::pos2(1370.0, 439.5), egui::pos2(1469.0, 530.0));

        let outline = l_shape_outline(top, bottom);

        assert_eq!(
            outline,
            vec![
                egui::pos2(1342.0, 348.0),
                egui::pos2(1474.0, 348.0),
                egui::pos2(1474.0, 439.5),
                egui::pos2(1469.0, 439.5),
                egui::pos2(1469.0, 530.0),
                egui::pos2(1370.0, 530.0),
                egui::pos2(1370.0, 439.5),
                egui::pos2(1342.0, 439.5),
            ]
        );
    }

    /// Filet de sécurité contre une coquille de transcription dans le
    /// tableau `KEYS` (76 rectangles saisis à la main depuis les profils de
    /// détection de bord) : chacun doit rester dans les limites de l'image
    /// source et avoir une aire strictement positive.
    #[test]
    fn every_key_rect_stays_within_the_source_image_and_has_positive_area() {
        for key in KEYS {
            assert!(
                key.rect.min.x >= 0.0 && key.rect.max.x <= IMAGE_SIZE.x,
                "rectangle hors limites en x pour {:?} : {:?}",
                key.position,
                key.rect
            );
            assert!(
                key.rect.min.y >= 0.0 && key.rect.max.y <= IMAGE_SIZE.y,
                "rectangle hors limites en y pour {:?} : {:?}",
                key.position,
                key.rect
            );
            assert!(
                key.rect.min.x < key.rect.max.x && key.rect.min.y < key.rect.max.y,
                "rectangle degenere pour {:?} : {:?}",
                key.position,
                key.rect
            );
        }
    }

    /// Deux touches de positions PSG différentes ne doivent jamais se
    /// chevaucher (un clic irait alors à la mauvaise touche selon l'ordre
    /// d'itération) — sauf SHIFT et RETURN, dont les deux rectangles
    /// partagent délibérément la même position (touche unique, deux zones :
    /// SHIFT gauche/droite, RETURN en L sur deux rangées), voir
    /// `KEYS_WITH_TWO_RECTS` plus bas.
    #[test]
    fn key_rects_of_different_positions_never_overlap() {
        for (i, a) in KEYS.iter().enumerate() {
            for b in &KEYS[i + 1..] {
                if a.position == b.position {
                    continue;
                }
                assert!(
                    !a.rect.intersects(b.rect),
                    "chevauchement entre {:?} ({:?}) et {:?} ({:?})",
                    a.position,
                    a.rect,
                    b.position,
                    b.rect
                );
            }
        }
    }

    /// Recense les positions qui apparaissent plus d'une fois : seules
    /// SHIFT (2 zones, gauche/droite) et RETURN (2 zones, une touche en L à
    /// cheval sur deux rangées) doivent l'être. Une troisième occurrence
    /// accidentelle d'une position, ou un doublon inattendu sur une autre
    /// touche, indiquerait une coquille de transcription.
    #[test]
    fn only_shift_and_return_have_two_rects() {
        let mut counts: HashMap<Position, u32> = HashMap::new();
        for key in KEYS {
            *counts.entry(key.position).or_insert(0) += 1;
        }
        let duplicated: Vec<Position> = counts
            .into_iter()
            .filter(|&(_, count)| count > 1)
            .map(|(position, count)| {
                assert_eq!(count, 2, "position {position:?} apparait {count} fois");
                position
            })
            .collect();
        assert_eq!(
            duplicated.len(),
            2,
            "seules SHIFT et RETURN devraient avoir deux rectangles : {duplicated:?}"
        );
        assert!(duplicated.contains(&SHIFT));
        assert!(duplicated.contains(&(2, 2)), "RETURN (2,2)");
    }

    /// Aucune position ne doit dépasser les limites matérielles de la
    /// matrice PSG (10 lignes, 8 bits) : une coquille de transcription (ex.
    /// une ligne à deux chiffres tapée par erreur) plutôt qu'une vraie
    /// touche produirait un index hors bornes silencieusement accepté par
    /// `set_matrix_bit` sinon (`self.keyboard_matrix[line]` painquerait à
    /// l'exécution, mais seulement si cette touche est un jour cliquée).
    #[test]
    fn every_position_fits_the_psg_matrix() {
        for key in KEYS {
            let (line, bit) = key.position;
            assert!(line < 10, "ligne hors bornes pour {:?}", key.position);
            assert!(bit < 8, "bit hors bornes pour {:?}", key.position);
        }
    }
}
