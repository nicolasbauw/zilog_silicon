mod audio;
mod config_panel;
mod console_log;
mod console_panel;
mod console_window;
mod egui_gpu;
mod keyboard_panel;
mod osd;
mod renderer;
mod rom_install_panel;
mod sdl;
mod status_panel;
mod ui_scale;

use bytebox_core::app_log;
use bytebox_core::autotype;
use bytebox_core::machine::Machine;
use std::env;

/// Cherche la valeur d'une option `--nom=valeur`, `--nom valeur`, `-nom=valeur`
/// ou `-nom valeur` parmi les arguments de la ligne de commande. `names`
/// donne les formes acceptées (par ex. `["--autocmd", "-a"]`), pour ne pas
/// obliger l'utilisateur à retenir laquelle est la forme longue.
fn cli_value(args: &[String], names: &[&str]) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        for name in names {
            if let Some(v) = arg.strip_prefix(&format!("{name}=")) {
                return Some(v.to_string());
            }
            if arg == name {
                return args.get(i + 1).cloned();
            }
        }
    }
    None
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    app_log!("=== Amstrad CPC 6128 ===");

    // 1. Analyse des arguments de la ligne de commande pour le choix du mode
    let args: Vec<String> = env::args().collect();
    let mut diag_mode = false; // Par défaut, on démarre en mode normal

    if args.contains(&"--diag".to_string()) {
        diag_mode = true;
    }

    // Injection d'une commande BASIC au démarrage, comme le fait Caprice32
    // avec --autocmd. Utile pour lancer directement un jeu sans repasser par
    // le clavier à chaque essai — le cas d'usage qui a motivé cette option
    // est justement le débogage répété d'un même jeu, disquette après
    // disquette, pendant les séances de mise au point.
    let autocmd = cli_value(&args, &["--autocmd", "-a"]);

    // Chargement direct d'une image disque sur le lecteur A, sans passer par
    // la console. "-d" (forme courte) est accepté en plus de la forme longue
    // habituelle, pour rester proche de la syntaxe Caprice32.
    let disk = cli_value(&args, &["--disk", "-d"]);

    // Chargement direct d'une image cassette dans le lecteur, même principe
    // que --disk.
    let tape = cli_value(&args, &["--tape", "-t"]);

    // Reprise directe d'un instantané .SNA. Pensé pour le cycle "assemble
    // puis teste" : RASM sait produire un .SNA prêt à tourner à partir du
    // code assemblé, donc `bytebox --snapshot=jeu.sna` remplace tout le
    // détour par une image disque à chaque essai.
    let snapshot = cli_value(&args, &["--snapshot", "-s"]);

    // 2. Initialisation de la Machine
    let mut machine = Machine::new();
    machine.diagnostic_mode = diag_mode;
    // Pas de `?` ici : sur un premier lancement (ou une installation via
    // gestionnaire de paquets sans les ROMs elles-mêmes, leur statut légal
    // n'étant pas tranché — voir doc/roms-installation.md), aucune ROM
    // n'existe encore dans ~/.bytebox/ROM. Autrefois, ça faisait échouer le
    // programme avant même l'ouverture d'une fenêtre, sans aucun message
    // exploitable pour qui que ce soit qui ne lit pas les journaux. `sdl::run`
    // ouvre maintenant la fenêtre de toute façon (la machine tourne sur une
    // ROM vide, inoffensif — NOP en boucle) et route automatiquement vers
    // l'écran d'installation des ROMs (F6, onglet ROMs) si elles manquent.
    let roms_missing = machine.load_roms().is_err();
    if roms_missing {
        app_log!("No ROMs found in ~/.bytebox/ROM — opening the ROM installer.");
    }

    if let Some(path) = &disk
        && let Err(e) = machine.load_disk(path)
    {
        app_log!("Can't load disk '{path}': {e}");
    }

    if let Some(path) = &tape
        && let Err(e) = machine.load_tape(path)
    {
        app_log!("Can't load tape '{path}': {e}");
    }

    // Après les médias, jamais avant : restaurer un instantané fait repasser
    // la machine par un cycle d'alimentation (voir `snapshot::load`), qui
    // conserve les disquettes et la cassette insérées mais réinitialise tout
    // le reste — l'ordre inverse annulerait donc la restauration.
    if let Some(path) = &snapshot
        && let Err(e) = machine.load_snapshot(path)
    {
        app_log!("Can't load snapshot '{path}': {e}");
    }

    let autotyper = autocmd
        .as_deref()
        .map(|cmd| autotype::AutoTyper::new(&autotype::ensure_validated(cmd)));

    // 3. Fenêtrage SDL2 et boucle principale, jusqu'à la fermeture.
    sdl::run(machine, autotyper, roms_missing)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cli_value_accepts_the_equals_and_the_space_forms() {
        let args: Vec<String> = ["prog", "--autocmd=RUN\"A", "-d", "d.dsk"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        assert_eq!(
            cli_value(&args, &["--autocmd", "-a"]),
            Some("RUN\"A".to_string())
        );
        assert_eq!(
            cli_value(&args, &["--disk", "-d"]),
            Some("d.dsk".to_string())
        );
        assert!(!args.contains(&"--diag".to_string()));
    }
}
