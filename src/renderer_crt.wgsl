// Shader CRT (F5, Plan V2.md jalon M4) : troisième réécriture.
//
// Les deux précédentes (assombrissement du pixel selon sa position dans son
// propre texel, puis reconstruction gaussienne du faisceau) restaient trop
// éloignées du rendu d'un vrai CRT selon le retour utilisateur. On ne peut
// pas reprendre le code d'un shader RetroArch existant (Mega Bezel, CyberLab,
// crt-easymode...) : ce sont tous des shaders GPLv3, et ByteBox est MIT — les
// intégrer littéralement en ferait une œuvre dérivée GPL. En revanche, la
// TECHNIQUE de crt-easymode est documentée publiquement
// (https://docs.libretro.com/shader/crt/, et son algorithme analysé depuis
// son code source public) : on s'en inspire ici pour une implémentation WGSL
// entièrement réécrite à partir de zéro, avec nos propres noms et notre
// propre structure — pas une traduction du fichier GPL.
//
// Deux idées qui manquaient aux essais précédents :
//
// - Un vrai CRT n'assombrit pas un pixel carré de façon isotrope : il balaie
//   des LIGNES, chacune un point lumineux continu qui déborde horizontalement
//   sur ses voisines (d'où un flou horizontal doux) mais reste une bande fine
//   verticalement (d'où des bandes de balayage nettes, pas un flou vertical).
//   Le flou gaussien symétrique du deuxième essai traitait X et Y de la même
//   façon modulo un sigma différent ; ici on distingue franchement : un
//   simple lissage cosinus entre les deux colonnes voisines à l'horizontale,
//   et une vraie courbe de "profil de faisceau" (cos²) à la verticale, qui
//   RESTE NON NORMALISÉE entre deux lignes pour creuser un vrai espace sombre
//   entre elles (contrairement à une moyenne pondérée normalisée, qui donne
//   toujours l'illusion d'une pleine luminosité).
//
// - Le masque phosphore d'un vrai tube n'est pas un point qui s'assombrit :
//   c'est une TRIADE de sous-pixels rouge/vert/bleu. Un masque qui se
//   contente de moduler la luminosité, comme dans les deux essais
//   précédents, ne peut jamais ressembler à la texture fine et colorée
//   visible sur la référence RetroArch fournie par l'utilisateur. Ici,
//   chaque pixel de sortie appartient à une colonne R, G ou B (motif qui se
//   répète tous les 3 pixels d'écran, avec un décalage d'une colonne sur les
//   lignes impaires — le "staggering" d'un vrai masque perforé) et voit sa
//   couleur pondérée en conséquence.
//
// Le mélange se fait en espace linéaire (le mixage additif de lumière n'a de
// sens qu'après avoir défait la correction gamma de la source), avant de
// ré-encoder en sortie.

// Les six derniers champs sont réglables en direct depuis le panneau de
// configuration (F6, section "Shader CRT") : voir `CrtSettings` côté Rust
// (`renderer.rs`) pour leurs valeurs par défaut, et les commentaires plus
// bas pour ce que chacun contrôle visuellement.
struct CrtParams {
    source_size: vec2<f32>,
    /// Hauteur d'une VRAIE ligne de balayage CPC, en lignes du tampon
    /// source (`video::PIXELS_PER_SCANLINE`). Vaut 2 : `video::render`
    /// double chaque scanline verticalement, donc les 600 lignes du tampon
    /// ne sont que 300 lignes de balayage réelles. Dessiner une scanline par
    /// ligne de tampon en dessinerait deux fois trop, chacune deux fois trop
    /// fine — ce qui les rendait presque invisibles quel que soit le réglage.
    line_height: f32,
    mask_cell_px: f32,
    mask_min: f32,
    mask_strength: f32,
    scanline_beam: f32,
    scanline_strength: f32,
    beam_bloom: f32,
    bright_boost: f32,
    /// Écart-type du spot du faisceau à l'HORIZONTALE, en pixels source.
    /// C'est la bande passante limitée du signal analogique : sur un vrai
    /// tube, le faisceau balaie chaque ligne en continu, donc rien ne s'y
    /// termine par une arête franche — mais seulement dans ce sens, chaque
    /// ligne étant balayée séparément (à la verticale, c'est
    /// `scanline_beam` qui décide, et lui doit rester étroit pour creuser
    /// les scanlines). Tendant vers 0, seul le texel le plus proche pèse :
    /// on retombe exactement sur le pixel net d'origine.
    horizontal_blur: f32,
    // Les 11 `f32` qui précèdent font 44 octets ; ce dernier les porte à 48,
    // un multiple de 16 comme l'exige un uniforme — et la taille exacte de
    // `CrtParams` côté Rust.
    _padding: f32,
};

