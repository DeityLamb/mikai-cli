//! Layout-independent hotkeys.
//!
//! With a Cyrillic layout the key labelled `C` types `с`, so a hotkey would
//! silently do nothing. Terminals can report the Latin key through the kitty
//! keyboard protocol, but crossterm does not parse that yet, so keys are mapped
//! back here by their position on a ЙЦУКЕН keyboard (Ukrainian and Russian).
//! Only hotkeys go through this; typed text stays as typed.

use ratatui::crossterm::event::{KeyCode, KeyEvent};

/// Lower-case ЙЦУКЕН letter → the QWERTY key at the same place.
const MAP: &[(char, char)] = &[
    ('й', 'q'), ('ц', 'w'), ('у', 'e'), ('к', 'r'), ('е', 't'), ('н', 'y'), ('г', 'u'), ('ш', 'i'),
    ('щ', 'o'), ('з', 'p'), ('х', '['), ('ї', ']'), ('ъ', ']'),
    ('ф', 'a'), ('і', 's'), ('ы', 's'), ('в', 'd'), ('а', 'f'), ('п', 'g'), ('р', 'h'), ('о', 'j'),
    ('л', 'k'), ('д', 'l'), ('ж', ';'), ('є', '\''), ('э', '\''),
    ('я', 'z'), ('ч', 'x'), ('с', 'c'), ('м', 'v'), ('и', 'b'), ('т', 'n'), ('ь', 'm'), ('б', ','),
    ('ю', '.'), ('ґ', '`'), ('ё', '`'),
];

/// The Latin key for a Cyrillic letter, keeping case. Other chars pass through.
pub fn latin(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match MAP.iter().find(|(k, _)| *k == lower) {
        Some(&(_, l)) if c.is_uppercase() => l.to_ascii_uppercase(),
        Some(&(_, l)) => l,
        None => c,
    }
}

/// The same key event with a Cyrillic letter turned into its Latin key.
pub fn hotkey(mut k: KeyEvent) -> KeyEvent {
    if let KeyCode::Char(c) = k.code {
        k.code = KeyCode::Char(latin(c));
    }
    k
}

/// A command name typed in the wrong layout: `.ыуфкср` → `/search`.
/// On a Ukrainian keyboard the `/` key types `.`.
pub fn command_word(word: &str) -> String {
    word.chars().map(latin).collect()
}
