//! Multi-write wrapper — routes operations across primary and backup backends.
//!
//! Implements `MultiWriteWrappedFS` which handles:
//! - Write fanout to primary + backup backends (sync/async)
//! - Read routing with priority-based fallback chain
//! - Redirect policy evaluation
//! - Exclude policy filtering
//! - `.redirect.json` / `.sync_log.json` metadata management

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use regex::Regex;
use serde_json::{json, Value};

use super::context::{FsContext, FS_CTX};
use super::errors::{Error, Result};
use super::filesystem::{normalize_prefix_path, FileSystem};
use super::multiwrite_meta::{
    current_required_ctx, file_name, parent_dir, DefaultFsContextResolver, MetaStateStore,
    PathSerializer,
};
use super::types::{
    BackendRole, BackendSyncState, FileInfo, GrepResult, OperationItemConfig, RedirectEntry,
    RedirectPolicy, SyncLogEntry, SyncType, TreeEntry, WriteFlag,
};

/// Internal file names that are invisible to users.
const INTERNAL_NAMES: &[&str] = &[".path.ovlock", ".sync_log.json", ".redirect.json"];

/// Maximum retries per file per retry_loop round.
const MAX_RETRY_PER_ROUND: usize = 3;

/// A backend entry within the multi-write wrapper.
pub struct BackendEntry {
    /// Logical name (globally unique)
    pub name: String,
    /// Role: Primary or Backup
    pub role: BackendRole,
    /// The backend filesystem handle (may be encrypted)
    pub backend: Arc<dyn FileSystem>,
    /// Operations this backend participates in (only for Backup)
    pub operations: Vec<OperationItemConfig>,
    /// Exclude policies (only for Backup)
    pub excludes: Vec<RedirectPolicy>,
}

impl BackendEntry {
    /// Check if this backend participates in read operations.
    fn participates_in_read(&self) -> bool {
        self.operations.iter().any(|op| op.operation == "read")
    }

    /// Check if this backend participates in write operations.
    /// Backups default to write-enabled when operations is empty.
    fn participates_in_write(&self) -> bool {
        if self.operations.is_empty() {
            true
        } else {
            self.operations.iter().any(|op| op.operation == "write")
        }
    }

    /// Get read priority (lower = higher priority). Returns None if not read-enabled.
    fn read_priority(&self) -> Option<u32> {
        self.operations
            .iter()
            .find(|op| op.operation == "read")
            .map(|op| op.priority)
    }
}

/// File policy trait — shared by redirects and excludes.
pub trait FilePolicy {
    /// Check if this policy matches the given file.
    fn matches(&self, path: &str, size: u64) -> bool;
}

impl FilePolicy for RedirectPolicy {
    fn matches(&self, path: &str, size: u64) -> bool {
        match self {
            RedirectPolicy::FileOverSizePolicy { max_size_mb, .. } => {
                let max_bytes = max_size_mb * 1024 * 1024;
                size > max_bytes
            }
            RedirectPolicy::FileExtensionPolicy { extensions, .. } => {
                let name = file_name(path);
                extensions.iter().any(|ext_pattern| {
                    if let Ok(re) = Regex::new(ext_pattern) {
                        re.is_match(name)
                    } else {
                        name.ends_with(ext_pattern.as_str())
                    }
                })
            }
        }
    }
}

/// Inner state shared via Arc for async spawn and retry_loop.
struct Inner {
    /// All backend entries (primary at index 0)
    backends: Vec<BackendEntry>,
    /// Index of the primary backend
    primary_idx: usize,
    /// Sync type: Async or Sync
    sync_type: SyncType,
    /// Minimum backup ack count for sync mode
    write_ack_count: usize,
    /// Timeout for waiting backup ack in sync mode (ms)
    write_ack_timeout_ms: u64,
    /// Semaphore for async write concurrency control
    write_sem: Option<Arc<tokio::sync::Semaphore>>,
    /// Primary redirect policies
    redirects: Vec<RedirectPolicy>,
    /// Metadata store (encrypted via primary_backend)
    meta_store: MetaStateStore,
    /// Per-path serialization queues
    path_queues: PathSerializer,
    /// Global sequence counter for sync_log
    seq_counter: AtomicU64,
}

/// Multi-write wrapped filesystem.
pub struct MultiWriteWrappedFS {
    inner: Arc<Inner>,
}

impl MultiWriteWrappedFS {
    /// Build a MultiWriteWrappedFS with pre-built backup backends.
    pub fn with_backends(
        primary_backend: Arc<dyn FileSystem>,
        backup_entries: Vec<BackendEntry>,
        redirects: Vec<RedirectPolicy>,
        sync_type_str: &str,
        write_ack_count: Option<usize>,
        write_ack_timeout_ms: Option<u64>,
        write_concurrency: Option<usize>,
    ) -> Result<Self> {
        let mut backends = Vec::new();
        backends.push(BackendEntry {
            name: "primary".to_string(),
            role: BackendRole::Primary,
            backend: primary_backend.clone(),
            operations: Vec::new(),
            excludes: Vec::new(),
        });
        backends.extend(backup_entries);

        let sync_type = match sync_type_str {
            "sync" => SyncType::Sync,
            _ => SyncType::Async,
        };

        let write_sem = write_concurrency
            .filter(|&n| n > 0)
            .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

        let ctx_resolver = Arc::new(DefaultFsContextResolver);
        let meta_store = MetaStateStore::new(primary_backend, ctx_resolver);

        let inner = Arc::new(Inner {
            backends,
            primary_idx: 0,
            sync_type,
            write_ack_count: write_ack_count.unwrap_or(usize::MAX),
            write_ack_timeout_ms: write_ack_timeout_ms.unwrap_or(0),
            write_sem,
            redirects,
            meta_store,
            path_queues: PathSerializer::new(),
            seq_counter: AtomicU64::new(0),
        });

        // Start retry_loop if there are write-enabled backups.
        if inner.write_backups().next().is_some() {
            tokio::spawn(Inner::retry_loop(Arc::clone(&inner)));
        }

        Ok(Self { inner })
    }

