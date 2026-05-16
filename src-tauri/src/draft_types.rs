//! Shared plain data types for the ARAM draft overlay pipeline.

use serde::Serialize;

/// A rectangle in primary-monitor screen pixels.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

impl Rect {
    /// Horizontal centre of the rect.
    pub fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    /// Divide all coordinates by `factor` — converts physical pixels to the
    /// logical (CSS) pixels the overlay window renders in.
    pub fn descale(&self, factor: f64) -> Rect {
        Rect {
            x: self.x / factor,
            y: self.y / factor,
            width: self.width / factor,
            height: self.height / factor,
        }
    }
}

/// One line of recognized text with its on-screen bounding box.
#[derive(Debug, Clone, PartialEq)]
pub struct OcrLine {
    pub text: String,
    pub rect: Rect,
}

/// A hero offered to a player, with the bounding box of its name label.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftHero {
    /// Canonical hero name (e.g. "Li-Ming").
    pub hero: String,
    pub rect: Rect,
}

/// One player column in the draft: a name and (up to) 3 hero choices.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftPlayer {
    /// Player name as OCR'd from the screen (no battletag discriminator).
    pub name: String,
    pub name_rect: Rect,
    pub heroes: Vec<DraftHero>,
}

/// The parsed ARAM draft screen.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Draft {
    pub players: Vec<DraftPlayer>,
}

/// Win rates for a single hero (overall ARAM + a specific player's).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HeroWinRates {
    pub overall: Option<f64>,
    pub player: Option<f64>,
    pub player_games: Option<u32>,
}

/// A hero ready to draw: its box plus its win rates.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftOverlayHero {
    pub hero: String,
    pub rect: Rect,
    pub win_rates: HeroWinRates,
}

/// The full payload emitted to the overlay window.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DraftOverlayPayload {
    pub heroes: Vec<DraftOverlayHero>,
}
