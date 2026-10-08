//! Checkpoint construction, validation, publication, and reading.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use tokio::sync::watch;
use tracing::warn;

use crate::core::context::{FsContextInner, FS_CTX};
use crate::core::errors::{Error, Result};
use crate::core::filesystem::FileSystem;
use crate::core::internal_names::is_multiwrite_internal_path;
use crate::multibackend::catch_up::{CheckpointConsumer, CheckpointReadResult, CheckpointSnapshot};
use crate::multibackend::codec::{decode_checkpoint_chunk, encode_checkpoint_chunk, sha256_hex};
use crate::multibackend::gc::MetadataGc;
use crate::multibackend::meta::{MetadataStore, MultiWriteWorker};
use crate::multibackend::model::{
    is_checkpoint_directory_name, CheckpointManifest, CheckpointNode, CheckpointsManifest,
    ChunkDescriptor, DirectoryOperation, FileState, LatestCheckpoint, PartitionsManifest,
    ProtocolStatus, ScopeKey, ScopedSeq, SegmentEventType,
};
use crate::multibackend::provider::MultiWriteProvider;
use crate::multibackend::router::AccountRouter;

const MAX_CHUNK_NODES: usize = 500_000;

/// Periodically builds checkpoints for every Stable account partition.
pub struct CheckpointWorker {
    store: Arc<MetadataStore>,
    builder: CheckpointBuilder,
    gc: MetadataGc,
}
impl CheckpointWorker {
    /// Create a worker over the production primary, store, and provider.
    pub fn new(
        primary: Arc<dyn FileSystem>,
        store: Arc<MetadataStore>,
        provider: Arc<dyn MultiWriteProvider>,
    ) -> Self {
        let builder = CheckpointBuilder::new(primary, store.clone(), provider.clone());
        Self {
            gc: MetadataGc::new(store.clone(), provider),
            builder,
            store,
        }
    }

    /// Run checkpoint rounds at the configured interval until cancellation.
    pub async fn run(&self, interval: Duration, mut cancellation: watch::Receiver<bool>) {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = ticker.tick() => if let Err(error) = self.run_once(interval).await {
                    warn!(error = %error, "multi-write checkpoint discovery failed");
                },
                changed = cancellation.changed() => {
                    if changed.is_err() || *cancellation.borrow() {
                        break;
                    }
                }
            }
        }
    }

    /// Build one checkpoint round while isolating account and scope failures.
    pub async fn run_once(&self, interval: Duration) -> Result<()> {
        let result = self.run_once_inner(interval).await;
        if result.is_err() {
            self.store.record_worker_error(MultiWriteWorker::Checkpoint);
        }
        result
    }

    /// Execute one checkpoint round and return discovery failures.
    async fn run_once_inner(&self, interval: Duration) -> Result<()> {
        if self.store.protocol_status().await? != ProtocolStatus::Stable {
            return Ok(());
        }
        let now = now_ns()?;
        let interval_ns = u64::try_from(interval.as_nanos()).unwrap_or(u64::MAX);
        for account in self.store.initialized_accounts().await? {
            let path = self.store.paths().account_manifest(&account)?.0;
            let manifest: PartitionsManifest = match self.store.read_json(&path).await {
                Ok(manifest) => manifest,
                Err(Error::NotFound(_)) => continue,
                Err(error) => {
                    self.store.record_worker_error(MultiWriteWorker::Checkpoint);
                    warn!(account = %account, error = %error,
                        "multi-write checkpoint account metadata unavailable");
                    continue;
                }
            };
            if let Err(error) = manifest.validate() {
                self.store.record_worker_error(MultiWriteWorker::Checkpoint);
                warn!(account = %account, error = %error,
                    "multi-write checkpoint account metadata invalid");
                continue;
            }
            for (&partition_id, partition) in &manifest.partitions {
                if partition.state != crate::multibackend::model::PartitionState::Stable {
                    continue;
                }
                let scope = ScopeKey {
                    account_id: account.clone(),
                    partition_id,
                    epoch: manifest.epoch,
                };
                let pointer_path = self
                    .store
                    .paths()
                    .checkpoints_manifest(&account, partition_id)?
                    .0;
                let checkpoints: CheckpointsManifest =
                    match self.store.read_json(&pointer_path).await {
                        Ok(checkpoints) => checkpoints,
                        Err(Error::NotFound(_)) => continue,
                        Err(error) => {
                            self.store.record_worker_error(MultiWriteWorker::Checkpoint);
                            warn!(account = %account, partition = partition_id, error = %error,
                            "multi-write checkpoint pointer unavailable");
                            continue;
                        }
                    };
                if let Err(error) = checkpoints.validate() {
                    self.store.record_worker_error(MultiWriteWorker::Checkpoint);
                    warn!(account = %account, partition = partition_id, error = %error,
                        "multi-write checkpoint pointer invalid");
                    continue;
                }
                if checkpoints
                    .latest_checkpoint
                    .as_ref()
                    .is_some_and(|latest| now < latest.updated_at_ns.saturating_add(interval_ns))
                {
                    continue;
                }
                match self.builder.build_scope(&scope).await {
                    Ok(CheckpointBuildResult::Published(_)) => {
                        if let Err(error) = self.gc.collect_scope(&scope).await {
                            warn!(account = %account, partition = partition_id, error = %error,
                                "multi-write metadata garbage collection failed");
                        }
                    }
                    Ok(_) => {}
                    Err(Error::NotFound(_)) => {}
                    Err(error) => {
                        self.store.record_worker_error(MultiWriteWorker::Checkpoint);
                        warn!(account = %account, partition = partition_id, error = %error,
                            "multi-write checkpoint build failed");
                    }
                }
            }
        }
        Ok(())
    }
}

