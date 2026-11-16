//! Spell check for the input line, with Hunspell dictionaries: the system's, or
//! ones downloaded from LibreOffice's collection into the app's settings folder.

use std::path::PathBuf;

pub struct SpellChecker {
    dict: spellbook::Dictionary,
}

impl SpellChecker {
    /// Parsing a dictionary takes a moment, so this runs off the UI thread.
    pub fn load(lang: &str) -> Result<Self, String> {
        let (aff, dic) = find_dict_files(lang).ok_or_else(|| format!("No {lang} dictionary"))?;
        let aff = std::fs::read(aff).map_err(|e| e.to_string())?;
        let dic = std::fs::read(dic).map_err(|e| e.to_string())?;
        // Older dictionaries (German, Polish) aren't UTF-8; their SET line names the encoding.
        let encoding = aff.split(|&b| b == b'\n')
            .find_map(|line| line.strip_prefix(b"SET "))
            .and_then(|label| encoding_rs::Encoding::for_label(label.trim_ascii()))
            .unwrap_or(encoding_rs::UTF_8);
        let (aff, dic) = (encoding.decode(&aff).0, encoding.decode(&dic).0);
        let dict = spellbook::Dictionary::new(&aff, &dic).map_err(|e| format!("{lang}: {e}"))?;
        Ok(Self { dict })
    }

    pub fn check(&self, word: &str) -> bool {
        self.dict.check(word)
    }
}

/// The installed dictionaries, by language code (`en_US`).
pub fn available_languages() -> Vec<String> {
    let mut langs: Vec<String> = search_dirs().iter()
        .filter_map(|dir| std::fs::read_dir(dir).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            (path.extension()? == "dic").then_some(())?;
            let stem = path.file_stem()?.to_str()?.to_string();
            path.with_extension("aff").exists().then_some(stem)
        })
        .collect();
    langs.sort();
    langs.dedup();
    langs
}

/// The words of `text` to check, as byte ranges. Commands aren't checked, nor URLs,
/// channels, nicks the caller knows of, or words with digits.
pub fn misspelled(checkers: &[&SpellChecker], text: &str, nicks: &[String]) -> Vec<(usize, usize)> {
    if checkers.is_empty() || text.starts_with('/') {
        return Vec::new();
    }
    let mut ranges = Vec::new();
    let mut offset = 0;
    for token in text.split_inclusive(char::is_whitespace) {
        let start = offset;
        offset += token.len();
        let token = token.trim_end();
        if token.contains("://") || token.starts_with(['#', '@', ':']) || token.chars().any(|c| c.is_ascii_digit()) {
            continue;
        }
        let bare = token.trim_end_matches([':', ',']);
        if nicks.iter().any(|n| n.eq_ignore_ascii_case(bare)) {
            continue;
        }
        for (at, word) in words(token) {
            if word.chars().count() > 1 && !checkers.iter().any(|c| c.check(word)) {
                ranges.push((start + at, start + at + word.len()));
            }
        }
    }
    ranges
}

/// Runs of letters, with apostrophes inside them ("don't").
fn words(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in text.char_indices() {
        if c.is_alphabetic() || (c == '\'' && start.is_some()) {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push((s, text[s..i].trim_end_matches('\'')));
        }
    }
    if let Some(s) = start {
        out.push((s, text[s..].trim_end_matches('\'')));
    }
    out
}

const LO_BASE: &str = "https://raw.githubusercontent.com/LibreOffice/dictionaries/master";

/// A dictionary that can be downloaded, from LibreOffice's collection.
pub struct DictEntry {
    pub name: &'static str,
    pub code: &'static str,
    path: &'static str,
}

