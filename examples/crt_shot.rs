//! Capture hors écran du shader CRT (sans fenêtre) : une image source
//! synthétique façon CPC (800×600, chaque ligne de balayage sur
//! `pixels_per_scanline` rangées) dessinée par `renderer_crt.wgsl` dans une
//! surface de la taille donnée, comme `Renderer::present_rgba`, puis écrite
//! en PNG. Pour régler le shader sans lancer d'émulateur.
//!
//! `cargo run --example crt_shot -- SORTIE.png LARGEUR HAUTEUR [SCANLINES]`

use zilog_silicon::renderer::{crt_shot, CrtSettings};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let out = &a[0];
    let (w, h): (u32, u32) = (a[1].parse().unwrap(), a[2].parse().unwrap());
    let mut s = CrtSettings::default();
    if let Some(v) = a.get(3) {
        s.scanlines = v.parse().unwrap();
    }
    // Source : lignes de balayage isolées (2 rangées chacune), un pavé de
    // « texte » clair sur fond noir, un dégradé.
    let (sw, sh) = (800usize, 600usize);
    let mut rgba = vec![0u8; sw * sh * 4];
    for y in 0..sh {
        let line = y / 2;
        for x in 0..sw {
            let lit = (x < 200 && line % 4 == 0)
                || (200..400).contains(&x) && line % 8 < 5 && (x / 8) % 2 == 0
                || (400..600).contains(&x) && line % 2 == 0;
            let v = if x >= 600 { (line * 255 / 300) as u8 } else if lit { 230 } else { 0 };
            let o = (y * sw + x) * 4;
            rgba[o..o + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    let img = crt_shot(&rgba, sw, sh, w, h, s);
    let file = std::fs::File::create(out).unwrap();
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&img).unwrap();
    eprintln!("{out} : {w}x{h}");
}
