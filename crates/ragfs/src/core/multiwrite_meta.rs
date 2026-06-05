//! Multi-write metadata management.
//!
//! Provides `MetaStateStore` for serialized read-modify-write of `.redirect.json` and
//! `.sync_log.json` through `primary_backend`, and `FsContextResolver` for recovering
//! `FsContext` from paths in background tasks.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

use super::context::{FsContext, FsContextInner, FS_CTX};
use super::errors::{Error, Result};
use super::filesystem::FileSystem;
use super::types::{RedirectMeta, SyncLogMeta, WriteFlag};

/// Trait for resolving `FsContext` from a filesystem path.
///
/// Used by background tasks (retry_loop, backfill, system_sync_retry) that lack a
/// foreground request context. Implementations extract `account_id` from the path
/// (e.g. `/local/{account_id}/...`).
pub trait FsContextResolver: Send + Sync {
    /// Recover `FsContext` from a normalized path.
    /// Returns an error if the path cannot be resolved to a valid context.
    fn resolve(&self, path: &str) -> Result<FsContext>;
}

/// Default resolver that extracts `account_id` from `/local/{account_id}/...` paths.
pub struct DefaultFsContextResolver;

impl FsContextResolver for DefaultFsContextResolver {
    fn resolve(&self, path: &str) -> Result<FsContext> {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        // Path format: /local/{account_id}/...
        if parts.len() >= 2 && parts[0] == "local" && !parts[1].is_empty() {
            Ok(Arc::new(FsContextInner::new(parts[1].to_string())))
        } else {
            Err(Error::internal(format!(
                "cannot resolve FsContext from path: {}",
                path
            )))
        }
    }
}

/// Internal file names for metadata files.
const REDIRECT_FILE: &str = ".redirect.json";
const SYNC_LOG_FILE: &str = ".sync_log.json";

/// Unified metadata store for `.redirect.json` and `.sync_log.json`.
///
/// All reads and writes go through `primary_backend`, inheriting its encryption
/// configuration. Directory-level locks ensure serialized access to both metadata
/// files within the same directory.
pub struct MetaStateStore {
    /// Primary backend (may be encrypted)
    primary_backend: Arc<dyn FileSystem>,
    /// Per-directory locks for serialized read-modify-write
    dir_locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    /// Context resolver for background tasks
    ctx_resolver: Arc<dyn FsContextResolver>,
}

impl MetaStateStore {
    /// Create a new MetaStateStore.
    pub fn new(
        primary_backend: Arc<dyn FileSystem>,
        ctx_resolver: Arc<dyn FsContextResolver>,
    ) -> Self {
        Self {
            primary_backend,
            dir_locks: Mutex::new(HashMap::new()),
            ctx_resolver,
        }
    }