pub const CATALOGUE: &[DictEntry] = &[
    DictEntry { name: "English (US)", code: "en_US", path: "en/en_US" },
    DictEntry { name: "English (GB)", code: "en_GB", path: "en/en_GB" },
    DictEntry { name: "German", code: "de_DE", path: "de/de_DE_frami" },
    DictEntry { name: "French", code: "fr_FR", path: "fr_FR/dictionaries/fr" },
    DictEntry { name: "Spanish", code: "es_ES", path: "es/es_ES" },
    DictEntry { name: "Swedish", code: "sv_SE", path: "sv_SE/dictionaries/sv_SE" },
    DictEntry { name: "Dutch", code: "nl_NL", path: "nl_NL/nl_NL" },
    DictEntry { name: "Italian", code: "it_IT", path: "it_IT/it_IT" },
    DictEntry { name: "Portuguese (Brazil)", code: "pt_BR", path: "pt_BR/pt_BR" },
    DictEntry { name: "Portuguese (Portugal)", code: "pt_PT", path: "pt_PT/pt_PT" },
    DictEntry { name: "Russian", code: "ru_RU", path: "ru_RU/ru_RU" },
    DictEntry { name: "Polish", code: "pl_PL", path: "pl_PL/pl_PL" },
    DictEntry { name: "Norwegian (Bokmål)", code: "nb_NO", path: "no/nb_NO" },
];

/// The name of a language code from the catalogue, or the code itself.
pub fn language_name(code: &str) -> &str {
    CATALOGUE.iter().find(|d| d.code == code).map_or(code, |d| d.name)
}

/// Downloads `code`'s dictionary into the app's folder.
pub async fn download(code: &str) -> Result<(), String> {
    let entry = CATALOGUE.iter().find(|d| d.code == code).ok_or("Unknown dictionary")?;
    let dir = app_dict_dir().ok_or("No folder for dictionaries")?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // A stalled download ends, and nothing far larger than a dictionary is kept.
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(15))
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| format!("Download failed: {e}"))?;
    const MAX_BYTES: usize = 64 * 1024 * 1024;
    for ext in ["aff", "dic"] {
        let url = format!("{LO_BASE}/{}.{ext}", entry.path);
        let mut resp = client.get(&url).send().await
            .and_then(|r| r.error_for_status())
            .map_err(|e| format!("Download failed: {e}"))?;
        let mut bytes = Vec::new();
        while let Some(chunk) = resp.chunk().await.map_err(|e| format!("Download failed: {e}"))? {
            if bytes.len() + chunk.len() > MAX_BYTES {
                return Err("Download failed: the file is too large to be a dictionary".to_string());
            }
            bytes.extend_from_slice(&chunk);
        }
        std::fs::write(dir.join(format!("{code}.{ext}")), &bytes).map_err(|e| format!("Couldn't save it: {e}"))?;
    }
    Ok(())
}

/// Downloaded dictionaries, next to the app's settings.
fn app_dict_dir() -> Option<PathBuf> {
    eframe::storage_dir(&crate::app_id()).map(|dir| dir.join("dictionaries"))
}

fn search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = app_dict_dir().into_iter().collect();
    dirs.extend([
        "/usr/share/hunspell",
        "/usr/share/myspell/dicts",
        "/usr/share/myspell",
        "/opt/homebrew/share/hunspell",
        "/usr/local/share/hunspell",
        "/Library/Spelling",
    ].map(PathBuf::from));
    if let Some(home) = std::env::var_os("HOME") {
        dirs.push(PathBuf::from(home).join("Library/Spelling"));
    }
    dirs
}

fn find_dict_files(lang: &str) -> Option<(PathBuf, PathBuf)> {
    search_dirs().into_iter()
        .map(|dir| (dir.join(format!("{lang}.aff")), dir.join(format!("{lang}.dic"))))
        .find(|(aff, dic)| aff.exists() && dic.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_words_and_skips_what_isnt_prose() {
        assert_eq!(words("don't stop, it's 'fine'"), [(0, "don't"), (6, "stop"), (12, "it's"), (18, "fine")]);
        let aff = "SET UTF-8\n";
        let dic = "3\nhello\nworld\nthere\n";
        let checker = SpellChecker { dict: spellbook::Dictionary::new(aff, dic).unwrap() };
        let text = "hello wrold bob: https://x.io/wrold #wrold there";
        let bad = misspelled(&[&checker], text, &["Bob".into()]);
        assert_eq!(bad.iter().map(|&(s, e)| &text[s..e]).collect::<Vec<_>>(), ["wrold"]);
        assert!(misspelled(&[&checker], "/msg wrold", &[]).is_empty());
    }
}
