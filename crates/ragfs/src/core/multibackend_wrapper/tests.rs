use std::sync::Arc;

use serde_json::Value;

use super::*;
use crate::core::context::FsContextInner;
use crate::plugins::memfs::MemFileSystem;

/// Create the shared test context used by multi-write tests.
fn test_ctx() -> FsContext {
    Arc::new(FsContextInner::new("acct".to_string()))
}

/// Create a sync multi-write filesystem with one memfs backup.
fn test_multiwrite_fs(redirects: Vec<RedirectPolicy>) -> MultiWriteWrappedFS {
    let primary: Arc<dyn FileSystem> = Arc::new(MemFileSystem::new());
    let backup: Arc<dyn FileSystem> = Arc::new(MemFileSystem::new());
    let builder = MultiWriteWrappedFS::builder(primary)
        .with_backups(vec![BackendEntry {
            name: "backup1".to_string(),
            role: BackendRole::Backup,
            backend: backup,
            operations: Vec::new(),
            excludes: Vec::new(),
        }])
        .sync_mode(SyncMode::Sync {
            ack_count: 1,
            timeout_ms: 0,
        });

    if redirects.is_empty() {
        builder
    } else {
        builder.with_redirects(redirects)
    }
    .build()
    .unwrap()
}

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

#[tokio::test]
async fn test_read_dir_redirect_entries_use_target_stat() {
    let fs = test_multiwrite_fs(vec![RedirectPolicy::FileExtensionPolicy {
        extensions: vec!["\\.pdf$".to_string()],
        target: Some(vec!["backup1".to_string()]),
    }]);
    let ctx = test_ctx();

    FS_CTX
        .scope(ctx, async {
            fs.ensure_parent_dirs("/local/acct/docs/report.pdf", 0o755)
                .await?;
            fs.write(
                "/local/acct/docs/report.pdf",
                b"pdf body",
                0,
                WriteFlag::Create,
            )
            .await?;

            let entries = fs.read_dir("/local/acct/docs").await?;
            let report = entries
                .iter()
                .find(|entry| entry.name == "report.pdf")
                .expect("redirected file should be visible in read_dir");
            assert_eq!(report.size, 8);
            assert_eq!(report.mode, 0o644);
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_cross_dir_rename_does_not_create_target_write_log() {
    let fs = test_multiwrite_fs(Vec::new());
    let ctx = test_ctx();

    FS_CTX
        .scope(ctx.clone(), async {
            fs.ensure_parent_dirs("/local/acct/src/file.txt", 0o755)
                .await?;
            fs.ensure_parent_dirs("/local/acct/dst/file.txt", 0o755)
                .await?;
            fs.write(
                "/local/acct/src/file.txt",
                b"rename me",
                0,
                WriteFlag::Create,
            )
            .await?;
            fs.rename("/local/acct/src/file.txt", "/local/acct/dst/file.txt")
                .await?;

            let target_log = fs
                .inner
                .meta_store
                .get_sync_log_meta("/local/acct/dst", &ctx)
                .await?;
            assert!(
                !target_log.entries.contains_key("file.txt"),
                "target directory must not record a fake write for cross-dir rename"
            );
            let source_log = fs
                .inner
                .meta_store
                .get_sync_log_meta("/local/acct/src", &ctx)
                .await?;
            let rename_entry = source_log
                .entries
                .get("file.txt")
                .expect("source directory should record rename command");
            assert!(matches!(
                &rename_entry.op,
                SyncOp::Rename { to } if to == "/local/acct/dst/file.txt"
            ));
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_rename_redirected_file_renames_original_redirect_target() {
    let primary: Arc<dyn FileSystem> = Arc::new(MemFileSystem::new());
    let backup: Arc<dyn FileSystem> = Arc::new(MemFileSystem::new());
    let backup_handle = backup.clone();
    let fs = MultiWriteWrappedFS::builder(primary)
        .with_backups(vec![BackendEntry {
            name: "backup1".to_string(),
            role: BackendRole::Backup,
            backend: backup,
            operations: vec![OperationItemConfig {
                operation: "read".to_string(),
                priority: 1,
            }],
            excludes: Vec::new(),
        }])
        .with_redirects(vec![RedirectPolicy::FileExtensionPolicy {
            extensions: vec!["\\.pdf$".to_string()],
            target: Some(vec!["backup1".to_string()]),
        }])
        .sync_mode(SyncMode::Sync {
            ack_count: 1,
            timeout_ms: 0,
        })
        .build()
        .unwrap();
    let ctx = test_ctx();

    FS_CTX
        .scope(ctx, async {
            fs.ensure_parent_dirs("/local/acct/src/a.pdf", 0o755)
                .await?;
            fs.ensure_parent_dirs("/local/acct/dst/b.pdf", 0o755)
                .await?;
            fs.write(
                "/local/acct/src/a.pdf",
                b"pdf payload",
                0,
                WriteFlag::Create,
            )
            .await?;
            assert!(backup_handle.exists("/local/acct/src/a.pdf").await);

            fs.rename("/local/acct/src/a.pdf", "/local/acct/dst/b.pdf")
                .await?;

            assert!(
                backup_handle.exists("/local/acct/dst/b.pdf").await,
                "redirect target backend should receive rename fanout"
            );
            assert!(
                !backup_handle.exists("/local/acct/src/a.pdf").await,
                "redirect target backend should not retain the old path"
            );
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_sync_log_write_entry_does_not_embed_large_payload() {
    let fs = test_multiwrite_fs(Vec::new());
    let ctx = test_ctx();
    let payload = vec![b'x'; 256 * 1024];

    FS_CTX
        .scope(ctx.clone(), async {
            fs.ensure_parent_dirs("/local/acct/docs/large.bin", 0o755)
                .await?;
            fs.write("/local/acct/docs/large.bin", &payload, 0, WriteFlag::Create)
                .await?;

            let sync_log = fs
                .inner
                .meta_store
                .get_sync_log_meta("/local/acct/docs", &ctx)
                .await?;
            let encoded = serde_json::to_vec(&sync_log)?;
            assert!(
                encoded.len() < 16 * 1024,
                "sync log should stay metadata-sized, got {} bytes",
                encoded.len()
            );
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_sync_log_truncate_entry_does_not_embed_snapshot() {
    let fs = test_multiwrite_fs(Vec::new());
    let ctx = test_ctx();
    let payload = vec![b'y'; 256 * 1024];

    FS_CTX
        .scope(ctx.clone(), async {
            fs.ensure_parent_dirs("/local/acct/docs/large.bin", 0o755)
                .await?;
            fs.write("/local/acct/docs/large.bin", &payload, 0, WriteFlag::Create)
                .await?;
            fs.truncate("/local/acct/docs/large.bin", 128).await?;

            let sync_log = fs
                .inner
                .meta_store
                .get_sync_log_meta("/local/acct/docs", &ctx)
                .await?;
            let encoded = serde_json::to_vec(&sync_log)?;
            assert!(
                encoded.len() < 16 * 1024,
                "truncate sync log should stay metadata-sized, got {} bytes",
                encoded.len()
            );
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_fanout_sync_returns_structured_quorum_error() {
    let fs = test_multiwrite_fs(Vec::new());
    let ctx = test_ctx();
    let targets = fs.inner.write_targets("/local/acct/docs/fail.txt", 0);

    let err = Inner::fanout_sync(
        &fs.inner,
        "/local/acct/docs/fail.txt",
        &targets,
        &ctx,
        boxed_write_op(|_backup| async { Err(Error::not_found("/backup/fail.txt")) }),
    )
    .await
    .expect_err("sync fanout should surface structured quorum failure");

    match err {
        Error::SyncWriteQuorum {
            succeeded,
            required,
            attempted,
            failures,
        } => {
            assert_eq!(succeeded, 0);
            assert_eq!(required, 1);
            assert_eq!(attempted, 1);
            assert_eq!(failures.len(), 1);
            assert_eq!(failures[0].backend, "backup1");
            assert_eq!(failures[0].kind, "not_found");
            assert_eq!(failures[0].message, "not found: /backup/fail.txt");
        }
        other => panic!("unexpected error: {other}"),
    }
}

#[tokio::test]
async fn test_read_route_metrics_capture_backup_and_cache_hits() {
    let primary: Arc<dyn FileSystem> = Arc::new(MemFileSystem::new());
    let backup: Arc<dyn FileSystem> = Arc::new(MemFileSystem::new());
    let backup_handle = backup.clone();
    let fs = MultiWriteWrappedFS::builder(primary)
        .with_backups(vec![BackendEntry {
            name: "backup1".to_string(),
            role: BackendRole::Backup,
            backend: backup,
            operations: vec![OperationItemConfig {
                operation: "read".to_string(),
                priority: 1,
            }],
            excludes: Vec::new(),
        }])
        .build()
        .unwrap();
    let ctx = test_ctx();

    FS_CTX
        .scope(ctx, async {
            backup_handle
                .ensure_parent_dirs("/local/acct/hot/cache.txt", 0o755)
                .await?;
            backup_handle
                .write("/local/acct/hot/cache.txt", b"hot", 0, WriteFlag::Create)
                .await?;

            assert_eq!(fs.read("/local/acct/hot/cache.txt", 0, 0).await?, b"hot");
            assert_eq!(fs.read("/local/acct/hot/cache.txt", 0, 0).await?, b"hot");

            let metrics = fs.inner.read_route_metrics();
            assert_eq!(metrics.get("backup_hits").and_then(Value::as_u64), Some(1));
            assert_eq!(metrics.get("cache_hits").and_then(Value::as_u64), Some(1));
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_shutdown_drains_background_retry_loop() {
    let fs = test_multiwrite_fs(Vec::new());
    fs.shutdown().await.unwrap();
}

#[tokio::test]
async fn test_system_sync_status_reports_corrupt_redirect_meta() {
    let fs = test_multiwrite_fs(Vec::new());
    let ctx = test_ctx();

    FS_CTX
        .scope(ctx.clone(), async {
            fs.ensure_parent_dirs("/local/acct/docs/a.txt", 0o755)
                .await?;
            fs.write("/local/acct/docs/a.txt", b"hello", 0, WriteFlag::Create)
                .await?;
            fs.inner
                .primary()
                .backend
                .write(
                    "/local/acct/docs/.redirect.json",
                    b"{not-json",
                    0,
                    WriteFlag::Create,
                )
                .await?;

            let err = fs
                .system_sync_status("/local/acct/docs/a.txt")
                .await
                .expect_err("corrupt redirect metadata must be reported");
            assert!(
                err.to_string().contains("redirect")
                    || err.to_string().contains("json")
                    || err.to_string().contains("serialization"),
                "unexpected error: {err}"
            );
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}

#[tokio::test]
async fn test_retry_pending_dir_reports_corrupt_redirect_meta() {
    let fs = test_multiwrite_fs(Vec::new());
    let ctx = test_ctx();

    FS_CTX
        .scope(ctx.clone(), async {
            fs.ensure_parent_dirs("/local/acct/docs/a.txt", 0o755)
                .await?;
            fs.write("/local/acct/docs/a.txt", b"hello", 0, WriteFlag::Create)
                .await?;
            fs.inner
                .primary()
                .backend
                .write(
                    "/local/acct/docs/.redirect.json",
                    b"{not-json",
                    0,
                    WriteFlag::Create,
                )
                .await?;
            fs.inner
                .meta_store
                .update_dir_meta("/local/acct/docs", &ctx, |_redirect, sync_log| {
                    sync_log
                        .entries
                        .insert("a.txt".to_string(), SyncLogEntry::new(1, SyncOp::Create));
                    Ok(())
                })
                .await?;

            let err = fs
                .inner
                .retry_pending_dir("/local/acct/docs")
                .await
                .expect_err("corrupt redirect metadata must be reported to retry logic");
            assert!(
                err.to_string().contains("redirect")
                    || err.to_string().contains("json")
                    || err.to_string().contains("serialization"),
                "unexpected error: {err}"
            );
            Ok::<(), Error>(())
        })
        .await
        .unwrap();
}
