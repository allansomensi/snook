//! Interface languages: English (the source text), Brazilian Portuguese and
//! Spanish.
//!
//! Texts are written in English in the code and looked up with [`t`]; texts
//! with values use named placeholders, filled in by [`tw`]. The translations
//! live in one file per language (`lang/*.txt`: the English text, a tab, the
//! translation), so adding a language is adding a file, not code.

use std::collections::HashMap;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

use snook_core::{BallId, Difficulty, Foul, GameOverReason, Group, PocketId, ShotError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    English,
    Portuguese,
    Spanish,
}

impl Lang {
    pub const ALL: [Lang; 3] = [Lang::English, Lang::Portuguese, Lang::Spanish];

    /// The language's own name, as shown in the settings.
    pub fn name(self) -> &'static str {
        match self {
            Lang::English => "English",
            Lang::Portuguese => "Português (Brasil)",
            Lang::Spanish => "Español",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Lang::English => "en",
            Lang::Portuguese => "pt-BR",
            Lang::Spanish => "es",
        }
    }

    pub fn parse(s: &str) -> Option<Lang> {
        let s = s.trim().to_ascii_lowercase();
        if s.starts_with("pt") {
            Some(Lang::Portuguese)
        } else if s.starts_with("es") {
            Some(Lang::Spanish)
        } else if s.starts_with("en") {
            Some(Lang::English)
        } else {
            None
        }
    }

    /// The language of the operating system, if we have a translation for it.
    pub fn system() -> Lang {
        sys_locale::get_locale().and_then(|l| Lang::parse(&l)).unwrap_or(Lang::English)
    }

    /// The translation file (none for English, the source).
    fn source(self) -> Option<&'static str> {
        match self {
            Lang::English => None,
            Lang::Portuguese => Some(include_str!("../lang/pt-BR.txt")),
            Lang::Spanish => Some(include_str!("../lang/es.txt")),
        }
    }
}

static LANG: AtomicU8 = AtomicU8::new(0);

pub fn set_lang(lang: Lang) {
    LANG.store(lang as u8, Ordering::Relaxed);
}

pub fn lang() -> Lang {
    Lang::ALL
        .into_iter()
        .find(|l| *l as u8 == LANG.load(Ordering::Relaxed))
        .unwrap_or(Lang::English)
}

type Table = HashMap<&'static str, &'static str>;

/// Reads a translation file: `English<TAB>translation` per line, `#` for
/// comments, `\n` and `\t` for a new line and a tab. A comment has no tab,
/// so texts may start with `#` ("#{n} in the world").
fn parse_table(text: &'static str) -> Table {
    fn unescape(s: &'static str) -> &'static str {
        if !s.contains('\\') {
            return s;
        }
        let mut out = String::with_capacity(s.len());
        let mut chars = s.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                match chars.next() {
                    Some('n') => out.push('\n'),
                    Some('t') => out.push('\t'),
                    Some(other) => out.push(other),
                    None => {}
                }
            } else {
                out.push(c);
            }
        }
        // Once per text with an escape, for the life of the game.
        Box::leak(out.into_boxed_str())
    }
    text.lines()
        .filter(|l| !l.trim().is_empty() && !(l.starts_with('#') && !l.contains('\t')))
        .filter_map(|l| l.split_once('\t'))
        .map(|(en, tr)| (unescape(en), unescape(tr)))
        .collect()
}

fn table(lang: Lang) -> Option<&'static Table> {
    static TABLES: [OnceLock<Table>; 3] = [OnceLock::new(), OnceLock::new(), OnceLock::new()];
    let source = lang.source()?;
    Some(TABLES[lang as usize].get_or_init(|| parse_table(source)))
}

fn translate(lang: Lang, en: &str) -> Option<&'static str> {
    table(lang)?.get(en).copied()
}

/// Translates an interface text (returns it unchanged when there is no translation).
pub fn t(en: &str) -> &str {
    translate(lang(), en).unwrap_or(en)
}

/// Translates a text with `{name}` placeholders and fills them in.
pub fn tw(en: &str, values: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = t(en).to_string();
    for (key, value) in values {
        out = out.replace(&format!("{{{key}}}"), &value.to_string());
    }
    out
}

/// Translates a message that came from the server or the network (exact
/// texts, plus a few with a variable part).
pub fn server_text(text: &str) -> String {
    if lang() == Lang::English {
        return text.to_string();
    }
    if let Some(t) = translate(lang(), text) {
        return t.to_string();
    }
    if let Some(rest) = text.strip_prefix("You are muted for ") {
        let n = rest.split_whitespace().next().unwrap_or("?");
        return tw("You are muted for {n} more minutes.", &[("n", &n)]);
    }
    if let Some(rest) = text.strip_prefix("This account is banned for ") {
        let n = rest.split_whitespace().next().unwrap_or("?");
        let reason = rest.split("Reason: ").nth(1).unwrap_or("").trim_end_matches('.');
        return tw(
            "This account is suspended for {n} more day(s). Reason: {reason}.",
            &[("n", &n), ("reason", &reason)],
        );
    }
    if let Some(rest) = text.strip_prefix("This account is banned. Reason: ") {
        return tw(
            "This account is banned. Reason: {reason}.",
            &[("reason", &rest.trim_end_matches('.'))],
        );
    }
    for (en, key) in [
        ("You were muted for ", "You were muted: {r}"),
        ("Your account was suspended for ", "Your account was suspended: {r}"),
        ("Your account was banned: ", "Your account was banned: {r}"),
        ("You were disconnected by a moderator: ", "You were disconnected by a moderator: {r}"),
    ] {
        if let Some(rest) = text.strip_prefix(en) {
            return tw(key, &[("r", &rest)]);
        }
    }
    if let Some(rest) = text.strip_prefix("Can't shoot: ") {
        let reason = rest.trim_end_matches('.');
        return tw("Can't shoot: {e}.", &[("e", &t(reason))]);
    }
    text.to_string()
}

