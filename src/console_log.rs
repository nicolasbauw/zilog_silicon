//! Historique de commandes partagé entre les deux façades console (Plan
//! V2.md, jalon M2) : la barre rapide (F10, `console_panel.rs`) n'en montre
//! que la dernière ligne, la console complète (F11, `console_window.rs`)
//! l'affiche en entier, défilant. Les deux alimentent le même journal, donc
//! une commande tapée dans l'une apparaît aussi dans l'autre.

pub struct ConsoleLog {
    lines: Vec<String>,
}

impl ConsoleLog {
    pub fn new() -> Self {
        Self { lines: Vec::new() }
    }

    /// Ajoute la commande telle que tapée, préfixée comme un prompt.
    pub fn push_command(&mut self, line: &str) {
        self.lines.push(format!("> {line}"));
    }

    /// Ajoute la sortie d'une commande déjà traitée (voir
    /// `Machine::console_handle`), une ligne du journal par ligne de texte.
    pub fn push_output(&mut self, output: &str) {
        for line in output.lines() {
            self.lines.push(line.to_string());
        }
    }

    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// La toute dernière ligne du journal, pour la barre rapide (F10) dont
    /// l'affichage ne doit jamais dépasser une ligne.
    pub fn last_line(&self) -> Option<&str> {
        self.lines.last().map(String::as_str)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_line_reflects_the_most_recent_push() {
        let mut log = ConsoleLog::new();
        assert_eq!(log.last_line(), None);
        log.push_command("disk foo.dsk");
        assert_eq!(log.last_line(), Some("> disk foo.dsk"));
        log.push_output("Floppy DSK Loaded on drive A: foo.dsk");
        assert_eq!(
            log.last_line(),
            Some("Floppy DSK Loaded on drive A: foo.dsk")
        );
    }

    #[test]
    fn multiline_output_becomes_one_log_entry_per_line() {
        let mut log = ConsoleLog::new();
        log.push_output("line 1\nline 2\nline 3");
        assert_eq!(log.lines(), ["line 1", "line 2", "line 3"]);
    }
}