@group(0) @binding(0)
var t_frame: texture_2d<f32>;
@group(0) @binding(1)
var s_frame: sampler;
@group(1) @binding(0)
var<uniform> params: CrtParams;

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) idx: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 4>(
        vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(1.0, 1.0),
        vec2<f32>(1.0, -1.0),
    );
    var uvs = array<vec2<f32>, 4>(
        vec2<f32>(0.0, 0.0),
        vec2<f32>(0.0, 1.0),
        vec2<f32>(1.0, 0.0),
        vec2<f32>(1.0, 1.0),
    );
    var out: VertexOutput;
    out.clip_position = vec4<f32>(positions[idx], 0.0, 1.0);
    out.uv = uvs[idx];
    return out;
}

const HALF_PI: f32 = 1.57079633;

// Gamma de linéarisation à l'entrée / de ré-encodage à la sortie : mélanger
// des couleurs directement en espace gamma (sRGB-like) assombrit trop les
// zones de transition. Valeurs usuelles pour ce type de shader.
const GAMMA_IN: f32 = 2.4;
const GAMMA_OUT: f32 = 2.2;

// SCANLINE_BEAM (exposant du profil de faisceau, cos² à une puissance) :
// plus grand = pic lumineux plus étroit, donc creux plus large entre deux
// lignes. SCANLINE_STRENGTH : force de ce creux, 0 = pas de bande visible,
// 1 = obscurité totale entre deux lignes de balayage. MASK_MIN : luminosité
// résiduelle des deux sous-pixels "éteints" d'une triade de masque (0 =
// uniquement la couleur dominante, 1 = pas de masque du tout). MASK_STRENGTH :
// force globale du masque. MASK_CELL_PX : largeur d'une colonne de la triade,
// en pixels de SORTIE réels — donc la triade complète (3 colonnes) fait
// 3 x MASK_CELL_PX de large à l'écran ; sur un écran haute densité (4K), une
// seule colonne de sortie par sous-pixel (1.0) est trop fine pour rester
// visible, d'où un défaut plus grand. BRIGHT_BOOST : compense la perte de
// luminosité moyenne entraînée par les bandes de balayage et le masque.
// Toutes réglables en direct depuis F6 (voir le commentaire de `CrtParams`
// ci-dessus) : les valeurs ci-dessous ne servent que si le panneau n'a
// jamais touché les réglages, elles doivent rester synchronisées avec
// `CrtSettings::default()` côté Rust.

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(GAMMA_IN));
}

fn to_display(c: vec3<f32>) -> vec3<f32> {
    return pow(max(c, vec3<f32>(0.0)), vec3<f32>(1.0 / GAMMA_OUT));
}

// Nombre de colonnes voisines sommées de part et d'autre pour le flou
// horizontal. 2 (donc 5 colonnes) suffit tant que `horizontal_blur` reste
// sous ~1 pixel source : au-delà de 2,5 écarts-types, le poids gaussien
// tombe sous 5 % et tronquer ne se voit pas. C'est ce qui borne la plage du
// curseur correspondant, côté panneau F6.
const BLUR_TAPS: i32 = 2;