/// Result of one bounded checkpoint build attempt.
pub enum CheckpointBuildResult {
    /// No new sealed sequence exists for this scope.
    NoSealedChanges,
    /// A validated checkpoint became the latest publication.
    Published(LatestCheckpoint),
    /// Another builder published first and this candidate was discarded.
    Retry,
}

/// Builds immutable checkpoints from sealed records and current primary state.
pub struct CheckpointBuilder {
    primary: Arc<dyn FileSystem>,
    store: Arc<MetadataStore>,
    provider: Arc<dyn MultiWriteProvider>,
    router: AccountRouter,
}
impl CheckpointBuilder {
    /// Create a builder over the encrypted primary and raw metadata store.
    pub fn new(
        primary: Arc<dyn FileSystem>,
        store: Arc<MetadataStore>,
        provider: Arc<dyn MultiWriteProvider>,
    ) -> Self {
        Self {
            primary,
            router: AccountRouter::new(store.clone()),
            store,
            provider,
        }
    }

    /// Build and compare-before-publish one Stable partition checkpoint.
    pub async fn build_scope(&self, scope: &ScopeKey) -> Result<CheckpointBuildResult> {
        scope.validate()?;
        let pointer_path = self
            .store
            .paths()
            .checkpoints_manifest(&scope.account_id, scope.partition_id)?
            .0;
        let lease = self
            .store
            .pathlock_manager()
            .acquire_exact(
                &pointer_path,
       self.store.pathlock_manager().default_lock_timeout().max(Duration::from_secs(5)),
                None,
            )
            .await?;
        let read_pointer = async {
            let start: CheckpointsManifest = self.store.read_json(&pointer_path).await?;
            start.validate()?;
            Ok(start)
        }
        .await;
        let release = self
            .store
            .pathlock_manager()
            .release(&lease)
            .await
            .map_err(Error::from);
        let start = finish_with_release(read_pointer, release)?;
        let old_states = match &start.latest_checkpoint {
            Some(latest) => read_published(self.store.as_ref(), scope, latest).await?,
            None => Vec::new(),
        };

        let segments = self.provider.read_manifest(scope).await?;
        segments.validate()?;
        let Some(tail) = segments
            .segments
            .iter()
            .filter_map(|segment| segment.segment_to_seq)
            .last()
        else {
            return Ok(CheckpointBuildResult::NoSealedChanges);
        };
        let old_to = start
            .latest_checkpoint
            .as_ref()
            .map_or(0, |latest| latest.checkpoint_to_seq);
        if tail <= old_to {
            return Ok(CheckpointBuildResult::NoSealedChanges);
        }
        let records = self
            .provider
            .read_committed_range(scope, old_to + 1, tail + 1)
            .await?;
        let mut states = old_states
            .into_iter()
            .map(|state| (state.path.clone(), state))
            .collect::<BTreeMap<_, _>>();
        for record in records {
            match record.event_type {
                SegmentEventType::Write | SegmentEventType::Remove => {
                    let backend_path = self
                        .store
                        .paths()
                        .backend_path(&scope.account_id, &record.path)?;
                    let deleted = match FS_CTX
                        .scope(
                            Arc::new(
                                FsContextInner::new(&scope.account_id)
                                    .with_bypass_cache(true)
                                    .with_auto_pathlock_disabled(),
                            ),
                            self.primary.stat(&backend_path),
                        )
                        .await
                    {
                        Ok(_) => None,
                        Err(Error::NotFound(_)) => Some(true),
                        Err(error) => return Err(error),
                    };
                    states.insert(
                        record.path.clone(),
                        FileState {
                            path: record.path,
                            latest_seq: ScopedSeq {
                                scope: scope.clone(),
                                seq: record.seq,
                            },
                            deleted,
                        },
                    );
                }
                SegmentEventType::RemoveTree | SegmentEventType::MoveTree => {
                    let mut prefixes = vec![record.path.clone()];
                    if let Some(destination) = record.destination_path {
                        prefixes.push(destination);
                    }
                    for prefix in prefixes {
                        states.retain(|path, _| !is_path_within(path, &prefix));
                        for path in self.scan_prefix(scope, &prefix).await? {
                            states.insert(
                                path.clone(),
                                FileState {
                                    path,
                                    latest_seq: ScopedSeq {
                                        scope: scope.clone(),
                                        seq: record.seq,
                                    },
                                    deleted: None,
                                },
                            );
                        }
                    }
                }
            }
        }
        let min_synced = segments
            .backend_states
            .values()
            .map(|state| state.synced_seq)
            .min();
        states.retain(|_, state| {
            state.deleted.is_none()
                || min_synced.is_some_and(|synced| state.latest_seq.seq >= synced)
        });

        let created_at_ns = now_ns()?;
        let (root, chunks) = encode_states(scope, states.into_values().collect(), tail)?;
        let checksum = tree_checksum(&root)?;
        let manifest = CheckpointManifest {
            version: 1,
            partition_id: scope.partition_id,
            epoch: scope.epoch,
            checkpoint_from_seq: old_to + 1,
            checkpoint_to_seq: tail,
            created_at_ns,
            root,
            checksum,
        };
        manifest.validate_for_scope(scope)?;
        let manifest_bytes = serde_json::to_vec(&manifest)?;
        let candidate = format!("cp-{tail}-{}/", &sha256_hex(&manifest_bytes)[..12]);
        let directory = self
            .store
            .paths()
            .checkpoints_dir(&scope.account_id, scope.partition_id)?
            .0;
        let candidate_path = format!("{directory}/{}", candidate.trim_end_matches('/'));
        let candidate_lease = self
            .store
            .pathlock_manager()
            .acquire_exact(
                &candidate_path,
                self.store.pathlock_manager().default_lock_timeout().max(Duration::from_secs(5)),
                None,
            )
            .await?;
        let operation = async {
            for (path, bytes) in &chunks {
                self.store
                    .write_bytes(&format!("{directory}/{candidate}{path}"), bytes)
                    .await?;
            }
            self.store
                .write_bytes(
                    &format!("{directory}/{candidate}manifest.json"),
                    &manifest_bytes,
                )
                .await?;
            let manifest = read_manifest(self.store.as_ref(), scope, &candidate).await?;
            let verified = read_chunks(self.store.as_ref(), scope, &manifest, &candidate).await?;
            if tree_checksum(&manifest.root)? != manifest.checksum
                || verified.iter().any(|state| state.latest_seq.seq > tail)
            {
                return Err(Error::Serialization(
                    "checkpoint candidate verification failed".into(),
                ));
            }
            let latest = LatestCheckpoint {
                path: candidate,
                checkpoint_to_seq: tail,
                updated_at_ns: created_at_ns,
            };
            let lease = self
                .store
                .pathlock_manager()
                .acquire_exact(
                    &pointer_path,
           self.store.pathlock_manager().default_lock_timeout().max(Duration::from_secs(5)),
                    None,
                )
                .await?;
            let publish = async {
                let current: CheckpointsManifest = self.store.read_json(&pointer_path).await?;
                if current.latest_checkpoint != start.latest_checkpoint {
                    return Ok(CheckpointBuildResult::Retry);
                }
                self.store
                    .publish_json(
                        &pointer_path,
                        &CheckpointsManifest {
                            version: 1,
                            latest_checkpoint: Some(latest.clone()),
                        },
                        CheckpointsManifest::validate,
                    )
                    .await?;
                Ok(CheckpointBuildResult::Published(latest))
            }
            .await;
            let release = self
                .store
                .pathlock_manager()
                .release(&lease)
                .await
                .map_err(Error::from);
            finish_with_release(publish, release)
        }
        .await;
        let release = self
            .store
            .pathlock_manager()
            .release(&candidate_lease)
            .await
            .map_err(Error::from);
        finish_with_release(operation, release)
    }

