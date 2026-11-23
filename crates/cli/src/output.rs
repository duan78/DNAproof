//! Sortie des commandes : texte lisible, ou un seul document JSON avec `--json`.

use serde::Serialize;
use std::io::Write;

/// Canal de sortie d'une commande.
pub struct Printer {
    json: bool,
}

impl Printer {
    pub fn new(json: bool) -> Self {
        Self { json }
    }

    /// Vrai si la commande doit se taire (sortie JSON : un seul document, rien d'autre).
    pub fn is_json(&self) -> bool {
        self.json
    }

    /// Émet le résultat de la commande.
    pub fn finish<T: Serialize>(
        &self,
        report: &T,
        summary: impl FnOnce() -> String,
    ) -> anyhow::Result<()> {
        if self.json {
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer_pretty(&mut stdout, report)?;
            stdout.write_all(b"\n")?;
            stdout.flush()?;
        } else {
            print!("{}", summary());
        }
        Ok(())
    }
}

/// Écrit des champs « libellé : valeur » alignés.
pub struct Fields {
    width: usize,
    rows: Vec<(String, String)>,
}

impl Fields {
    pub fn new() -> Self {
        Self {
            width: 0,
            rows: Vec::new(),
        }
    }

    pub fn add(&mut self, label: impl Into<String>, value: impl std::fmt::Display) -> &mut Self {
        let label = label.into();
        self.width = self.width.max(label.chars().count());
        self.rows.push((label, value.to_string()));
        self
    }

    /// Rend les champs, chacun sur une ligne, indenté de `indent` espaces.
    pub fn render(&self, indent: usize) -> String {
        let mut out = String::new();
        for (label, value) in &self.rows {
            let pad = " ".repeat(self.width - label.chars().count());
            out.push_str(&format!(
                "{indent}{label}{pad} : {value}\n",
                indent = " ".repeat(indent)
            ));
        }
        out
    }
}

/// Formate un entier avec des espaces insécables fins comme séparateurs de milliers.
pub fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push('\u{202F}');
        }
        out.push(digit);
    }
    out
}

/// Formate une durée en secondes.
pub fn seconds(value: f64) -> String {
    format!("{value:.2} s")
}

/// Taille lisible en octets.
pub fn bytes(value: u64) -> String {
    const UNITS: [&str; 5] = ["o", "Kio", "Mio", "Gio", "Tio"];
    let mut size = value as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit + 1 < UNITS.len() {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", thousands(value), UNITS[unit])
    } else {
        format!("{size:.2} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatting_helpers() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(1234), "1\u{202F}234");
        assert_eq!(thousands(1_000_000), "1\u{202F}000\u{202F}000");
        assert_eq!(bytes(999), "999 o");
        assert_eq!(bytes(1024), "1.00 Kio");
        assert_eq!(bytes(5 * 1024 * 1024), "5.00 Mio");
    }

    #[test]
    fn fields_are_aligned() {
        let mut fields = Fields::new();
        fields.add("Oligos", 12).add("GC", "50 %");
        let text = fields.render(2);
        assert_eq!(text, "  Oligos : 12\n  GC     : 50 %\n");
    }
}
