//! Client for the overlay batch endpoints on hotsds.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// One slot in a batch draft request. Each slot has its `heroes` and at
/// most one of `battletag` or `name`. The server returns `null` for a
/// slot that supplied no identifier or whose `name` did not resolve to a
/// unique battletag.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverlayDraftPlayer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub battletag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub heroes: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OverlayDraftRequest {
    pub players: Vec<OverlayDraftPlayer>,
}

/// One hero's per-player stats. Returned by the server as either a
/// `{win_rate, games}` object or `null`.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize)]
pub struct OverlayPlayerHero {
    pub win_rate: f64,
    pub games: u32,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct OverlayDraftResponse {
    pub overall: HashMap<String, f64>,
    /// Position-aligned with `OverlayDraftRequest.players`. A `None`
    /// element means the server could not produce stats for that slot.
    /// Inner `None` means the player has no games on that hero.
    pub players: Vec<Option<HashMap<String, Option<OverlayPlayerHero>>>>,
}

/// Fetch the hero-name catalog from `GET /api/overlay/heroes`.
pub async fn fetch_overlay_heroes() -> Result<Vec<String>, String> {
    #[derive(Deserialize)]
    struct Resp {
        heroes: Vec<String>,
    }
    let url = format!("{}/api/overlay/heroes", crate::API_URL);
    let resp = reqwest::Client::new()
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("overlay heroes request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("overlay heroes HTTP {}", resp.status()));
    }
    let parsed: Resp = resp
        .json()
        .await
        .map_err(|e| format!("overlay heroes parse failed: {e}"))?;
    Ok(parsed.heroes)
}

/// POST a batch draft request to `/api/overlay/draft`.
pub async fn fetch_overlay_draft(
    req: &OverlayDraftRequest,
) -> Result<OverlayDraftResponse, String> {
    let url = format!("{}/api/overlay/draft", crate::API_URL);
    let resp = reqwest::Client::new()
        .post(&url)
        .json(req)
        .send()
        .await
        .map_err(|e| format!("overlay draft request failed: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("overlay draft HTTP {}", resp.status()));
    }
    resp.json::<OverlayDraftResponse>()
        .await
        .map_err(|e| format!("overlay draft parse failed: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_request_omitting_unused_identifier_fields() {
        let req = OverlayDraftRequest {
            players: vec![
                OverlayDraftPlayer {
                    battletag: Some("Lightster#1234".into()),
                    name: None,
                    heroes: vec!["Stitches".into()],
                },
                OverlayDraftPlayer {
                    battletag: None,
                    name: Some("zulu".into()),
                    heroes: vec!["Genji".into()],
                },
                OverlayDraftPlayer {
                    battletag: None,
                    name: None,
                    heroes: vec!["Nova".into()],
                },
            ],
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["players"][0]["battletag"], "Lightster#1234");
        assert!(v["players"][0].get("name").is_none());
        assert_eq!(v["players"][1]["name"], "zulu");
        assert!(v["players"][1].get("battletag").is_none());
        assert!(v["players"][2].get("battletag").is_none());
        assert!(v["players"][2].get("name").is_none());
        assert_eq!(v["players"][2]["heroes"][0], "Nova");
    }

    #[test]
    fn parses_response_with_mixed_player_slots() {
        let json = r#"{
            "overall": { "Stitches": 51.2, "Genji": 49.0 },
            "players": [
                { "Stitches": { "win_rate": 60.0, "games": 12 }, "Genji": null },
                null,
                { "Genji": { "win_rate": 55.0, "games": 4 } }
            ]
        }"#;
        let resp: OverlayDraftResponse = serde_json::from_str(json).unwrap();
        assert_eq!(resp.overall.get("Stitches"), Some(&51.2));
        assert_eq!(
            resp.players[0].as_ref().unwrap().get("Stitches"),
            Some(&Some(OverlayPlayerHero { win_rate: 60.0, games: 12 }))
        );
        assert_eq!(resp.players[0].as_ref().unwrap().get("Genji"), Some(&None));
        assert!(resp.players[1].is_none());
        assert_eq!(
            resp.players[2].as_ref().unwrap().get("Genji"),
            Some(&Some(OverlayPlayerHero { win_rate: 55.0, games: 4 }))
        );
    }
}