    /// Scan one current primary prefix and retain paths owned by this scope.
    async fn scan_prefix(&self, scope: &ScopeKey, logical_prefix: &str) -> Result<Vec<String>> {
        let backend_prefix = self
            .store
            .paths()
            .backend_path(&scope.account_id, logical_prefix)?;
        let context = Arc::new(
            FsContextInner::new(&scope.account_id)
                .with_bypass_cache(true)
                .with_auto_pathlock_disabled(),
        );
        let paths = FS_CTX
            .scope(context, async {
                let root = match self.primary.stat(&backend_prefix).await {
                    Ok(root) => root,
                    Err(Error::NotFound(_)) => return Ok(Vec::new()),
                    Err(error) => return Err(error),
                };
                let mut paths = vec![backend_prefix.clone()];
                if root.is_dir {
                    paths.extend(
                        self.primary
                            .tree_directory(
                                &backend_prefix,
                                true,
                                None,
                                None,
                                None,
                                None,
                                None,
                                false,
                            )
                            .await?
                            .into_iter()
                            .map(|entry| entry.path),
                    );
                }
                Ok(paths)
            })
            .await?;
        let mut selected = Vec::new();
        for path in paths {
            let logical = format!("/local{path}");
            if !is_multiwrite_internal_path(&logical)
                && self.router.route(&scope.account_id, &logical).await?.scope == *scope
            {
                selected.push(logical);
            }
        }
        selected.sort();
        selected.dedup();
        Ok(selected)
    }
}

