//! A line-preserving editor for Spicetify's config-xpui.ini. Comments, order and spacing survive a write;
//! only the values Pantry sets change.

use std::path::Path;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Ini {
    lines: Vec<String>,
}

fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let t = line.trim_start();
    if t.starts_with(';') || t.starts_with('#') || t.starts_with('[') {
        return None;
    }
    let (k, v) = line.split_once('=')?;
    Some((k.trim(), v.trim()))
}

fn section_of(line: &str) -> Option<&str> {
    let t = line.trim();
    t.strip_prefix('[')?.strip_suffix(']')
}

impl Ini {
    pub fn parse(text: &str) -> Ini {
        Ini { lines: text.lines().map(str::to_owned).collect() }
    }

    pub fn load(p: &Path) -> std::io::Result<Ini> {
        Ok(Ini::parse(&std::fs::read_to_string(p)?))
    }

    pub fn to_text(&self) -> String {
        let mut s = self.lines.join("\r\n");
        s.push_str("\r\n");
        s
    }

    pub fn save(&self, p: &Path) -> std::io::Result<()> {
        std::fs::write(p, self.to_text())
    }

    /// (line index, key, value) for every key in `section`.
    fn entries(&self, section: &str) -> Vec<(usize, &str, &str)> {
        let mut cur = None;
        let mut out = vec![];
        for (i, l) in self.lines.iter().enumerate() {
            if let Some(s) = section_of(l) {
                cur = Some(s);
            } else if cur == Some(section) {
                if let Some((k, v)) = parse_kv(l) {
                    out.push((i, k, v));
                }
            }
        }
        out
    }

    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries(section).into_iter().find(|(_, k, _)| *k == key).map(|(_, _, v)| v)
    }

    /// `extensions = a.js|b.js` → `["a.js", "b.js"]`.
    pub fn get_list(&self, section: &str, key: &str) -> Vec<String> {
        self.get(section, key)
            .map(|v| v.split('|').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect())
            .unwrap_or_default()
    }

    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        if let Some((i, _, _)) = self.entries(section).into_iter().find(|(_, k, _)| *k == key) {
            let l = &self.lines[i];
            let eq = l.find('=').unwrap();
            // Keep the original alignment: everything up to and including "= ".
            let head = &l[..=eq];
            let pad = if l[eq + 1..].starts_with(' ') { " " } else { "" };
            self.lines[i] = format!("{head}{pad}{value}");
            return;
        }
        let at = self.lines.iter().position(|l| section_of(l) == Some(section));
        match at {
            Some(s) => {
                let mut end = s + 1;
                while end < self.lines.len() && section_of(&self.lines[end]).is_none() {
                    end += 1;
                }
                // Skip back over blank lines and comments that introduce the next section.
                while end > s + 1 && { let t = self.lines[end - 1].trim(); t.is_empty() || t.starts_with(';') || t.starts_with('#') } {
                    end -= 1;
                }
                self.lines.insert(end, format!("{key} = {value}"));
            }
            None => {
                if self.lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    self.lines.push(String::new());
                }
                self.lines.push(format!("[{section}]"));
                self.lines.push(format!("{key} = {value}"));
            }
        }
    }

    pub fn set_list(&mut self, section: &str, key: &str, values: &[String]) {
        self.set(section, key, &values.join("|"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "[AdditionalOptions]\nextensions            = gradientBars.js|quickQueue.js\ncustom_apps           = marketplace\n\n[Setting]\ncurrent_theme          = marketplace\ncolor_scheme           = \n\n; DO NOT CHANGE!\n[Backup]\nversion = 1.3.10.2586.g81a118a22460\nwith    = 2.46.7\n";

    #[test]
    fn reads_values_and_lists() {
        let i = Ini::parse(SAMPLE);
        assert_eq!(i.get_list("AdditionalOptions", "extensions"), vec!["gradientBars.js", "quickQueue.js"]);
        assert_eq!(i.get("Setting", "color_scheme"), Some(""));
        assert_eq!(i.get("Backup", "with"), Some("2.46.7"));
        assert_eq!(i.get("Setting", "missing"), None);
    }

    #[test]
    fn set_keeps_layout_and_comments() {
        let mut i = Ini::parse(SAMPLE);
        i.set("Setting", "color_scheme", "Base");
        i.set_list("AdditionalOptions", "extensions", &["a.js".into()]);
        let t = i.to_text();
        assert!(t.contains("color_scheme           = Base"));
        assert!(t.contains("extensions            = a.js\r\n"));
        assert!(t.contains("; DO NOT CHANGE!"));
        assert_eq!(Ini::parse(&t).get("Backup", "version"), Some("1.3.10.2586.g81a118a22460"));
    }

    #[test]
    fn set_adds_missing_keys_and_sections() {
        let mut i = Ini::parse(SAMPLE);
        i.set("Setting", "spotify_launch_flags", "");
        i.set("New", "k", "v");
        let p = Ini::parse(&i.to_text());
        assert_eq!(p.get("Setting", "spotify_launch_flags"), Some(""));
        assert_eq!(p.get("New", "k"), Some("v"));
        // Added inside [Setting], before the blank line and comment that precede [Backup].
        let t = i.to_text();
        assert!(t.find("spotify_launch_flags").unwrap() < t.find("; DO NOT CHANGE!").unwrap());
    }
}