    /// Get or create a per-directory lock.
    async fn get_dir_lock(&self, dir: &str) -> Arc<Mutex<()>> {
        let mut locks = self.dir_locks.lock().await;
        locks
            .entry(dir.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    /// Build the full path for a metadata file in a directory.
    fn meta_path(dir: &str, filename: &str) -> String {
        if dir == "/" {
            format!("/{}", filename)
        } else {
            format!("{}/{}", dir, filename)
        }
    }

    /// Read redirect metadata from a directory (returns default if not found).
    async fn read_redirect_meta(&self, dir: &str, ctx: &FsContext) -> Result<RedirectMeta> {
        let path = Self::meta_path(dir, REDIRECT_FILE);
        match FS_CTX.scope(ctx.clone(), async {
            self.primary_backend.read(&path, 0, 0).await
        }).await {
            Ok(data) => {
                if data.is_empty() {
                    Ok(RedirectMeta::default())
                } else {
                    Ok(serde_json::from_slice(&data).unwrap_or_default())
                }
            }
            Err(Error::NotFound(_)) => Ok(RedirectMeta::default()),
            Err(e) => Err(e),
        }
    }

    /// Read sync log metadata from a directory (returns default if not found).
    async fn read_sync_log_meta(&self, dir: &str, ctx: &FsContext) -> Result<SyncLogMeta> {
        let path = Self::meta_path(dir, SYNC_LOG_FILE);
        match FS_CTX.scope(ctx.clone(), async {
            self.primary_backend.read(&path, 0, 0).await
        }).await {
            Ok(data) => {
                if data.is_empty() {
                    Ok(SyncLogMeta::default())
                } else {
                    Ok(serde_json::from_slice(&data).unwrap_or_default())
                }
            }
            Err(Error::NotFound(_)) => Ok(SyncLogMeta::default()),
            Err(e) => Err(e),
        }
    }

    /// Write redirect metadata to a directory.
    async fn write_redirect_meta(&self, dir: &str, meta: &RedirectMeta, ctx: &FsContext) -> Result<()> {
        let path = Self::meta_path(dir, REDIRECT_FILE);
        let data = serde_json::to_vec(meta)?;
        FS_CTX.scope(ctx.clone(), async {
            self.primary_backend.write(&path, &data, 0, WriteFlag::Create).await.map(|_| ())
        }).await
    }

    /// Write sync log metadata to a directory.
    async fn write_sync_log_meta(&self, dir: &str, meta: &SyncLogMeta, ctx: &FsContext) -> Result<()> {
        let path = Self::meta_path(dir, SYNC_LOG_FILE);
        let data = serde_json::to_vec(meta)?;
        FS_CTX.scope(ctx.clone(), async {
            self.primary_backend.write(&path, &data, 0, WriteFlag::Create).await.map(|_| ())
        }).await
    }

    /// Serialized read-modify-write of both `.redirect.json` and `.sync_log.json` in a directory.
    ///
    /// Acquires the directory lock, reads both metadata files, applies `op`, and writes both back.
    /// This prevents concurrent updates from losing entries.
    pub async fn update_dir_meta<F>(&self, dir: &str, ctx: &FsContext, op: F) -> Result<()>
    where
        F: FnOnce(&mut RedirectMeta, &mut SyncLogMeta) -> Result<()>,
    {
        let lock = self.get_dir_lock(dir).await;
        let _guard = lock.lock().await;

        let mut redirect_meta = self.read_redirect_meta(dir, ctx).await?;
        let mut sync_log_meta = self.read_sync_log_meta(dir, ctx).await?;

        op(&mut redirect_meta, &mut sync_log_meta)?;

        self.write_redirect_meta(dir, &redirect_meta, ctx).await?;
        self.write_sync_log_meta(dir, &sync_log_meta, ctx).await?;

        Ok(())
    }

    /// Serialized read-modify-write of two directories' metadata (for cross-directory rename).
    ///
    /// Acquires both directory locks in lexicographic order to prevent deadlock,
    /// then reads and updates all four metadata files within the same critical section.
    /// Caller must ensure source_dir != target_dir; use update_dir_meta for same-directory case.
    pub async fn update_dual_dir_meta<F>(
        &self,
        source_dir: &str,
        target_dir: &str,
        ctx: &FsContext,
        op: F,
    ) -> Result<()>
    where
        F: FnOnce(
            &mut RedirectMeta,
            &mut SyncLogMeta,
            &mut RedirectMeta,
            &mut SyncLogMeta,
        ) -> Result<()>,
    {
        // Acquire locks in lexicographic order to avoid deadlock.
        let (first_dir, second_dir) = if source_dir < target_dir {
            (source_dir, target_dir)
        } else {
            (target_dir, source_dir)
        };

        let lock1 = self.get_dir_lock(first_dir).await;
        let lock2 = self.get_dir_lock(second_dir).await;
        let _guard1 = lock1.lock().await;
        let _guard2 = lock2.lock().await;

        let mut src_redirect = self.read_redirect_meta(source_dir, ctx).await?;
        let mut src_sync_log = self.read_sync_log_meta(source_dir, ctx).await?;
        let mut tgt_redirect = self.read_redirect_meta(target_dir, ctx).await?;
        let mut tgt_sync_log = self.read_sync_log_meta(target_dir, ctx).await?;

        op(
            &mut src_redirect,
            &mut src_sync_log,
            &mut tgt_redirect,
            &mut tgt_sync_log,
        )?;

        self.write_redirect_meta(source_dir, &src_redirect, ctx)
            .await?;
        self.write_sync_log_meta(source_dir, &src_sync_log, ctx)
            .await?;
        self.write_redirect_meta(target_dir, &tgt_redirect, ctx)
            .await?;
        self.write_sync_log_meta(target_dir, &tgt_sync_log, ctx)
            .await?;

        Ok(())
    }

    /// Read redirect metadata for a directory (public, used by read_dir to merge redirect entries).
    pub async fn get_redirect_meta(&self, dir: &str, ctx: &FsContext) -> Result<RedirectMeta> {
        self.read_redirect_meta(dir, ctx).await
    }

    /// Read sync log metadata for a directory (public, used by retry_loop).
    pub async fn get_sync_log_meta(&self, dir: &str, ctx: &FsContext) -> Result<SyncLogMeta> {
        self.read_sync_log_meta(dir, ctx).await
    }

    /// Get a reference to the context resolver.
    pub fn ctx_resolver(&self) -> &Arc<dyn FsContextResolver> {
        &self.ctx_resolver
    }

    /// Get a reference to the primary backend.
    pub fn primary_backend(&self) -> &Arc<dyn FileSystem> {
        &self.primary_backend
    }
}

/// Per-path serialization queue for async write ordering.
///
/// Ensures that multiple writes to the same path are executed in FIFO order
/// on backup backends, preventing out-of-order application.
pub struct PathSerializer {
    queues: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl PathSerializer {
    /// Create a new PathSerializer.
    pub fn new() -> Self {
        Self {
            queues: Mutex::new(HashMap::new()),
        }
    }

    /// Get or create a per-path serialization lock.
    pub async fn get_path_lock(&self, path: &str) -> Arc<Mutex<()>> {
        let mut queues = self.queues.lock().await;
        queues
            .entry(path.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }
}

impl Default for PathSerializer {
    fn default() -> Self {
        Self::new()
    }
}

/// Extract the directory path from a file path.
pub(crate) fn parent_dir(path: &str) -> String {
    match path.rfind('/') {
        Some(0) => "/".to_string(),
        Some(pos) => path[..pos].to_string(),
        None => "/".to_string(),
    }
}

/// Extract the file name from a path.
pub(crate) fn file_name(path: &str) -> &str {
    match path.rfind('/') {
        Some(pos) => &path[pos + 1..],
        None => path,
    }
}

/// Snapshot the current FsContext from the task-local, returning an error if unset.
pub fn current_required_ctx() -> Result<FsContext> {
    FS_CTX.try_with(|c| c.clone())
        .map_err(|_| Error::internal("FsContext not set in current task"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parent_dir() {
        assert_eq!(parent_dir("/a/b/c.txt"), "/a/b");
        assert_eq!(parent_dir("/a"), "/");
        assert_eq!(parent_dir("/"), "/");
    }

    #[test]
    fn test_file_name() {
        assert_eq!(file_name("/a/b/c.txt"), "c.txt");
        assert_eq!(file_name("/a"), "a");
    }

    #[test]
    fn test_default_resolver() {
        let resolver = DefaultFsContextResolver;
        let ctx = resolver.resolve("/local/tenant-1/resources/file.txt").unwrap();
        assert_eq!(ctx.account_id(), "tenant-1");
    }

    #[test]
    fn test_default_resolver_invalid_path() {
        let resolver = DefaultFsContextResolver;
        assert!(resolver.resolve("/invalid/path").is_err());
    }
}
