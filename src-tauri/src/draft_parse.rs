//! Pure parsing logic: OCR lines -> structured draft.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