// Profil du faisceau à la verticale : pic à 1.0 pile sur le centre d'une
// ligne source, retombe à 0.0 à mi-chemin de la ligne voisine. Volontairement
// NON normalisé par l'appelant (voir fs_main) : c'est cette absence de
// normalisation entre deux lignes qui creuse la bande sombre du balayage.
fn scan_weight(dist_lines: f32, beam: f32) -> f32 {
    let c = cos(clamp(dist_lines, -1.0, 1.0) * HALF_PI);
    return pow(max(c, 0.0), beam);
}

// Moyenne le profil de faisceau sur l'empreinte verticale d'un pixel de
// SORTIE (`footprint`, en lignes source — voir son calcul dans fs_main via
// `fwidth`). Un simple échantillon ponctuel se contenterait de lire
// `scan_weight` pile à `dist_lines`, mais à x1 (une ligne source = un pixel
// de sortie) ce point tombe systématiquement sur le centre de la ligne
// (`dist_lines` ~ 0), là où `scan_weight` vaut son maximum : le creux entre
// deux lignes n'est alors JAMAIS échantillonné, et l'effet disparaît
// entièrement — pas un simple affaiblissement, une absence totale, d'où le
// saut de luminosité observé en passant à x1. Moyenner sur trois points
// répartis dans l'empreinte du pixel restitue une valeur représentative même
// quand cette empreinte fait une ligne source entière.
fn average_scan_weight(dist_lines: f32, beam: f32, footprint: f32) -> f32 {
    let h = footprint * 0.5;
    let a = scan_weight(dist_lines - h, beam);
    let b = scan_weight(dist_lines, beam);
    let c = scan_weight(dist_lines + h, beam);
    return (a + b + c) / 3.0;
}