    /// Collect effective sync work entries under a path using the current request context.
    async fn collect_sync_work(
        &self,
        path: &str,
    ) -> Result<Vec<(String, SyncLogEntry, Vec<String>)>> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let normalized = normalize_prefix_path(path);
        let path_info = <Self as FileSystem>::stat(self, &normalized).await?;
        let mut dirs = Vec::new();
        let mut seen_dirs = HashSet::new();

        let add_dir = |dirs: &mut Vec<String>, seen_dirs: &mut HashSet<String>, dir: String| {
            if seen_dirs.insert(dir.clone()) {
                dirs.push(dir);
            }
        };

        if path_info.is_dir {
            add_dir(&mut dirs, &mut seen_dirs, normalized.clone());
            for entry in inner
                .primary()
                .backend
                .tree_directory(&normalized, true, None, None)
                .await?
            {
                if entry.info.is_dir {
                    add_dir(
                        &mut dirs,
                        &mut seen_dirs,
                        normalize_prefix_path(&entry.path),
                    );
                }
            }
        } else {
            add_dir(
                &mut dirs,
                &mut seen_dirs,
                normalize_prefix_path(&parent_dir(&normalized)),
            );
        }

        let mut work = Vec::new();
        for dir in dirs {
            let sync_log = inner.meta_store.get_sync_log_meta(&dir, &ctx).await?;
            if sync_log.entries.is_empty() {
                continue;
            }
            let redirect_meta = inner
                .meta_store
                .get_redirect_meta(&dir, &ctx)
                .await
                .unwrap_or_default();

            for (name, sync_entry) in sync_log.entries {
                let file_path = if dir == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", dir, name)
                };
                if !path_info.is_dir && file_path != normalized {
                    continue;
                }
                let target_backend_names = if let Some(redir) = redirect_meta.entries.get(&name) {
                    redir.targets.clone()
                } else {
                    inner
                        .write_backups()
                        .filter(|be| !inner.is_excluded(be, &file_path, 0))
                        .map(|be| be.name.clone())
                        .collect()
                };
                work.push((file_path, sync_entry, target_backend_names));
            }
        }

        Ok(work)
    }

    /// Query effective multi-write sync status under a file or directory path.
    pub async fn system_sync_status(&self, path: &str) -> Result<Value> {
        let work = self.collect_sync_work(path).await?;
        let mut entries = Vec::new();
        let mut pending_target_count = 0usize;

        for (file_path, sync_entry, target_backend_names) in work {
            let mut targets = Vec::new();
            let mut all_synced = true;

            for backend_name in target_backend_names {
                let acked_seq = sync_entry
                    .backends
                    .get(&backend_name)
                    .map(|state| state.acked_seq)
                    .unwrap_or(0);
                let in_sync = acked_seq >= sync_entry.latest_seq;
                if !in_sync {
                    pending_target_count += 1;
                    all_synced = false;
                }
                targets.push(json!({
                    "name": backend_name,
                    "acked_seq": acked_seq,
                    "in_sync": in_sync,
                }));
            }

            entries.push(json!({
                "path": file_path,
                "latest_seq": sync_entry.latest_seq,
                "last_op": sync_entry.last_op,
                "rename_to": sync_entry.rename_to,
                "mode": sync_entry.mode,
                "all_synced": all_synced,
                "targets": targets,
            }));
        }

        entries.sort_by(|a, b| {
            let ap = a.get("path").and_then(Value::as_str).unwrap_or_default();
            let bp = b.get("path").and_then(Value::as_str).unwrap_or_default();
            ap.cmp(bp)
        });

        Ok(json!({
            "path": normalize_prefix_path(path),
            "entry_count": entries.len(),
            "pending_target_count": pending_target_count,
            "entries": entries,
        }))
    }

    /// Manually retry lagging multi-write targets under a file or directory path.
    pub async fn system_sync_retry(&self, path: &str) -> Result<Value> {
        let ctx = current_required_ctx()?;
        let work = self.collect_sync_work(path).await?;
        let mut results = Vec::new();
        let mut retried = 0usize;
        let mut failed = 0usize;
        let mut skipped = 0usize;

        for (file_path, sync_entry, target_backend_names) in work {
            for backend_name in target_backend_names {
                let acked_seq = sync_entry
                    .backends
                    .get(&backend_name)
                    .map(|state| state.acked_seq)
                    .unwrap_or(0);
                if acked_seq >= sync_entry.latest_seq {
                    skipped += 1;
                    results.push(json!({
                        "path": file_path,
                        "target": backend_name,
                        "status": "skipped",
                        "latest_seq": sync_entry.latest_seq,
                        "acked_seq": acked_seq,
                    }));
                    continue;
                }

                let mut last_error = None;
                let mut success = false;
                for _attempt in 0..MAX_RETRY_PER_ROUND {
                    match self
                        .inner
                        .replay_operation(&file_path, &sync_entry, &backend_name, &ctx)
                        .await
                    {
                        Ok(()) => {
                            success = true;
                            break;
                        }
                        Err(err) => {
                            last_error = Some(err.to_string());
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }

                if success {
                    retried += 1;
                    results.push(json!({
                        "path": file_path,
                        "target": backend_name,
                        "status": "retried",
                        "latest_seq": sync_entry.latest_seq,
                        "acked_seq": sync_entry.latest_seq,
                    }));
                } else {
                    failed += 1;
                    results.push(json!({
                        "path": file_path,
                        "target": backend_name,
                        "status": "failed",
                        "latest_seq": sync_entry.latest_seq,
                        "acked_seq": acked_seq,
                        "error": last_error.unwrap_or_else(|| "unknown replay error".to_string()),
                    }));
                }
            }
        }

        Ok(json!({
            "path": normalize_prefix_path(path),
            "retried": retried,
            "failed": failed,
            "skipped": skipped,
            "results": results,
        }))
    }
}

impl Inner {
    /// Iterate over write-enabled backup entries.
    fn write_backups(&self) -> impl Iterator<Item = &BackendEntry> {
        self.backends[self.primary_idx + 1..]
            .iter()
            .filter(|be| be.participates_in_write())
    }

    /// Iterate over read-enabled backup entries sorted by priority.
    fn read_backups_sorted(&self) -> Vec<&BackendEntry> {
        let mut read_backups: Vec<&BackendEntry> = self.backends[self.primary_idx + 1..]
            .iter()
            .filter(|be| be.participates_in_read())
            .collect();
        read_backups.sort_by_key(|be| be.read_priority().unwrap_or(u32::MAX));
        read_backups
    }

    /// Get the primary backend entry.
    fn primary(&self) -> &BackendEntry {
        &self.backends[self.primary_idx]
    }

    /// Get a backup entry by name.
    fn backup_by_name(&self, name: &str) -> Option<&BackendEntry> {
        self.backends.iter().find(|be| be.name == name)
    }

    /// Check if a file should be excluded from a backup.
    fn is_excluded(&self, backup: &BackendEntry, path: &str, size: u64) -> bool {
        backup
            .excludes
            .iter()
            .any(|policy| policy.matches(path, size))
    }

    /// Check if a file matches any redirect policy.
    fn check_redirect(&self, path: &str, size: u64) -> Option<Vec<String>> {
        for policy in &self.redirects {
            if policy.matches(path, size) {
                let targets = match policy {
                    RedirectPolicy::FileOverSizePolicy { target, .. } => target.clone(),
                    RedirectPolicy::FileExtensionPolicy { target, .. } => target.clone(),
                };
                return targets;
            }
        }
        None
    }

    /// Generate the next sequence number.
    fn next_seq(&self) -> u64 {
        self.seq_counter.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Resolve the read backend for a path using the fallback chain.
    async fn resolve_read_backend(&self, path: &str) -> Option<Arc<dyn FileSystem>> {
        let normalized = normalize_prefix_path(path);

        // 1. Try read-enabled backups by priority
        for backup in self.read_backups_sorted() {
            if backup.backend.exists(&normalized).await {
                return Some(backup.backend.clone());
            }
        }

        // 2. Try primary
        if self.primary().backend.exists(&normalized).await {
            return Some(self.primary().backend.clone());
        }

        // 3. Check redirect targets
        let dir = parent_dir(&normalized);
        let name = file_name(&normalized).to_string();
        let ctx = match current_required_ctx() {
            Ok(c) => c,
            Err(_) => return None,
        };
        if let Ok(redirect_meta) = self.meta_store.get_redirect_meta(&dir, &ctx).await {
            if let Some(entry) = redirect_meta.entries.get(&name) {
                for target_name in &entry.targets {
                    if let Some(be) = self.backup_by_name(target_name) {
                        if be.backend.exists(&normalized).await {
                            return Some(be.backend.clone());
                        }
                    }
                }
            }
        }

        None
    }

    /// Fanout a write operation to all write-enabled backups.
    /// Takes `&Arc<Inner>` so spawned tasks can clone the Arc for acked_seq updates.
    /// `ctx` is required for encrypted backup backends and acked_seq updates.
    async fn fanout_write<F, Fut>(
        inner: &Arc<Inner>,
        path: &str,
        size: u64,
        ctx: FsContext,
        op: F,
    ) -> Result<()>
    where
        F: Fn(Arc<dyn FileSystem>) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = Result<()>> + Send,
    {
        let targets: Vec<&BackendEntry> = inner
            .write_backups()
            .filter(|be| !inner.is_excluded(be, path, size))
            .collect();

        if targets.is_empty() {
            return Ok(());
        }

        match inner.sync_type {
            SyncType::Sync => Inner::fanout_sync(inner, path, &targets, &ctx, op).await,
            SyncType::Async => {
                Inner::fanout_async(inner, path, targets, &ctx, op).await;
                Ok(())
            }
        }
    }

    /// Fanout a write operation to explicitly named backup targets (used by redirect path).
    /// Resolves names to BackendEntry references, then delegates to sync/async state machine.
    async fn fanout_write_to_targets<F, Fut>(
        inner: &Arc<Inner>,
        path: &str,
        target_names: &[String],
        ctx: FsContext,
        op: F,
    ) -> Result<()>
    where
        F: Fn(Arc<dyn FileSystem>) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = Result<()>> + Send,
    {
        let targets: Vec<&BackendEntry> = target_names
            .iter()
            .filter_map(|name| inner.backup_by_name(name))
            .collect();

        if targets.is_empty() {
            return Ok(());
        }

        match inner.sync_type {
            SyncType::Sync => Inner::fanout_sync(inner, path, &targets, &ctx, op).await,
            SyncType::Async => {
                Inner::fanout_async(inner, path, targets, &ctx, op).await;
                Ok(())
            }
        }
    }

    /// Synchronous fanout: execute writes in parallel, wait for quorum.
    async fn fanout_sync<F, Fut>(
        inner: &Arc<Inner>,
        path: &str,
        targets: &[&BackendEntry],
        ctx: &FsContext,
        op: F,
    ) -> Result<()>
    where
        F: Fn(Arc<dyn FileSystem>) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = Result<()>> + Send,
    {
        let ack_count = inner.write_ack_count.min(targets.len());
        let timeout = if inner.write_ack_timeout_ms > 0 {
            Some(Duration::from_millis(inner.write_ack_timeout_ms))
        } else {
            None
        };

        let path_owned = path.to_string();
        let ctx = Some(ctx.clone());

        // Launch parallel tasks for all backup writes.
        let mut handles = Vec::new();
        for target in targets {
            let fs = target.backend.clone();
            let name = target.name.clone();
            let path = path_owned.clone();
            let inner = Arc::clone(inner);
            let ctx = ctx.clone();
            let op_clone = op.clone();

            handles.push(tokio::spawn(async move {
                // Wrap in FS_CTX.scope so encrypted backends can access account_id.
                let exec = async {
                    if let Some(ref ctx) = ctx {
                        FS_CTX.scope(ctx.clone(), op_clone(fs)).await
                    } else {
                        op_clone(fs).await
                    }
                };

                let result = if let Some(timeout) = timeout {
                    match tokio::time::timeout(timeout, exec).await {
                        Ok(Ok(())) => Ok(()),
                        Ok(Err(e)) => Err(format!("{}: {}", name, e)),
                        Err(_) => Err(format!("{}: timeout", name)),
                    }
                } else {
                    match exec.await {
                        Ok(()) => Ok(()),
                        Err(e) => Err(format!("{}: {}", name, e)),
                    }
                };

                // Update acked_seq on success.
                if result.is_ok() {
                    if let Some(ref ctx) = ctx {
                        let _ = inner.update_backup_acked_seq(&path, &name, ctx).await;
                    }
                }

                (name, result)
            }));
        }

        let results = futures::future::join_all(handles).await;

        let mut successes = 0usize;
        let mut errors = Vec::new();

        for result in results {
            match result {
                Ok((_name, Ok(()))) => {
                    successes += 1;
                }
                Ok((_name, Err(e))) => {
                    errors.push(e);
                }
                Err(e) => {
                    errors.push(format!("join error: {}", e));
                }
            }
        }

        if successes >= ack_count {
            Ok(())
        } else {
            Err(Error::internal(format!(
                "sync write failed: {}/{} backups succeeded. Errors: {}",
                successes,
                targets.len(),
                errors.join("; ")
            )))
        }
    }

    /// Asynchronous fanout: spawn background tasks that update acked_seq on completion.
    /// Uses per-path serialization to prevent out-of-order application on backup backends.
    async fn fanout_async<F, Fut>(
        inner: &Arc<Inner>,
        path: &str,
        targets: Vec<&BackendEntry>,
        ctx: &FsContext,
        op: F,
    ) where
        F: Fn(Arc<dyn FileSystem>) -> Fut + Send + Sync + Clone + 'static,
        Fut: Future<Output = Result<()>> + Send,
    {
        let path_owned = path.to_string();
        let sem = inner.write_sem.clone();
        // Per-path serialization lock: all spawns for the same path share one mutex,
        // ensuring FIFO order of operations on backup backends.
        let path_lock = inner.path_queues.get_path_lock(&path_owned).await;

        for target in targets {
            let fs = target.backend.clone();
            let name = target.name.clone();
            let path = path_owned.clone();
            let ctx = ctx.clone();
            let sem = sem.clone();
            let inner = Arc::clone(inner);
            let op_clone = op.clone();
            let path_lock = path_lock.clone();

            tokio::spawn(async move {
                // Per-path serialization: ensure FIFO order for the same path.
                let _guard = path_lock.lock().await;

                let _permit = if let Some(ref sem) = sem {
                    sem.acquire().await.ok()
                } else {
                    None
                };

                // Wrap in FS_CTX.scope so encrypted backends can access account_id.
                let result = FS_CTX.scope(ctx.clone(), op_clone(fs)).await;

                // Update acked_seq on successful write.
                if result.is_ok() {
                    let _ = inner.update_backup_acked_seq(&path, &name, &ctx).await;
                }
            });
        }
    }

    /// Update the acked_seq for a backup in the sync log.
    async fn update_backup_acked_seq(
        &self,
        path: &str,
        backup_name: &str,
        ctx: &FsContext,
    ) -> Result<()> {
        let dir = parent_dir(path);
        let backup_name = backup_name.to_string();
        let name = file_name(path).to_string();
        self.meta_store
            .update_dir_meta(&dir, ctx, move |_redirect, sync_log| {
                if let Some(entry) = sync_log.entries.get_mut(&name) {
                    entry.backends.insert(
                        backup_name.clone(),
                        BackendSyncState {
                            acked_seq: entry.latest_seq,
                        },
                    );
                }
                Ok(())
            })
            .await
    }

    /// Replay a single operation on a lagging backup.
    async fn replay_operation(
        &self,
        file_path: &str,
        entry: &SyncLogEntry,
        backup_name: &str,
        ctx: &FsContext,
    ) -> Result<()> {
        let backup = match self.backup_by_name(backup_name) {
            Some(b) => b,
            None => {
                return Err(Error::internal(format!(
                    "backup '{}' not found",
                    backup_name
                )))
            }
        };

        match entry.last_op.as_str() {
            "write" | "truncate" => {
                // Read content from primary (or redirect target if redirected).
                let dir = parent_dir(file_path);
                let name = file_name(file_path).to_string();
                let data =
                    if let Ok(redirect_meta) = self.meta_store.get_redirect_meta(&dir, ctx).await {
                        if let Some(redir) = redirect_meta.entries.get(&name) {
                            // Read from the first redirect target.
                            let mut content = None;
                            for target_name in &redir.targets {
                                if let Some(be) = self.backup_by_name(target_name) {
                                    if let Ok(d) = be.backend.read(file_path, 0, 0).await {
                                        content = Some(d);
                                        break;
                                    }
                                }
                            }
                            content.unwrap_or_default()
                        } else {
                            self.primary()
                                .backend
                                .read(file_path, 0, 0)
                                .await
                                .unwrap_or_default()
                        }
                    } else {
                        self.primary()
                            .backend
                            .read(file_path, 0, 0)
                            .await
                            .unwrap_or_default()
                    };

                FS_CTX
                    .scope(ctx.clone(), async {
                        backup.backend.ensure_parent_dirs(file_path, 0o755).await?;
                        backup
                            .backend
                            .write(file_path, &data, 0, WriteFlag::Create)
                            .await
                            .map(|_| ())
                    })
                    .await?;
            }
            "create" => {
                FS_CTX
                    .scope(ctx.clone(), async {
                        backup.backend.create(file_path).await
                    })
                    .await?;
            }
            "mkdir" => {
                let mode = entry.mode.unwrap_or(0o755);
                FS_CTX
                    .scope(ctx.clone(), async {
                        backup.backend.mkdir(file_path, mode).await
                    })
                    .await?;
            }
            "remove" => {
                // Ignore NotFound — file may already be gone.
                let _ = FS_CTX
                    .scope(ctx.clone(), async {
                        backup.backend.remove(file_path).await
                    })
                    .await;
            }
            "remove_all" => {
                let _ = FS_CTX
                    .scope(ctx.clone(), async {
                        backup.backend.remove_all(file_path).await
                    })
                    .await;
            }
            "rename" => {
                if let Some(ref new_path) = entry.rename_to {
                    let _ = FS_CTX
                        .scope(ctx.clone(), async {
                            backup.backend.rename(file_path, new_path).await
                        })
                        .await;
                }
            }
            "chmod" => {
                let mode = entry.mode.unwrap_or(0o644);
                FS_CTX
                    .scope(ctx.clone(), async {
                        backup.backend.chmod(file_path, mode).await
                    })
                    .await?;
            }
            _ => {
                // Unknown operation — skip.
            }
        }

        // Update acked_seq after successful replay.
        self.update_backup_acked_seq(file_path, backup_name, ctx)
            .await?;

        Ok(())
    }

    /// Background retry loop: periodically scans sync_log for lagging backups and replays.
    async fn retry_loop(inner: Arc<Inner>) {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;

            // Scan primary backend for all .sync_log.json files.
            let primary = inner.primary().backend.clone();
            let tree_result = primary.tree_directory("/", true, None, None).await;
            let tree_entries = match tree_result {
                Ok(e) => e,
                Err(_) => continue,
            };

            for entry in tree_entries {
                if !entry.info.name.ends_with(SYNC_LOG_FILE) {
                    continue;
                }

                let dir = parent_dir(&entry.path);
                let ctx = match inner.meta_store.ctx_resolver().resolve(&dir) {
                    Ok(c) => c,
                    Err(_) => continue,
                };

                let sync_log = match inner.meta_store.get_sync_log_meta(&dir, &ctx).await {
                    Ok(s) => s,
                    Err(_) => continue,
                };

                for (file_name, sync_entry) in &sync_log.entries {
                    // Construct full path from dir + file name (entries key is now file name only)
                    let file_path = if dir == "/" {
                        format!("/{}", file_name)
                    } else {
                        format!("{}/{}", dir, file_name)
                    };

                    // Compute target_backends from current effective config:
                    // - If file is redirected → targets = redirect targets
                    // - Otherwise → all write-enabled backups minus excludes
                    // - Ignore backup names in sync_log.entries[file].backends that are no longer configured
                    let target_backend_names: Vec<String> = if let Ok(redirect_meta) =
                        inner.meta_store.get_redirect_meta(&dir, &ctx).await
                    {
                        if let Some(redir) = redirect_meta.entries.get(file_name) {
                            redir.targets.clone()
                        } else {
                            inner
                                .write_backups()
                                .filter(|be| {
                                    // Exclude backups whose exclude policy matches this file
                                    !inner.is_excluded(be, &file_path, 0)
                                })
                                .map(|be| be.name.clone())
                                .collect()
                        }
                    } else {
                        inner
                            .write_backups()
                            .filter(|be| !inner.is_excluded(be, &file_path, 0))
                            .map(|be| be.name.clone())
                            .collect()
                    };

                    for backup_name in &target_backend_names {
                        let acked = sync_entry
                            .backends
                            .get(backup_name)
                            .map(|s| s.acked_seq)
                            .unwrap_or(0);
                        if acked >= sync_entry.latest_seq {
                            continue;
                        }

                        // Retry up to MAX_RETRY_PER_ROUND times.
                        for _attempt in 0..MAX_RETRY_PER_ROUND {
                            if inner
                                .replay_operation(&file_path, sync_entry, backup_name, &ctx)
                                .await
                                .is_ok()
                            {
                                break;
                            }
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }
            }
        }
    }
}

/// Internal metadata file name used by retry_loop tree scan.
const SYNC_LOG_FILE: &str = ".sync_log.json";

// ── FileSystem trait implementation ──

#[async_trait]
impl FileSystem for MultiWriteWrappedFS {
    async fn create(&self, path: &str) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.create(&path_owned).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "create".to_string(),
                        rename_to: None,
                        mode: None,
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        let p = path_owned.clone();
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, 0, ctx.clone(), move |fs| {
            let p = p2.clone();
            async move { fs.create(&p).await }
        })
        .await?;

        Ok(())
    }

    async fn mkdir(&self, path: &str, mode: u32) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.mkdir(&path_owned, mode).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "mkdir".to_string(),
                        rename_to: None,
                        mode: Some(mode),
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        let p = path_owned.clone();
        let m = mode;
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, 0, ctx.clone(), move |fs| {
            let p = p2.clone();
            async move { fs.mkdir(&p, m).await }
        })
        .await?;

        Ok(())
    }

    async fn remove(&self, path: &str) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.remove(&path_owned).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "remove".to_string(),
                        rename_to: None,
                        mode: None,
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        let p = path_owned.clone();
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, 0, ctx.clone(), move |fs| {
            let p = p2.clone();
            async move { fs.remove(&p).await }
        })
        .await?;

        Ok(())
    }

    async fn remove_all(&self, path: &str) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.remove_all(&path_owned).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "remove_all".to_string(),
                        rename_to: None,
                        mode: None,
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        let p = path_owned.clone();
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, 0, ctx.clone(), move |fs| {
            let p = p2.clone();
            async move { fs.remove_all(&p).await }
        })
        .await?;

        Ok(())
    }

    async fn read(&self, path: &str, offset: u64, size: u64) -> Result<Vec<u8>> {
        if let Some(fs) = self.inner.resolve_read_backend(path).await {
            return fs.read(path, offset, size).await;
        }
        Err(Error::not_found(path))
    }

    async fn write(&self, path: &str, data: &[u8], offset: u64, flags: WriteFlag) -> Result<u64> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let data_len = data.len() as u64;
        let path_owned = path.to_string();

        // Check redirect policies
        if let Some(targets) = inner.check_redirect(&path_owned, data_len) {
            // Write redirect + sync_log entry BEFORE fanout so acked_seq updates can find it.
            let dir = parent_dir(&path_owned);
            let name = file_name(&path_owned).to_string();
            let seq = inner.next_seq();
            let targets_clone = targets.clone();
            inner
                .meta_store
                .update_dir_meta(&dir, &ctx, move |redirect, sync_log| {
                    redirect.entries.insert(
                        name.clone(),
                        RedirectEntry {
                            targets: targets_clone.clone(),
                        },
                    );
                    sync_log.entries.insert(
                        name.clone(),
                        SyncLogEntry {
                            latest_seq: seq,
                            last_op: "write".to_string(),
                            rename_to: None,
                            mode: None,
                            backends: HashMap::new(),
                        },
                    );
                    Ok(())
                })
                .await?;

            // Fanout to redirect targets via sync/async state machine.
            // On failure, retry_loop will compensate.
            let p = path_owned.clone();
            let d = data.to_vec();
            let t = targets.clone();
            let p2 = p.clone();
            Inner::fanout_write_to_targets(
                inner,
                &p,
                &t,
                ctx.clone(),
                move |fs: Arc<dyn FileSystem>| {
                    let p = p2.clone();
                    let d = d.clone();
                    async move {
                        fs.ensure_parent_dirs(&p, 0o755).await?;
                        fs.write(&p, &d, offset, flags).await.map(|_| ())
                    }
                },
            )
            .await?;

            return Ok(data_len);
        }

        // Normal write: primary first
        let p = path_owned.clone();
        let d = data.to_vec();
        let written = FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.write(&p, &d, offset, flags).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "write".to_string(),
                        rename_to: None,
                        mode: None,
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        // Fanout to backups (acked_seq updates now find the entry)
        let p = path_owned.clone();
        let d = data.to_vec();
        let p2 = p.clone();
        Inner::fanout_write(
            inner,
            &p,
            data_len,
            ctx.clone(),
            move |fs: Arc<dyn FileSystem>| {
                let p = p2.clone();
                let d = d.clone();
                async move { fs.write(&p, &d, offset, flags).await.map(|_| ()) }
            },
        )
        .await?;

        Ok(written)
    }

    async fn read_dir(&self, path: &str) -> Result<Vec<FileInfo>> {
        let inner = &self.inner;
        let mut entries = inner.primary().backend.read_dir(path).await?;

        // Filter internal names
        entries.retain(|e| !INTERNAL_NAMES.contains(&e.name.as_str()));

        // Merge redirect entries so users can see redirected files in listings.
        let ctx = match current_required_ctx() {
            Ok(c) => c,
            Err(_) => return Ok(entries),
        };

        if let Ok(redirect_meta) = inner.meta_store.get_redirect_meta(path, &ctx).await {
            for (name, _redirect_entry) in &redirect_meta.entries {
                if !entries.iter().any(|e| &e.name == name) {
                    entries.push(FileInfo::new_file(name.clone(), 0, 0o644));
                }
            }
        }

        Ok(entries)
    }

    async fn stat(&self, path: &str) -> Result<FileInfo> {
        if let Some(fs) = self.inner.resolve_read_backend(path).await {
            return fs.stat(path).await;
        }
        Err(Error::not_found(path))
    }

    async fn rename(&self, old_path: &str, new_path: &str) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let old_owned = old_path.to_string();
        let new_owned = new_path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.rename(&old_owned, &new_owned).await
            })
            .await?;

        // Write sync_log entries BEFORE fanout so acked_seq updates can find them.
        let source_dir = parent_dir(&old_owned);
        let target_dir = parent_dir(&new_owned);
        let old_name = file_name(&old_owned).to_string();
        let new_name = file_name(&new_owned).to_string();
        let seq = inner.next_seq();
        let new_owned_for_rename_to = new_owned.clone();

        if source_dir == target_dir {
            // Same-directory rename: single lock, update one dir's metadata.
            inner
                .meta_store
                .update_dir_meta(&source_dir, &ctx, move |redirect, sync_log| {
                    sync_log.entries.insert(
                        old_name.clone(),
                        SyncLogEntry {
                            latest_seq: seq,
                            last_op: "rename".to_string(),
                            rename_to: Some(new_owned_for_rename_to),
                            mode: None,
                            backends: HashMap::new(),
                        },
                    );
                    // Migrate redirect entry if present.
                    if let Some(redirect_entry) = redirect.entries.remove(&old_name) {
                        redirect.entries.insert(new_name.clone(), redirect_entry);
                    }
                    Ok(())
                })
                .await?;
        } else {
            // Cross-directory rename: dual lock, update both dirs' metadata.
            inner
                .meta_store
                .update_dual_dir_meta(
                    &source_dir,
                    &target_dir,
                    &ctx,
                    move |src_redirect, src_sync_log, tgt_redirect, tgt_sync_log| {
                        // Source dir: record rename for old file name.
                        src_sync_log.entries.insert(
                            old_name.clone(),
                            SyncLogEntry {
                                latest_seq: seq,
                                last_op: "rename".to_string(),
                                rename_to: Some(new_owned_for_rename_to),
                                mode: None,
                                backends: HashMap::new(),
                            },
                        );
                        // Source dir: if old file was redirected, migrate the redirect entry to target dir.
                        if let Some(redirect_entry) = src_redirect.entries.remove(&old_name) {
                            tgt_redirect
                                .entries
                                .insert(new_name.clone(), redirect_entry);
                        }

                        // Target dir: record that the file now exists here.
                        tgt_sync_log.entries.insert(
                            new_name,
                            SyncLogEntry {
                                latest_seq: seq,
                                last_op: "write".to_string(),
                                rename_to: None,
                                mode: None,
                                backends: HashMap::new(),
                            },
                        );
                        Ok(())
                    },
                )
                .await?;
        }

        let o = old_owned.clone();
        let n = new_owned.clone();
        let o2 = o.clone();
        Inner::fanout_write(inner, &o, 0, ctx.clone(), move |fs: Arc<dyn FileSystem>| {
            let o = o2.clone();
            let n = n.clone();
            async move { fs.rename(&o, &n).await }
        })
        .await?;

        Ok(())
    }

    async fn chmod(&self, path: &str, mode: u32) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.chmod(&path_owned, mode).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "chmod".to_string(),
                        rename_to: None,
                        mode: Some(mode),
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        let p = path_owned.clone();
        let m = mode;
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, 0, ctx.clone(), move |fs: Arc<dyn FileSystem>| {
            let p = p2.clone();
            async move { fs.chmod(&p, m).await }
        })
        .await?;

        Ok(())
    }

    async fn truncate(&self, path: &str, size: u64) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner.primary().backend.truncate(&path_owned, size).await
            })
            .await?;

        // Write sync_log entry BEFORE fanout so acked_seq updates can find it.
        let dir = parent_dir(&path_owned);
        let name = file_name(&path_owned).to_string();
        let seq = inner.next_seq();
        inner
            .meta_store
            .update_dir_meta(&dir, &ctx, move |_redirect, sync_log| {
                sync_log.entries.insert(
                    name,
                    SyncLogEntry {
                        latest_seq: seq,
                        last_op: "truncate".to_string(),
                        rename_to: None,
                        mode: None,
                        backends: HashMap::new(),
                    },
                );
                Ok(())
            })
            .await?;

        let p = path_owned.clone();
        let s = size;
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, s, ctx.clone(), move |fs: Arc<dyn FileSystem>| {
            let p = p2.clone();
            async move { fs.truncate(&p, s).await }
        })
        .await?;

        Ok(())
    }

    async fn ensure_parent_dirs(&self, path: &str, mode: u32) -> Result<()> {
        let ctx = current_required_ctx()?;
        let inner = &self.inner;
        let path_owned = path.to_string();

        FS_CTX
            .scope(ctx.clone(), async {
                inner
                    .primary()
                    .backend
                    .ensure_parent_dirs(&path_owned, mode)
                    .await
            })
            .await?;

        let p = path_owned.clone();
        let m = mode;
        let p2 = p.clone();
        Inner::fanout_write(inner, &p, 0, ctx.clone(), move |fs: Arc<dyn FileSystem>| {
            let p = p2.clone();
            async move { fs.ensure_parent_dirs(&p, m).await }
        })
        .await?;

        Ok(())
    }

    async fn grep(
        &self,
        path: &str,
        pattern: &str,
        recursive: bool,
        case_insensitive: bool,
        node_limit: Option<usize>,
        exclude_path: Option<&str>,
        level_limit: Option<usize>,
    ) -> Result<GrepResult> {
        let inner = &self.inner;
        let path_owned = path.to_string();
        let pattern_owned = pattern.to_string();
        let exclude_owned = exclude_path.map(|s| s.to_string());

        let mut result = inner
            .primary()
            .backend
            .grep(
                &path_owned,
                &pattern_owned,
                recursive,
                case_insensitive,
                node_limit,
                exclude_owned.as_deref(),
                level_limit,
            )
            .await?;

        // For redirect files, also grep in target backends.
        let ctx = match current_required_ctx() {
            Ok(c) => c,
            Err(_) => return Ok(result),
        };

        let search_dir = if inner
            .primary()
            .backend
            .stat(&path_owned)
            .await
            .map(|s| s.is_dir)
            .unwrap_or(false)
        {
            path_owned.clone()
        } else {
            parent_dir(&path_owned)
        };

        if let Ok(redirect_meta) = inner.meta_store.get_redirect_meta(&search_dir, &ctx).await {
            for (name, redirect_entry) in &redirect_meta.entries {
                for target_name in &redirect_entry.targets {
                    if let Some(be) = inner.backup_by_name(target_name) {
                        let redirect_path = if search_dir == "/" {
                            format!("/{}", name)
                        } else {
                            format!("{}/{}", search_dir, name)
                        };
                        if let Ok(target_result) = be
                            .backend
                            .grep(
                                &redirect_path,
                                &pattern_owned,
                                false,
                                case_insensitive,
                                node_limit,
                                None,
                                None,
                            )
                            .await
                        {
                            for m in target_result.matches {
                                if node_limit.is_some_and(|limit| result.count >= limit) {
                                    break;
                                }
                                result.add_match(m.file, m.line, m.content);
                            }
                        }
                    }
                }
            }
        }

        Ok(result)
    }

    async fn tree_directory(
        &self,
        path: &str,
        show_hidden: bool,
        node_limit: Option<usize>,
        level_limit: Option<usize>,
    ) -> Result<Vec<TreeEntry>> {
        let base = normalize_prefix_path(path);
        let mut entries = self
            .inner
            .primary()
            .backend
            .tree_directory(path, show_hidden, node_limit, level_limit)
            .await?;

        entries.retain(|e| {
            let name = file_name(&e.path);
            !INTERNAL_NAMES.contains(&name)
        });

        let ctx = match current_required_ctx() {
            Ok(c) => c,
            Err(_) => return Ok(entries),
        };
        let mut seen_paths: HashSet<String> = entries.iter().map(|e| e.path.clone()).collect();
        let mut dir_paths = vec![base.clone()];
        for entry in &entries {
            if entry.info.is_dir {
                let dir = normalize_prefix_path(&entry.path);
                if !dir_paths.iter().any(|p| p == &dir) {
                    dir_paths.push(dir);
                }
            }
        }

        for dir in dir_paths {
            let redirect_meta = match self.inner.meta_store.get_redirect_meta(&dir, &ctx).await {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            for (name, _redirect_entry) in redirect_meta.entries {
                let virtual_path = if dir == "/" {
                    format!("/{}", name)
                } else {
                    format!("{}/{}", dir, name)
                };
                if seen_paths.contains(&virtual_path) {
                    continue;
                }
                let rel_path = if base == "/" {
                    virtual_path.trim_start_matches('/').to_string()
                } else {
                    virtual_path
                        .strip_prefix(&base)
                        .unwrap_or(&virtual_path)
                        .trim_start_matches('/')
                        .to_string()
                };
                let mut extra = HashMap::new();
                extra.insert("redirect".to_string(), Value::Bool(true));
                entries.push(TreeEntry {
                    path: virtual_path.clone(),
                    rel_path,
                    info: FileInfo::new_file(name, 0, 0o644),
                    extra,
                });
                seen_paths.insert(virtual_path);
            }
        }

        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_policy_over_size() {
        let policy = RedirectPolicy::FileOverSizePolicy {
            max_size_mb: 1,
            target: Some(vec!["backup1".to_string()]),
        };
        assert!(policy.matches("/a/big.bin", 2 * 1024 * 1024));
        assert!(!policy.matches("/a/small.txt", 512));
    }

    #[test]
    fn test_file_policy_extension() {
        let policy = RedirectPolicy::FileExtensionPolicy {
            extensions: vec!["(pdf|ppt)".to_string()],
            target: Some(vec!["backup1".to_string()]),
        };
        assert!(policy.matches("/a/doc.pdf", 0));
        assert!(policy.matches("/a/slides.ppt", 0));
        assert!(!policy.matches("/a/text.txt", 0));
    }
}