/// Reads complete validated checkpoint snapshots under the publication lease.
pub struct CheckpointReader {
    store: Arc<MetadataStore>,
}
impl CheckpointReader {
    /// Create a reader over the raw metadata store.
    pub fn new(store: Arc<MetadataStore>) -> Self {
        Self { store }
    }
}

#[async_trait]
impl CheckpointConsumer for CheckpointReader {
    /// Read the latest checkpoint or return a typed retry for immutable races.
    async fn read_latest(
        &self,
        scope: &ScopeKey,
        synced_seq: u64,
        tail_seq: u64,
    ) -> Result<CheckpointReadResult> {
        let pointer_path = self
            .store
            .paths()
            .checkpoints_manifest(&scope.account_id, scope.partition_id)?
            .0;
        let lease = self
            .store
            .pathlock_manager()
            .acquire_exact(
                &pointer_path,
                self.store.pathlock_manager().default_lock_timeout().max(Duration::from_secs(5)),
                None,
            )
            .await?;
        let pointer = async {
            let pointer: CheckpointsManifest = self.store.read_json(&pointer_path).await?;
            pointer.validate()?;
            let Some(latest) = pointer.latest_checkpoint else {
                return Ok(None);
            };
            if latest.checkpoint_to_seq <= synced_seq || latest.checkpoint_to_seq > tail_seq {
                return Ok(None);
            }
            Ok(Some(latest))
        }
        .await;
        let release = self
            .store
            .pathlock_manager()
            .release(&lease)
            .await
            .map_err(Error::from);
        let latest = finish_with_release(pointer, release)?;
        let Some(latest) = latest else {
            return Ok(CheckpointReadResult::NoCheckpoint);
        };
        let states = match read_published(self.store.as_ref(), scope, &latest).await {
            Ok(states) => states,
            Err(Error::NotFound(_)) | Err(Error::Serialization(_)) => {
                return Ok(CheckpointReadResult::RetryLatestCheckpoint)
            }
            Err(error) => return Err(error),
        };
        let account_manifest_path = self.store.paths().account_manifest(&scope.account_id)?.0;
        let account_manifest: PartitionsManifest =
            self.store.read_json(&account_manifest_path).await?;
        account_manifest.validate()?;
        let mut directory_prefixes = BTreeSet::new();
        for event in account_manifest.directory_events {
            if event.positions.iter().any(|position| {
                position.partition_id == scope.partition_id
                    && position.epoch == scope.epoch
                    && position.seq <= latest.checkpoint_to_seq
            }) {
                directory_prefixes.insert(event.source_path);
                if event.operation == DirectoryOperation::MoveTree {
                    directory_prefixes.insert(event.destination_path.ok_or_else(|| {
                        Error::Serialization("move directory event lacks destination".into())
                    })?);
                }
            }
        }
        Ok(CheckpointReadResult::Snapshot(CheckpointSnapshot {
            checkpoint_to_seq: latest.checkpoint_to_seq,
            file_states: states,
            directory_prefixes: directory_prefixes.into_iter().collect(),
        }))
    }
}

