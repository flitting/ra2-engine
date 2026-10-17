//! The game's INI dialect: `[Section]` headers, `key=value` lines, `;` comments.
//! Section and key lookups ignore case.

pub struct Ini {
    sections: Vec<(String, Vec<(String, String)>)>,
}

impl Ini {
    pub fn parse(data: &[u8]) -> Self {
        let text = String::from_utf8_lossy(data);
        let mut sections: Vec<(String, Vec<(String, String)>)> = Vec::new();
        for line in text.lines() {
            let line = line.split(';').next().unwrap_or("").trim();
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.split(']').next()) {
                sections.push((name.trim().to_owned(), Vec::new()));
            } else if let (Some((key, value)), Some(section)) =
                (line.split_once('='), sections.last_mut())
            {
                section.1.push((key.trim().to_owned(), value.trim().to_owned()));
            }
        }
        Self { sections }
    }

    /// Lays another file's sections over this one's: a key already here takes the
    /// new value, others are added, and sections not here are added whole.
    pub fn overlay(&mut self, other: &Ini) {
        for (name, entries) in &other.sections {
            let at = match self.sections.iter().position(|(section, _)| section.eq_ignore_ascii_case(name)) {
                Some(at) => at,
                None => {
                    self.sections.push((name.clone(), Vec::new()));
                    self.sections.len() - 1
                }
            };
            for (key, value) in entries {
                match self.sections[at].1.iter_mut().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
                    Some(entry) => entry.1 = value.clone(),
                    None => self.sections[at].1.push((key.clone(), value.clone())),
                }
            }
        }
    }

    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.sections.iter().map(|(name, _)| name.as_str())
    }

    /// The entries of a section, in file order.
    pub fn section(&self, name: &str) -> &[(String, String)] {
        self.sections
            .iter()
            .find(|(section, _)| section.eq_ignore_ascii_case(name))
            .map(|(_, entries)| entries.as_slice())
            .unwrap_or(&[])
    }

    /// A value by the key spelt exactly, capitals included, as the game itself
    /// looks keys up.
    // Keys are matched on their raw bytes.
    pub fn get_exact(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section).iter().find(|(k, _)| k == key).map(|(_, value)| value.as_str())
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section(section)
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
    }
}
