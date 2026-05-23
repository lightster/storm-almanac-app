//! In-memory cache of the hero-name catalog, warmed at app start and
//! used by the draft pipeline. Concurrent callers see at most one
//! in-flight fetch.

use crate::overlay_api;
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Default, Clone)]
pub struct HeroCatalog {
    inner: Arc<Mutex<Option<Vec<String>>>>,
}

impl HeroCatalog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Return the cached hero list, fetching it on a miss. If a fetch
    /// is already in flight on another task, the second caller waits
    /// on the mutex and then sees the cache populated.
    pub async fn ensure(&self) -> Result<Vec<String>, String> {
        let mut guard = self.inner.lock().await;
        if let Some(heroes) = guard.as_ref() {
            return Ok(heroes.clone());
        }
        let heroes = overlay_api::fetch_overlay_heroes().await?;
        *guard = Some(heroes.clone());
        Ok(heroes)
    }
}

/// Type alias for the Tauri-managed catalog.
pub type SharedHeroCatalog = HeroCatalog;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn ensure_returns_cached_when_seeded() {
        // We can't easily test the fetch path without a live server, so
        // verify the cache-hit path by pre-populating the inner mutex.
        let cat = HeroCatalog::new();
        *cat.inner.lock().await = Some(vec!["Abathur".to_string(), "Genji".to_string()]);
        let heroes = cat.ensure().await.unwrap();
        assert_eq!(heroes, vec!["Abathur".to_string(), "Genji".to_string()]);
    }
}