/// Encode a deterministic checkpoint tree within the production node limit.
fn encode_states(
    scope: &ScopeKey,
    mut states: Vec<FileState>,
    checkpoint_to_seq: u64,
) -> Result<(ChunkDescriptor, Vec<(String, Vec<u8>)>)> {
    for state in &states {
        state.validate(scope)?;
    }
    states.sort_by(|left, right| left.path.cmp(&right.path));
    if states.windows(2).any(|pair| pair[0].path == pair[1].path) {
        return Err(Error::Serialization(
            "checkpoint contains duplicate paths".into(),
        ));
    }
    let root_path = format!("/local/{}", scope.account_id);
    let mut node = CheckpointNode {
        path_fragment: String::new(),
        latest_seq: None,
        deleted: None,
        children: Vec::new(),
    };
    for state in states {
        if !is_canonical_account_path(&state.path, &root_path) {
            return Err(Error::Serialization(
                "checkpoint path crosses account root".into(),
            ));
        }
        let relative = state
            .path
            .strip_prefix(&root_path)
            .ok_or_else(|| Error::Serialization("checkpoint path crosses account root".into()))?;
        insert_state(&mut node, relative, &state)?;
    }
    encode_checkpoint_tree(node, &root_path, checkpoint_to_seq)
}

/// Encode one preconstructed radix tree through the production chunk splitter.
fn encode_checkpoint_tree(
    mut node: CheckpointNode,
    root_path: &str,
    checkpoint_to_seq: u64,
) -> Result<(ChunkDescriptor, Vec<(String, Vec<u8>)>)> {
    node.validate()?;
    let mut files = Vec::new();
    let mut index = 0;
    let root = if checkpoint_node_count(&node) <= MAX_CHUNK_NODES {
        encode_node(
            node,
            &root_path,
            checkpoint_to_seq,
            true,
            &mut index,
            &mut files,
        )?
    } else {
        let children = std::mem::take(&mut node.children);
        let mut descriptor = encode_node(
            node,
            &root_path,
            checkpoint_to_seq,
            true,
            &mut index,
            &mut files,
        )?;
        partition_children(
            children,
            &root_path,
            checkpoint_to_seq,
            &mut index,
            &mut files,
            &mut descriptor.chunks,
        )?;
        descriptor
    };
    Ok((root, files))
}

