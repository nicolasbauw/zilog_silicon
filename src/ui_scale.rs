//! Facteur d'agrandissement du contenu des panneaux egui superposés à la
//! fenêtre principale (F6, F10 — pas F11/F12, fenêtres SDL2 séparées avec
//! leur propre contexte egui, voir `egui_gpu.rs`), en fonction du niveau de
//! zoom courant (F1-F4).
//!
//! Partagé plutôt que dupliqué dans chaque panneau : `scaled_style` fait une
//! quinzaine de lignes, et la formule de `content_scale` doit rester
//! identique pour que F6 et F10 grossissent au même rythme.

/// Seuil en dessous duquel `scale = 1.0` (taille d'origine, inchangée) :
/// une 4K (3840px) mise à l'échelle à 135% par le bureau (KDE, GNOME...),
/// soit 3840/1.35 ≈ 2844px — le cas réel qui a motivé cette valeur plutôt
/// que la 4K "nue". Un affichage réellement plus modeste (Full HD, 1440p,
/// ou une 4K mise à l'échelle à ce point ou plus) n'a pas besoin d'être
/// grossi.
const NO_SCALE_BELOW: f32 = 3840.0 / 1.35;

/// Au-delà de `NO_SCALE_BELOW`, grossit progressivement — plus doucement
/// qu'avant (le panneau F6 en plein écran sur une 4K à 135% doit paraître
/// "agrandi d'un tiers", pas doublé) : `scale` atteint 1.33 pour une
/// fenêtre large de 3840px (une 4K réellement sans mise à l'échelle), puis
/// continue au même rythme jusqu'au plafond de 2.5x, atteint vers 7300px
/// (5K/6K/8K).
const RAMP: f32 = 3000.0;

/// `scale = 1.0` (taille d'origine, inchangée) tant que la fenêtre CPC
/// reste sous `NO_SCALE_BELOW` — pas seulement en x1 : x2/x3 sur un écran
/// Full HD ou 1440p restent en dessous de ce seuil, et n'ont donc pas
/// besoin d'être grossis. Une ancienne version basait le calcul sur 800px,
/// ce qui grossissait déjà fortement le panneau en x2/x3/plein écran sur
/// n'importe quel écran, 4K ou non — proportionné à la taille de la
/// fenêtre en pixels bruts, pas à la résolution réelle de l'écran.
pub fn content_scale(window_size: egui::Vec2) -> f32 {
    (1.0 + (window_size.x - NO_SCALE_BELOW) / RAMP).clamp(1.0, 2.5)
}

/// Grossit tout ce qui détermine la taille visuelle du contenu — tailles de
/// police et espacements — plutôt que la seule largeur d'un panneau.
/// `egui::Context::set_pixels_per_point` ferait la même chose, mais pour
/// TOUS les panneaux en même temps (F6/F7/F10 partagent le même contexte
/// egui, voir `renderer.rs`) : une bascule propre à un seul panneau passe
/// donc par le style de son propre `Ui`, qui ne fuit pas vers les autres.
pub fn scaled_style(base: &egui::Style, scale: f32) -> egui::Style {
    let mut style = base.clone();
    for font_id in style.text_styles.values_mut() {
        font_id.size *= scale;
    }
    let spacing = &mut style.spacing;
    spacing.item_spacing *= scale;
    spacing.button_padding *= scale;
    spacing.interact_size *= scale;
    spacing.slider_width *= scale;
    spacing.combo_width *= scale;
    spacing.text_edit_width *= scale;
    spacing.icon_width *= scale;
    spacing.icon_width_inner *= scale;
    spacing.icon_spacing *= scale;
    spacing.indent *= scale;
    style
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Full HD et 1440p, même en x3/plein écran (donc la pleine largeur de
    /// l'écran), ne doivent jamais grossir le contenu. Une 4K mise à
    /// l'échelle à 135% par le bureau (le cas qui a fixé ce seuil) non plus.
    #[test]
    fn resolutions_at_or_below_a_scaled_4k_never_scale() {
        assert_eq!(content_scale(egui::vec2(1920.0, 1080.0)), 1.0);
        assert_eq!(content_scale(egui::vec2(2560.0, 1440.0)), 1.0);
        assert_eq!(
            content_scale(egui::vec2(3840.0 / 1.35, 2160.0 / 1.35)),
            1.0,
            "4K a 135% : encore 1.0"
        );
    }

    /// Une vraie 4K sans mise à l'échelle du bureau doit paraître "agrandie
    /// d'un tiers" (1.33), pas doublée.
    #[test]
    fn a_real_unscaled_4k_is_enlarged_by_a_third() {
        let scale = content_scale(egui::vec2(3840.0, 2160.0));
        assert!((scale - 1.33).abs() < 0.01, "attendu ~1.33, obtenu {scale}");
    }

    #[test]
    fn further_above_4k_scales_up_to_the_2_5_cap() {
        let mid = content_scale(egui::vec2(5844.0, 3290.0));
        assert!((mid - 2.0).abs() < 0.01, "attendu ~2.0, obtenu {mid}");
        assert_eq!(content_scale(egui::vec2(8000.0, 4500.0)), 2.5, "au-dela du plafond, deja atteint");
        assert_eq!(content_scale(egui::vec2(10000.0, 5625.0)), 2.5, "plafonne, ne depasse jamais 2.5");
    }
}
