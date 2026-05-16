//! Win-rate API client for the ARAM draft overlay.

use serde::Deserialize;
use std::collections::HashMap;

/// Raw shape of `GET /api/draft`.
#[derive(Debug, Deserialize)]
pub struct DraftApiResponse {
    pub heroes: Vec<HeroEntry>,
    #[serde(rename = "globalStats")]
    pub global_stats: Vec<GlobalStat>,
    #[serde(rename = "playerStats")]
    pub player_stats: Vec<PlayerStat>,
}

#[derive(Debug, Deserialize)]
pub struct HeroEntry {
    pub hero: String,
}

#[derive(Debug, Deserialize)]
pub struct GlobalStat {
    pub hero: String,
    pub global_win_rate: f64,
}

#[derive(Debug, Deserialize)]
pub struct PlayerStat {
    pub hero: String,
    pub win_rate: f64,
    pub games: u32,
}

/// Indexed win rates derived from a `DraftApiResponse`.
pub struct WinRateTable {
    pub hero_names: Vec<String>,
    pub overall: HashMap<String, f64>,
    pub player: HashMap<String, (f64, u32)>,
}

/// Build lookup tables from a raw API response.
pub fn build_table(resp: &DraftApiResponse) -> WinRateTable {
    WinRateTable {
        hero_names: resp.heroes.iter().map(|h| h.hero.clone()).collect(),
        overall: resp
            .global_stats
            .iter()
            .map(|g| (g.hero.clone(), g.global_win_rate))
            .collect(),
        player: resp
            .player_stats
            .iter()
            .map(|p| (p.hero.clone(), (p.win_rate, p.games)))
            .collect(),
    }
}

/// The name part of a battletag, before the `#` discriminator.
pub fn battletag_name_part(battletag: &str) -> &str {
    battletag.split('#').next().unwrap_or(battletag)
}

/// From a `/api/players/search` result list, pick the unique battletag whose
/// name-part exactly (case-insensitively) equals `ocr_name`. `None` if zero or
/// more than one matches.
pub fn resolve_unique(ocr_name: &str, candidates: &[String]) -> Option<String> {
    let matches: Vec<&String> = candidates
        .iter()
        .filter(|bt| battletag_name_part(bt).eq_ignore_ascii_case(ocr_name))
        .collect();
    if matches.len() == 1 {
        Some(matches[0].clone())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_indexes_response() {
        let json = r#"{
            "heroes": [{"hero": "Nazeebo"}, {"hero": "Genji"}],
            "globalStats": [{"hero": "Nazeebo", "global_win_rate": 52.3}],
            "playerStats": [{"hero": "Nazeebo", "win_rate": 61.0, "games": 12}]
        }"#;
        let resp: DraftApiResponse = serde_json::from_str(json).unwrap();
        let table = build_table(&resp);
        assert_eq!(table.hero_names, vec!["Nazeebo", "Genji"]);
        assert_eq!(table.overall.get("Nazeebo"), Some(&52.3));
        assert_eq!(table.player.get("Nazeebo"), Some(&(61.0, 12)));
        assert_eq!(table.player.get("Genji"), None);
    }

    #[test]
    fn battletag_name_part_splits_on_hash() {
        assert_eq!(battletag_name_part("Togo99#1234"), "Togo99");
        assert_eq!(battletag_name_part("NoDiscriminator"), "NoDiscriminator");
    }

    #[test]
    fn resolve_unique_requires_exactly_one() {
        let one = vec!["Togo99#1234".to_string()];
        assert_eq!(resolve_unique("togo99", &one), Some("Togo99#1234".to_string()));

        let ambiguous = vec!["Togo99#1234".to_string(), "Togo99#5678".to_string()];
        assert_eq!(resolve_unique("Togo99", &ambiguous), None);

        let substring_noise = vec!["NotTogo99#1".to_string()];
        assert_eq!(resolve_unique("Togo99", &substring_noise), None);

        assert_eq!(resolve_unique("Togo99", &[]), None);
    }
}

/// Fetch `/api/draft` for an optional player battletag. Pass `None` to fetch
/// just the hero list + global stats (no player param).
pub async fn fetch_draft(player: Option<&str>) -> Result<DraftApiResponse, String> {
    let url = format!("{}/api/draft", crate::API_URL);
    let mut req = reqwest::Client::new().get(&url);
    if let Some(bt) = player {
        req = req.query(&[("player", bt)]);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("draft request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("draft request HTTP {}", resp.status()));
    }
    resp.json::<DraftApiResponse>()
        .await
        .map_err(|e| format!("draft response parse failed: {e}"))
}

/// Search for battletags by plain name via `/api/players/search`.
pub async fn search_players(name: &str) -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct SearchRow {
        battletag: String,
    }
    let url = format!("{}/api/players/search", crate::API_URL);
    let resp = reqwest::Client::new()
        .get(&url)
        .query(&[("q", name)])
        .send()
        .await
        .map_err(|e| format!("player search failed: {e}"))?;
    let rows: Vec<SearchRow> = resp
        .json()
        .await
        .map_err(|e| format!("player search parse failed: {e}"))?;
    Ok(rows.into_iter().map(|r| r.battletag).collect())
}