/// Insert one file state into the account-relative component radix tree.
fn insert_state(root: &mut CheckpointNode, relative: &str, state: &FileState) -> Result<()> {
    let mut node = root;
    let mut remaining = relative;
    while !remaining.is_empty() {
        let Some(index) = node
            .children
            .iter()
            .position(|child| common_prefix_len(&child.path_fragment, remaining) > 0)
        else {
            node.children.push(CheckpointNode {
                path_fragment: remaining.to_string(),
                latest_seq: Some(state.latest_seq.seq),
                deleted: state.deleted,
                children: Vec::new(),
            });
            node.children
                .sort_by(|left, right| left.path_fragment.cmp(&right.path_fragment));
            return Ok(());
        };
        let common = common_prefix_len(&node.children[index].path_fragment, remaining);
        if common == node.children[index].path_fragment.len() {
            remaining = &remaining[common..];
            node = &mut node.children[index];
        } else {
            let mut old = node.children.remove(index);
            let suffix = old.path_fragment.split_off(common);
            let prefix = std::mem::replace(&mut old.path_fragment, suffix);
            let mut parent = CheckpointNode {
                path_fragment: prefix,
                latest_seq: None,
                deleted: None,
                children: vec![old],
            };
            remaining = &remaining[common..];
            if remaining.is_empty() {
                parent.latest_seq = Some(state.latest_seq.seq);
                parent.deleted = state.deleted;
            } else {
                parent.children.push(CheckpointNode {
                    path_fragment: remaining.to_string(),
                    latest_seq: Some(state.latest_seq.seq),
                    deleted: state.deleted,
                    children: Vec::new(),
                });
                parent
                    .children
                    .sort_by(|left, right| left.path_fragment.cmp(&right.path_fragment));
            }
            node.children.insert(index, parent);
            return Ok(());
        }
    }
    if node.latest_seq.is_some() {
        return Err(Error::Serialization(
            "checkpoint contains duplicate paths".into(),
        ));
    }
    node.latest_seq = Some(state.latest_seq.seq);
    node.deleted = state.deleted;
    Ok(())
}

/// Return the longest UTF-8-safe common prefix length.
fn common_prefix_len(left: &str, right: &str) -> usize {
    left.char_indices()
        .zip(right.char_indices())
        .take_while(|((_, left), (_, right))| left == right)
        .last()
        .map_or(0, |((index, ch), _)| index + ch.len_utf8())
}

/// Split ordered direct child ranges into bounded immutable chunks.
fn partition_children(
    children: Vec<CheckpointNode>,
    parent_path: &str,
    checkpoint_to_seq: u64,
    index: &mut usize,
    files: &mut Vec<(String, Vec<u8>)>,
    descriptors: &mut Vec<ChunkDescriptor>,
) -> Result<()> {
    let mut group = Vec::new();
    let mut group_nodes = 1;
    for mut child in children {
        let child_nodes = checkpoint_node_count(&child);
        if child_nodes + 1 > MAX_CHUNK_NODES {
            flush_child_group(
                &mut group,
                parent_path,
                checkpoint_to_seq,
                index,
                files,
                descriptors,
            )?;
            let fragment = child.path_fragment.clone();
            let mut grandchildren = std::mem::take(&mut child.children);
            if child.latest_seq.is_some() {
                descriptors.push(encode_node(
                    child,
                    parent_path,
                    checkpoint_to_seq,
                    false,
                    index,
                    files,
                )?);
            }
            for grandchild in &mut grandchildren {
                grandchild.path_fragment.insert_str(0, &fragment);
            }
            partition_children(
                grandchildren,
                parent_path,
                checkpoint_to_seq,
                index,
                files,
                descriptors,
            )?;
        } else {
            if group_nodes + child_nodes > MAX_CHUNK_NODES {
                flush_child_group(
                    &mut group,
                    parent_path,
                    checkpoint_to_seq,
                    index,
                    files,
                    descriptors,
                )?;
                group_nodes = 1;
            }
            group_nodes += child_nodes;
            group.push(child);
        }
    }
    flush_child_group(
        &mut group,
        parent_path,
        checkpoint_to_seq,
        index,
        files,
        descriptors,
    )
}

/// Encode one non-empty direct child range as a synthetic-root chunk.
fn flush_child_group(
    children: &mut Vec<CheckpointNode>,
    parent_path: &str,
    checkpoint_to_seq: u64,
    index: &mut usize,
    files: &mut Vec<(String, Vec<u8>)>,
    descriptors: &mut Vec<ChunkDescriptor>,
) -> Result<()> {
    if children.is_empty() {
        return Ok(());
    }
    let node = CheckpointNode {
        path_fragment: String::new(),
        latest_seq: None,
        deleted: None,
        children: std::mem::take(children),
    };
    descriptors.push(encode_node(
        node,
        parent_path,
        checkpoint_to_seq,
        false,
        index,
        files,
    )?);
    Ok(())
}

