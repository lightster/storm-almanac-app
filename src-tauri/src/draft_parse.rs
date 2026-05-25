//! Pure parsing logic: OCR lines -> structured draft.

use crate::draft_types::{Draft, DraftHero, DraftPlayer, OcrLine};

/// Normalize a name for comparison: lowercase, keep only ASCII alphanumerics.
fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// Levenshtein edit distance between two strings.
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Match an OCR'd hero label against the canonical hero list.
/// Returns the canonical hero name, or `None` if nothing matches well enough.
pub fn match_hero(ocr_text: &str, hero_list: &[String]) -> Option<String> {
    let target = normalize(ocr_text);
    if target.is_empty() {
        return None;
    }
    let normalized: Vec<String> = hero_list.iter().map(|h| normalize(h)).collect();
    // Exact normalized match.
    if let Some(i) = normalized.iter().position(|n| *n == target) {
        return Some(hero_list[i].clone());
    }
    // Closest hero within an edit distance of 1 (single OCR error).
    hero_list
        .iter()
        .zip(normalized.iter())
        .filter_map(|(h, n)| {
            let d = levenshtein(n, &target);
            if d <= 1 {
                Some((d, h))
            } else {
                None
            }
        })
        .min_by_key(|(d, _)| *d)
        .map(|(_, h)| h.clone())
}

/// Maximum horizontal gap (px) between hero labels considered the same column.
const COLUMN_GAP: f64 = 140.0;

/// Whether OCR output looks like the ARAM "CHOOSE A HERO" draft screen.
pub fn looks_like_draft(lines: &[OcrLine]) -> bool {
    lines
        .iter()
        .any(|l| l.text.to_uppercase().contains("CHOOSE A HERO"))
}

