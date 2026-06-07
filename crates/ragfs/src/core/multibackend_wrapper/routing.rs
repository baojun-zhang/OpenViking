use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Value};

use super::*;
use crate::multibackend::meta::{current_required_ctx, file_name, parent_dir};

impl Inner {
    /// Invalidate one cached read-route entry after a write-side state change.
    pub(super) async fn invalidate_read_route(&self, path: &str) {
        self.read_route_cache.lock().await.remove(path);
    }

    /// Cache a resolved read route for a short TTL window.
    async fn cache_read_route(&self, path: &str, backend_name: Option<String>) {
        let now = Instant::now();
        let mut cache = self.read_route_cache.lock().await;
        cache.insert(
            path.to_string(),
            ReadRouteCacheEntry {
                backend_name,
                cached_at: now,
            },
        );
        cache.retain(|_, entry| now.duration_since(entry.cached_at) <= self.read_route_cache_ttl);
        while cache.len() > self.read_route_cache_capacity {
            let oldest_key = cache
                .iter()
                .min_by_key(|(_, entry)| entry.cached_at)
                .map(|(key, _)| key.clone());
            if let Some(oldest_key) = oldest_key {
                cache.remove(&oldest_key);
            } else {
                break;
            }
        }
    }

    /// Read and validate a cached route if it is still fresh.
    async fn cached_read_route(&self, path: &str) -> Option<Option<Arc<dyn FileSystem>>> {
        let entry = self.read_route_cache.lock().await.get(path).cloned()?;
        if entry.cached_at.elapsed() > self.read_route_cache_ttl {
            self.read_route_cache.lock().await.remove(path);
            return None;
        }

        match entry.backend_name {
            Some(name) if name == self.primary().name => Some(Some(self.primary().backend.clone())),
            Some(name) => Some(self.backup_by_name(&name).map(|be| be.backend.clone())),
            None => Some(None),
        }
    }

    /// Record read-route counters in one place so hot paths stay explicit.
    fn record_read_route(&self, source: ReadRouteSource) {
        match source {
            ReadRouteSource::Cache => {
                self.read_cache_hits.fetch_add(1, Ordering::Relaxed);
            }
            ReadRouteSource::Backup => {
                self.read_backup_hits.fetch_add(1, Ordering::Relaxed);
            }
            ReadRouteSource::Primary => {
                self.read_primary_hits.fetch_add(1, Ordering::Relaxed);
            }
            ReadRouteSource::Redirect => {
                self.read_redirect_hits.fetch_add(1, Ordering::Relaxed);
            }
            ReadRouteSource::Miss => {
                self.read_misses.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Export read-route metrics for operational introspection.
    pub(crate) fn read_route_metrics(&self) -> Value {
        json!({
            "cache_hits": self.read_cache_hits.load(Ordering::Relaxed),
            "backup_hits": self.read_backup_hits.load(Ordering::Relaxed),
            "primary_hits": self.read_primary_hits.load(Ordering::Relaxed),
            "redirect_hits": self.read_redirect_hits.load(Ordering::Relaxed),
            "misses": self.read_misses.load(Ordering::Relaxed),
        })
    }

    /// Stat the first reachable redirect target and return user-visible metadata.
    pub(super) async fn redirect_file_info(
        &self,
        path: &str,
        name: &str,
        redirect_entry: &RedirectEntry,
    ) -> FileInfo {
        for target_name in &redirect_entry.targets {
            if let Some(be) = self.backup_by_name(target_name) {
                if let Ok(mut info) = be.backend.stat(path).await {
                    info.name = name.to_string();
                    return info;
                }
            }
        }
        FileInfo::new_file(name.to_string(), 0, 0o644)
    }

    /// Resolve the read backend for a path using the fallback chain.
    pub(super) async fn resolve_read_backend(&self, path: &str) -> Option<Arc<dyn FileSystem>> {
        let normalized = normalize_prefix_path(path);
        if let Some(cached) = self.cached_read_route(&normalized).await {
            self.record_read_route(ReadRouteSource::Cache);
            return cached;
        }

        let read_backups = self.read_backups_sorted();
        let backup_exists = futures::future::join_all(read_backups.iter().map(|backup| async {
            (
                backup.name.clone(),
                backup.backend.clone(),
                backup.backend.exists(&normalized).await,
            )
        }))
        .await;
        for (name, backend, exists) in backup_exists {
            if exists {
                self.cache_read_route(&normalized, Some(name)).await;
                self.record_read_route(ReadRouteSource::Backup);
                return Some(backend);
            }
        }

        if self.primary().backend.exists(&normalized).await {
            self.cache_read_route(&normalized, Some(self.primary().name.clone()))
                .await;
            self.record_read_route(ReadRouteSource::Primary);
            return Some(self.primary().backend.clone());
        }

        let dir = parent_dir(&normalized);
        let name = file_name(&normalized).to_string();
        let ctx = current_required_ctx()
            .or_else(|_| self.meta_store.ctx_resolver().resolve(&dir))
            .ok()?;
        if let Ok(redirect_meta) = self.meta_store.get_redirect_meta(&dir, &ctx).await {
            if let Some(entry) = redirect_meta.entries.get(&name) {
                let redirect_targets: Vec<(String, Arc<dyn FileSystem>)> = entry
                    .targets
                    .iter()
                    .filter_map(|target_name| {
                        self.backup_by_name(target_name)
                            .map(|be| (be.name.clone(), be.backend.clone()))
                    })
                    .collect();
                let redirect_exists = futures::future::join_all(redirect_targets.iter().map(
                    |(target_name, backend)| async {
                        (
                            target_name.clone(),
                            backend.clone(),
                            backend.exists(&normalized).await,
                        )
                    },
                ))
                .await;
                for (target_name, backend, exists) in redirect_exists {
                    if exists {
                        self.cache_read_route(&normalized, Some(target_name)).await;
                        self.record_read_route(ReadRouteSource::Redirect);
                        return Some(backend);
                    }
                }
            }
        }

        self.cache_read_route(&normalized, None).await;
        self.record_read_route(ReadRouteSource::Miss);
        None
    }
}