/// Encode one bounded radix subtree and derive its exact path range.
fn encode_node(
    node: CheckpointNode,
    root_path: &str,
    checkpoint_to_seq: u64,
    root: bool,
    index: &mut usize,
    files: &mut Vec<(String, Vec<u8>)>,
) -> Result<ChunkDescriptor> {
    let (count, first_path, last_path) = checkpoint_node_range(&node, root_path);
    let bytes = encode_checkpoint_chunk(&node)?;
    let checksum = sha256_hex(&bytes);
    let file = format!("chunk-{checkpoint_to_seq}-{index:06}-{checksum}.ovcp");
    *index += 1;
    files.push((file.clone(), bytes));
    Ok(ChunkDescriptor {
        file,
        root_path: root_path.to_string(),
        first_path: (!root).then_some(first_path).flatten(),
        last_path: (!root).then_some(last_path).flatten(),
        file_state_count: count as u32,
        checksum,
        chunks: Vec::new(),
    })
}

/// Count nodes in one radix subtree.
fn checkpoint_node_count(root: &CheckpointNode) -> usize {
    let mut nodes = vec![root];
    let mut count = 0;
    while let Some(node) = nodes.pop() {
        count += 1;
        nodes.extend(&node.children);
    }
    count
}

/// Return the state count and ordered path bounds encoded in one chunk.
fn checkpoint_node_range(
    root: &CheckpointNode,
    root_path: &str,
) -> (usize, Option<String>, Option<String>) {
    let mut paths = Vec::new();
    collect_node_paths(root, root_path, &mut paths);
    (paths.len(), paths.first().cloned(), paths.last().cloned())
}

/// Collect state paths from one radix subtree in canonical order.
fn collect_node_paths(node: &CheckpointNode, parent: &str, paths: &mut Vec<String>) {
    let path = format!("{parent}{}", node.path_fragment);
    if node.latest_seq.is_some() {
        paths.push(path.clone());
    }
    for child in &node.children {
        collect_node_paths(child, &path, paths);
    }
}

/// Read and validate one published checkpoint reference.
async fn read_published(
    store: &MetadataStore,
    scope: &ScopeKey,
    latest: &LatestCheckpoint,
) -> Result<Vec<FileState>> {
    let manifest = read_manifest(store, scope, &latest.path).await?;
    if manifest.checkpoint_to_seq != latest.checkpoint_to_seq {
        return Err(Error::Serialization(
            "checkpoint pointer sequence mismatch".into(),
        ));
    }
    read_chunks(store, scope, &manifest, &latest.path).await
}

/// Read and validate one immutable checkpoint manifest.
async fn read_manifest(
    store: &MetadataStore,
    scope: &ScopeKey,
    candidate: &str,
) -> Result<CheckpointManifest> {
    let directory = checkpoint_directory(store, scope, candidate)?;
    let manifest: CheckpointManifest = store
        .read_json(&format!("{directory}/manifest.json"))
        .await?;
    manifest.validate_for_scope(scope)?;
    if tree_checksum(&manifest.root)? != manifest.checksum {
        return Err(Error::Serialization(
            "checkpoint tree checksum mismatch".into(),
        ));
    }
    Ok(manifest)
}

