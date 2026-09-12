use std::process::Command;

// Titre de fenêtre (sdl.rs) : "ByteBox - <hash>" pour un build depuis un
// checkout git (dépôt cloné, `cargo build` local ou packaging/PKGBUILD-git),
// "ByteBox - <version>" pour un build depuis l'archive d'une release taguée
// (packaging/PKGBUILD, qui construit depuis un tarball GitHub sans .git) —
// `git rev-parse` échoue naturellement dans ce dernier cas, d'où le fallback
// sur CARGO_PKG_VERSION côté sdl.rs plutôt qu'un test explicite ici.
fn main() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/index");

    // Icône du .exe compilé (ressource PE, voir windows.rc) : sans elle,
    // l'Explorateur Windows affiche l'icône générique par défaut pour
    // bytebox.exe, avant même de le lancer — indépendant de l'icône que
    // set_window_icon (sdl.rs) pose sur la fenêtre une fois lancée, et de
    // celle du raccourci Menu Démarrer (bytebox/wix/main.wxs). No-op sur
    // les autres cibles : safe à appeler sans condition `cfg(windows)`.
    println!("cargo:rerun-if-changed=windows.rc");
    // manifest_optional() : seul un `Failed` (compilateur de ressources
    // présent mais qui échoue) doit interrompre le build — `NotWindows`
    // (toute autre cible) et `NotAttempted` (pas de compilateur trouvé, ce
    // qui ne devrait pas arriver sur les runners CI Windows utilisés, mais
    // ne doit pas non plus faire échouer un `cargo build` local sur une
    // machine Windows mal équipée) sont sans conséquence.
    embed_resource::compile("windows.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();

    if let Ok(output) = Command::new("git").args(["rev-parse", "HEAD"]).output()
        && output.status.success()
    {
        let hash = String::from_utf8_lossy(&output.stdout);
        let hash = hash.trim();
        // Les 7 DERNIERS caractères du hash complet, pas les 7 premiers
        // (= `git rev-parse --short`, l'abréviation habituelle) : demandé
        // tel quel, pour distinguer ce hash de version des hash courts déjà
        // utilisés ailleurs (logs de commit...).
        let suffix = &hash[hash.len().saturating_sub(7)..];
        println!("cargo:rustc-env=BYTEBOX_GIT_HASH={suffix}");
    }
}