pub fn difficulty(d: Difficulty) -> &'static str {
    match d {
        Difficulty::Easy => t("Easy"),
        Difficulty::Medium => {
            if lang() == Lang::English {
                "Medium"
            } else {
                t("Normal")
            }
        }
        Difficulty::Hard => t("Hard"),
    }
}

pub fn group(g: Group) -> &'static str {
    t(g.name())
}

pub fn pocket(p: PocketId) -> &'static str {
    t(p.name())
}

pub fn ball(id: BallId) -> String {
    if id == BallId::CUE { t("cue ball").to_string() } else { tw("{n}-ball", &[("n", &id.0)]) }
}

/// How a game ended, naming who did it ("Ana sank the 8 too early").
pub fn result_line(r: GameOverReason, winner: &str, loser: &str) -> String {
    use GameOverReason::*;
    let (key, name) = match r {
        PocketedEight => ("{name} sank the 8-ball", winner),
        EightPocketedEarly => ("{name} sank the 8 too early", loser),
        EightInWrongPocket => ("{name} sank the 8 in the wrong pocket", loser),
        EightPocketedOnFoul => ("{name} fouled on the 8", loser),
        EightOffTable => ("{name} knocked the 8 off the table", loser),
        Resignation => ("{name} resigned", loser),
        Abandoned => ("{name} left the game", loser),
    };
    tw(key, &[("name", &name)])
}

/// How a game in the history ended, from the player's side.
pub fn history_reason(r: GameOverReason, won: bool) -> &'static str {
    use GameOverReason::*;
    t(match (r, won) {
        (PocketedEight, true) => "sank the 8-ball",
        (PocketedEight, false) => "the opponent sank the 8-ball",
        (EightPocketedEarly, true) => "the opponent sank the 8 too early",
        (EightPocketedEarly, false) => "sank the 8 too early",
        (EightInWrongPocket, true) => "the opponent sank the 8 in the wrong pocket",
        (EightInWrongPocket, false) => "sank the 8 in the wrong pocket",
        (EightPocketedOnFoul, true) => "the opponent fouled on the 8",
        (EightPocketedOnFoul, false) => "fouled on the 8",
        (EightOffTable, true) => "the opponent knocked the 8 off the table",
        (EightOffTable, false) => "knocked the 8 off the table",
        (Resignation, true) => "the opponent resigned",
        (Resignation, false) => "resigned",
        (Abandoned, true) => "the opponent left",
        (Abandoned, false) => "left the game",
    })
}

pub fn foul(f: &Foul) -> String {
    match f {
        Foul::CueBallPocketed => t("Scratch: the cue ball was pocketed").into(),
        Foul::CueBallOffTable => t("The cue ball left the table").into(),
        Foul::NoContact => t("The cue ball didn't hit any ball").into(),
        Foul::WrongBallFirst(id) => tw("Wrong ball hit first ({ball})", &[("ball", &ball(*id))]),
        Foul::NoRailAfterContact => t("No ball reached a cushion after contact").into(),
        Foul::ObjectBallOffTable(id) => tw("The {ball} left the table", &[("ball", &ball(*id))]),
        Foul::IllegalBreak => t("Illegal break").into(),
    }
}

pub fn shot_error(e: ShotError) -> String {
    t(&e.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_and_fills_placeholders() {
        set_lang(Lang::Portuguese);
        assert_eq!(t("Play"), "Jogar");
        assert_eq!(t("not in the table"), "not in the table");
        assert_eq!(tw("{name} wins!", &[("name", &"Ana")]), "Ana vence!");
        // A text that starts with `#` is not a comment.
        assert_eq!(tw("#{n} in the world", &[("n", &3)]), "#3 no mundo");
        assert_eq!(ball(BallId(8)), "bola 8");
        assert_eq!(
            server_text("Can't shoot: the game is over."),
            "Não dá para tacar: a partida acabou."
        );
        set_lang(Lang::Spanish);
        assert_eq!(t("Play"), "Jugar");
        set_lang(Lang::English);
        assert_eq!(tw("{name} wins!", &[("name", &"Ana")]), "Ana wins!");
    }

    /// Every translation keeps the placeholders of its English text, and
    /// every language translates the same texts.
    #[test]
    fn translation_files_are_consistent() {
        let placeholders = |s: &str| {
            let mut v: Vec<String> = s
                .split('{')
                .skip(1)
                .filter_map(|p| p.split_once('}').map(|(k, _)| k.to_string()))
                .collect();
            v.sort();
            v
        };
        let pt = table(Lang::Portuguese).unwrap();
        let es = table(Lang::Spanish).unwrap();
        for lang in [Lang::Portuguese, Lang::Spanish] {
            for (en, tr) in table(lang).unwrap() {
                assert_eq!(placeholders(en), placeholders(tr), "{lang:?}: {en} → {tr}");
            }
        }
        let missing: Vec<&&str> = pt.keys().filter(|k| !es.contains_key(*k)).collect();
        assert!(missing.is_empty(), "not in Spanish: {missing:?}");
        let extra: Vec<&&str> = es.keys().filter(|k| !pt.contains_key(*k)).collect();
        assert!(extra.is_empty(), "not in Portuguese: {extra:?}");
    }
}