/// Read every referenced chunk and reconstruct sorted file states.
async fn read_chunks(
    store: &MetadataStore,
    scope: &ScopeKey,
    manifest: &CheckpointManifest,
    candidate: &str,
) -> Result<Vec<FileState>> {
    let directory = checkpoint_directory(store, scope, candidate)?;
    let account_root = format!("/local/{}", scope.account_id);
    let mut descriptors = vec![(&manifest.root, true)];
    let mut states = Vec::new();
    while let Some((descriptor, root)) = descriptors.pop() {
        if descriptor.root_path != account_root {
            return Err(Error::Serialization(
                "checkpoint descriptor root mismatch".into(),
            ));
        }
        let bytes = store
            .read_bytes(&format!("{directory}/{}", descriptor.file))
            .await?;
        if sha256_hex(&bytes) != descriptor.checksum {
            return Err(Error::Serialization(
                "checkpoint chunk checksum mismatch".into(),
            ));
        }
        let node = decode_checkpoint_chunk(&bytes)?;
        if encode_checkpoint_chunk(&node)? != bytes {
            return Err(Error::Serialization(
                "checkpoint chunk is not canonical".into(),
            ));
        }
        let mut chunk_states = Vec::new();
        flatten_node(scope, &node, &descriptor.root_path, true, &mut chunk_states)?;
        if chunk_states.len() != descriptor.file_state_count as usize {
            return Err(Error::Serialization(
                "checkpoint chunk state count mismatch".into(),
            ));
        }
        if chunk_states
            .windows(2)
            .any(|pair| pair[0].path >= pair[1].path)
            || chunk_states
                .iter()
                .any(|state| !is_canonical_account_path(&state.path, &account_root))
        {
            return Err(Error::Serialization(
                "checkpoint chunk paths are invalid or unordered".into(),
            ));
        }
        if !root
            && (chunk_states.first().map(|state| &state.path) != descriptor.first_path.as_ref()
                || chunk_states.last().map(|state| &state.path) != descriptor.last_path.as_ref())
        {
            return Err(Error::Serialization(
                "checkpoint chunk range mismatch".into(),
            ));
        }
        states.extend(chunk_states);
        descriptors.extend(descriptor.chunks.iter().rev().map(|child| (child, false)));
    }
    if states.windows(2).any(|pair| pair[0].path >= pair[1].path) {
        return Err(Error::Serialization(
            "checkpoint paths are not strictly ordered".into(),
        ));
    }
    Ok(states)
}

/// Flatten one decoded checkpoint node into validated file states.
fn flatten_node(
    scope: &ScopeKey,
    node: &CheckpointNode,
    parent: &str,
    root: bool,
    states: &mut Vec<FileState>,
) -> Result<()> {
    if (!root || !node.path_fragment.is_empty()) && !valid_path_fragment(&node.path_fragment) {
        return Err(Error::Serialization(
            "checkpoint path fragment is not relative".into(),
        ));
    }
    let path = format!("{parent}{}", node.path_fragment);
    if let Some(seq) = node.latest_seq {
        let state = FileState {
            path: path.clone(),
            latest_seq: ScopedSeq {
                scope: scope.clone(),
                seq,
            },
            deleted: node.deleted,
        };
        state.validate(scope)?;
        states.push(state);
    }
    for child in &node.children {
        flatten_node(scope, child, &path, false, states)?;
    }
    Ok(())
}

/// Resolve one isolated checkpoint directory from its relative latest pointer.
fn checkpoint_directory(
    store: &MetadataStore,
    scope: &ScopeKey,
    candidate: &str,
) -> Result<String> {
    let name = candidate.strip_suffix('/').ok_or_else(|| {
        Error::Serialization("checkpoint pointer must reference a directory".into())
    })?;
    if !is_checkpoint_directory_name(name) {
        return Err(Error::Serialization(
            "checkpoint pointer directory is invalid".into(),
        ));
    }
    let root = store
        .paths()
        .checkpoints_dir(&scope.account_id, scope.partition_id)?
        .0;
    Ok(format!("{root}/{name}"))
}

/// Return whether one radix edge is a non-empty relative fragment.
fn valid_path_fragment(fragment: &str) -> bool {
    !fragment.is_empty() && !fragment.contains('\0')
}

/// Return whether a path is canonical and belongs to one account root.
fn is_canonical_account_path(path: &str, root: &str) -> bool {
    is_path_within(path, root)
        && !path.ends_with('/')
        && path
            .split('/')
            .skip(1)
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

/// Hash the canonical serialized descriptor tree.
fn tree_checksum(root: &ChunkDescriptor) -> Result<String> {
    Ok(sha256_hex(&serde_json::to_vec(root)?))
}

/// Return whether a path equals a prefix or is one of its descendants.
fn is_path_within(path: &str, prefix: &str) -> bool {
    path == prefix
        || path
            .strip_prefix(prefix)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Return the current Unix timestamp in nanoseconds.
fn now_ns() -> Result<u64> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| Error::internal(error.to_string()))?
        .as_nanos();
    u64::try_from(nanos).map_err(|_| Error::internal("checkpoint timestamp overflow"))
}

/// Preserve an operation result while requiring lease release to succeed.
fn finish_with_release<T>(operation: Result<T>, release: Result<()>) -> Result<T> {
    match (operation, release) {
        (Ok(value), Ok(())) => Ok(value),
        (Err(error), _) => Err(error),
        (Ok(_), Err(error)) => Err(error),
    }
}