// Couleur de la ligne de balayage CPC `line` (pas la ligne de tampon : voir
// `line_height`), ré-échantillonnée horizontalement entre ses deux colonnes
// voisines autour de `cont_x` (position continue, en texels source).
fn sample_line(cont_x: f32, line: f32) -> vec3<f32> {
    // Centre du groupe de `line_height` lignes de tampon qui portent cette
    // ligne de balayage. Elles sont identiques (`video::render` duplique),
    // donc n'importe laquelle ferait l'affaire — viser le centre garde
    // l'échantillonnage robuste si le doublage venait à changer.
    let row = (line + 0.5) * params.line_height;

    // Somme gaussienne centrée sur la position continue du fragment, plutôt
    // qu'une interpolation entre les deux seuls texels encadrants : c'est ce
    // qui permet au spot de déborder au-delà de ses voisins immédiats quand
    // on élargit le faisceau, et donc au réglage d'aller quelque part.
    // Un plancher sur l'écart-type évite la division par zéro (et le NaN qui
    // s'ensuivrait) quand le curseur est à fond à gauche ; à cette valeur,
    // le texel le plus proche emporte déjà tout le poids.
    let sigma = max(params.horizontal_blur, 0.03);
    let nearest = floor(cont_x) + 0.5;
    var sum = vec3<f32>(0.0);
    var weight_sum = 0.0;
    for (var k = -BLUR_TAPS; k <= BLUR_TAPS; k++) {
        let col = nearest + f32(k);
        let d = cont_x - col;
        let w = exp(-(d * d) / (2.0 * sigma * sigma));
        let uv = vec2<f32>(col, row) / params.source_size;
        sum += to_linear(textureSample(t_frame, s_frame, uv).rgb) * w;
        weight_sum += w;
    }
    return sum / weight_sum;
}

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Reconstruction du faisceau, en espace pixel SOURCE (ni le flou
    // horizontal ni le pas des bandes de balayage ne doivent dépendre du
    // zoom de sortie : x1/x2/x3/plein écran doivent tous montrer le même
    // nombre de bandes, dans les mêmes proportions relatives à l'image).
    let texel = in.uv * params.source_size;
    // Position verticale en LIGNES DE BALAYAGE CPC réelles, pas en lignes du
    // tampon (voir `line_height`) : c'est la période à laquelle un vrai tube
    // dessine ses scanlines.
    let line_coord = texel.y / params.line_height;
    // `fwidth` donne l'empreinte verticale d'un pixel de sortie, dans cette
    // même unité — voir `average_scan_weight`.
    let footprint = fwidth(line_coord);
    let line0 = floor(line_coord - 0.5);
    let dist0 = (line_coord - 0.5) - line0;

    let c0 = sample_line(texel.x, line0);
    let c1 = sample_line(texel.x, line0 + 1.0);
    // Un mélange linéaire non pondéré (sans creux de balayage) sert de plancher
    // de luminosité : scanline_strength règle l'intensité du seul effet de
    // bande, indépendamment de GAMMA_IN/OUT ou du reste du pipeline.
    let color_flat = mix(c0, c1, dist0);

    // Largeur du faisceau selon la luminosité locale : sur un vrai tube, un
    // faisceau plus intense est physiquement plus large ("bloom" du spot),
    // donc les zones claires montrent des scanlines plus discrètes que les
    // zones sombres. Sans ça, le seul moyen de rattraper l'assombrissement
    // global qu'imposent les scanlines est un facteur multiplicatif
    // (`bright_boost`) — mais il sature les blancs, et écrase justement le
    // contraste des scanlines qu'on cherchait à renforcer : d'où le "il faut
    // tout mettre à fond et elles restent discrètes". Avec le bloom, les
    // blancs restent blancs sans boost, et l'exposant peut monter beaucoup
    // plus haut là où ça se voit.
    let luma = dot(color_flat, vec3<f32>(0.2126, 0.7152, 0.0722));
    // Comparé en espace perceptuel plutôt que linéaire : sinon la quasi
    // totalité de l'image (tout sauf les blancs francs) compterait comme
    // "sombre" et le bloom ne servirait presque jamais.
    let brightness = pow(clamp(luma, 0.0, 1.0), 1.0 / GAMMA_OUT);
    let beam = params.scanline_beam * mix(1.0, params.beam_bloom, brightness);

    let w0 = average_scan_weight(dist0, beam, footprint);
    let w1 = average_scan_weight(dist0 - 1.0, beam, footprint);
    let color_scanned = c0 * w0 + c1 * w1;
    let color = mix(color_flat, color_scanned, params.scanline_strength);

    // Masque phosphore : triade RVB, en pixels de SORTIE réels (propriété du
    // tube, sans rapport avec la résolution de l'image source) — décalée
    // d'une colonne sur les lignes impaires, comme un vrai masque perforé.
    let out_x = i32(floor(in.clip_position.x / params.mask_cell_px));
    let out_y = i32(floor(in.clip_position.y / params.mask_cell_px));
    let stagger = out_y % 2;
    let phase = (out_x + stagger) % 3;
    var mask_color: vec3<f32>;
    if (phase == 0) {
        mask_color = vec3<f32>(1.0, params.mask_min, params.mask_min);
    } else if (phase == 1) {
        mask_color = vec3<f32>(params.mask_min, 1.0, params.mask_min);
    } else {
        mask_color = vec3<f32>(params.mask_min, params.mask_min, 1.0);
    }
    let mask = mix(vec3<f32>(1.0), mask_color, params.mask_strength);

    // bright_boost multiplie en espace LINÉAIRE, avant le ré-encodage gamma
    // — pas après. Appliqué après (comme dans un essai précédent), un simple
    // facteur constant sur des valeurs déjà ré-encodées éclaircit les creux
    // de balayage bien plus que les crêtes (la courbe gamma remonte déjà les
    // tons sombres davantage que les tons clairs), ce qui masquait une
    // bonne partie du contraste que scanline_beam/scanline_strength étaient
    // censés produire — augmenter ces deux réglages n'avait alors presque
    // plus d'effet visible une fois `to_display` et le boost appliqués.
    let final_color = to_display(color * mask * params.bright_boost);
    return vec4<f32>(final_color, 1.0);
}