/// Parse OCR lines into a structured draft.
pub fn parse_draft(lines: &[OcrLine], hero_list: &[String]) -> Draft {
    struct HeroLabel<'a> {
        hero: String,
        line: &'a OcrLine,
    }
    #[cfg(debug_assertions)]
    log::info!(
        "draft_parse: matching {} OCR lines against {} catalog heroes",
        lines.len(),
        hero_list.len()
    );
    let mut hero_labels: Vec<HeroLabel> = Vec::new();
    for l in lines {
        let m = match_hero(&l.text, hero_list);
        #[cfg(debug_assertions)]
        log::info!(
            "draft_parse OCR line: text={:?} rect=({:.0},{:.0},{:.0},{:.0}) hero_match={:?}",
            l.text,
            l.rect.x,
            l.rect.y,
            l.rect.width,
            l.rect.height,
            m
        );
        if let Some(hero) = m {
            hero_labels.push(HeroLabel { hero, line: l });
        }
    }
    if hero_labels.is_empty() {
        return Draft { players: vec![] };
    }

    // Screen coordinates are always finite, so partial_cmp never returns None.
    hero_labels.sort_by(|a, b| {
        a.line.rect.center_x().partial_cmp(&b.line.rect.center_x()).unwrap()
    });
    let mut columns: Vec<Vec<&HeroLabel>> = vec![vec![&hero_labels[0]]];
    for label in &hero_labels[1..] {
        let col = columns.last_mut().unwrap();
        // Compare to the column's anchor (first) hero, not the previous one,
        // so a column can't "creep" wider than COLUMN_GAP via small steps.
        let anchor_x = col[0].line.rect.center_x();
        if (label.line.rect.center_x() - anchor_x).abs() <= COLUMN_GAP {
            col.push(label);
        } else {
            columns.push(vec![label]);
        }
    }

    let mut players: Vec<DraftPlayer> = Vec::new();
    for col in columns {
        let col_x = col.iter().map(|l| l.line.rect.center_x()).sum::<f64>() / col.len() as f64;
        let top_hero_y = col
            .iter()
            .map(|l| l.line.rect.y)
            .fold(f64::INFINITY, f64::min);

        let name_line = lines
            .iter()
            .filter(|l| match_hero(&l.text, hero_list).is_none())
            .filter(|l| l.rect.y < top_hero_y)
            .filter(|l| (l.rect.center_x() - col_x).abs() <= COLUMN_GAP)
            .min_by(|a, b| {
                (top_hero_y - a.rect.y)
                    .partial_cmp(&(top_hero_y - b.rect.y))
                    .unwrap()
            });

        let (name, name_rect) = match name_line {
            Some(l) => (l.text.clone(), l.rect),
            None => continue,
        };

        let mut heroes: Vec<DraftHero> = col
            .iter()
            .map(|l| DraftHero { hero: l.hero.clone(), rect: l.line.rect })
            .collect();
        heroes.sort_by(|a, b| a.rect.y.partial_cmp(&b.rect.y).unwrap());

        players.push(DraftPlayer { name, name_rect, heroes, is_self: false });
    }

    players.sort_by(|a, b| a.name_rect.center_x().partial_cmp(&b.name_rect.center_x()).unwrap());
    if !players.is_empty() {
        let mid = players.len() / 2;
        players[mid].is_self = true;
    }
    Draft { players }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draft_types::Rect;

    fn heroes() -> Vec<String> {
        ["Nazeebo", "Li-Ming", "The Butcher", "D.Va", "Genji", "Nova"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn exact_match() {
        assert_eq!(match_hero("Genji", &heroes()), Some("Genji".to_string()));
    }

    #[test]
    fn punctuation_insensitive() {
        assert_eq!(match_hero("LiMing", &heroes()), Some("Li-Ming".to_string()));
        assert_eq!(match_hero("DVa", &heroes()), Some("D.Va".to_string()));
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(match_hero("NAZEEBO", &heroes()), Some("Nazeebo".to_string()));
    }

    #[test]
    fn single_char_ocr_error() {
        // OCR misread 'b' as 'h'.
        assert_eq!(match_hero("Nazeeho", &heroes()), Some("Nazeebo".to_string()));
    }

    #[test]
    fn no_match_returns_none() {
        assert_eq!(match_hero("Xyzzy", &heroes()), None);
        assert_eq!(match_hero("", &heroes()), None);
    }

    #[test]
    fn whitespace_only_returns_none() {
        assert_eq!(match_hero("   ", &heroes()), None);
    }

    #[test]
    fn empty_hero_list_returns_none() {
        assert_eq!(match_hero("Genji", &[]), None);
    }

    fn line(text: &str, x: f64, y: f64) -> OcrLine {
        OcrLine {
            text: text.to_string(),
            rect: Rect { x, y, width: 80.0, height: 18.0 },
        }
    }

    #[test]
    fn parses_two_columns_with_names_and_heroes() {
        let lines = vec![
            line("Togo99", 100.0, 200.0),
            line("Nazeebo", 100.0, 260.0),
            line("Genji", 100.0, 320.0),
            line("Nova", 100.0, 380.0),
            line("Serverus", 400.0, 200.0),
            line("Li-Ming", 400.0, 260.0),
            line("D.Va", 400.0, 320.0),
            line("The Butcher", 400.0, 380.0),
            line("CHOOSE A HERO", 250.0, 50.0),
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players.len(), 2);
        assert_eq!(draft.players[0].name, "Togo99");
        assert_eq!(draft.players[0].heroes.len(), 3);
        assert_eq!(draft.players[0].heroes[0].hero, "Nazeebo");
        assert_eq!(draft.players[1].name, "Serverus");
        assert_eq!(draft.players[1].heroes[2].hero, "The Butcher");
    }

    #[test]
    fn columns_sorted_left_to_right() {
        let lines = vec![
            line("Right", 500.0, 200.0),
            line("Genji", 500.0, 260.0),
            line("Left", 100.0, 200.0),
            line("Nova", 100.0, 260.0),
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players[0].name, "Left");
        assert_eq!(draft.players[1].name, "Right");
    }

    #[test]
    fn empty_input_yields_no_players() {
        assert_eq!(parse_draft(&[], &heroes()).players.len(), 0);
    }

    #[test]
    fn column_without_a_name_line_is_dropped() {
        // The x=400 column has a hero but no name line above it.
        let lines = vec![
            line("Togo99", 100.0, 200.0),
            line("Nova", 100.0, 260.0),
            line("Genji", 400.0, 260.0),
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players.len(), 1);
        assert_eq!(draft.players[0].name, "Togo99");
    }

    #[test]
    fn column_with_fewer_than_three_heroes_is_kept() {
        let lines = vec![
            line("Togo99", 100.0, 200.0),
            line("Nova", 100.0, 260.0),
            line("Genji", 100.0, 320.0),
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players.len(), 1);
        assert_eq!(draft.players[0].heroes.len(), 2);
    }

    #[test]
    fn looks_like_draft_detects_the_header() {
        let lines = vec![line("CHOOSE A HERO", 100.0, 50.0)];
        assert!(looks_like_draft(&lines));
    }

    #[test]
    fn looks_like_draft_is_case_insensitive() {
        let lines = vec![line("Choose a Hero", 100.0, 50.0)];
        assert!(looks_like_draft(&lines));
    }

    #[test]
    fn looks_like_draft_false_without_the_header() {
        let lines = vec![line("Nova", 100.0, 50.0)];
        assert!(!looks_like_draft(&lines));
    }

    #[test]
    fn middle_column_is_marked_self() {
        let lines = vec![
            line("Left", 100.0, 200.0),
            line("Nova", 100.0, 260.0),
            line("Mid", 400.0, 200.0),
            line("Genji", 400.0, 260.0),
            line("Right", 700.0, 200.0),
            line("Nazeebo", 700.0, 260.0),
        ];
        let draft = parse_draft(&lines, &heroes());
        assert_eq!(draft.players.len(), 3);
        assert!(!draft.players[0].is_self);
        assert!(draft.players[1].is_self);
        assert!(!draft.players[2].is_self);
    }
}
