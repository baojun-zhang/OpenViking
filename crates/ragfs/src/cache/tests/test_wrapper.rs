use super::super::envelope::QueryCacheEnvelope;
use super::*;
use crate::cache_runtime::{CacheRuntime, MemoryMockProvider};
use crate::core::{FileInfo, FileSystem, WriteFlag};
use crate::plugins::memfs::MemFileSystem;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const SERVICE_NAME: &str = "query-cache-test";

/// Return a sorted list of entry names for stable assertions.
fn sorted_names(entries: Vec<FileInfo>) -> Vec<String> {
    let mut names = entries
        .into_iter()
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    names.sort();
    names
}

/// Wait until asynchronous invalidation reaches the expected unlink count.
async fn wait_for_unlink_count(provider: &MemoryMockProvider, expected: u64) {
    for _ in 0..50 {
        if provider.unlink_count() >= expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!(
        "expected at least {expected} unlink calls, got {}",
        provider.unlink_count()
    );
}

/// Verifies read_dir hits the query cache and write invalidates the parent bucket.
#[tokio::test]
async fn query_cache_hits_and_invalidates_after_write() {
    let backend = MemFileSystem::new();
    backend.mkdir("/docs", 0o755).await.unwrap();
    backend
        .write("/docs/a.txt", b"a", 0, WriteFlag::Create)
        .await
        .unwrap();
    let provider = Arc::new(MemoryMockProvider::new());
    let runtime = CacheRuntime::memory_with_provider(Arc::clone(&provider));
    let cached = CachedFileSystem::with_runtime(Box::new(backend), runtime, SERVICE_NAME);

    assert_eq!(
        sorted_names(cached.read_dir("/docs", None, None, None, None).await.unwrap()),
        vec!["a.txt"]
    );
    assert_eq!(
        sorted_names(cached.read_dir("/docs", None, None, None, None).await.unwrap()),
        vec!["a.txt"]
    );
    let snapshot = cached.metrics().snapshot();
    assert_eq!(snapshot.query_misses, 1);
    assert_eq!(snapshot.query_hits, 1);
    assert_eq!(snapshot.backend_fallbacks, 1);
    assert_eq!(snapshot.puts, 1);

    let expected_unlinks = provider.unlink_count() + 1;
    cached
        .write("/docs/b.txt", b"b", 0, WriteFlag::Create)
        .await
        .unwrap();
    wait_for_unlink_count(&provider, expected_unlinks).await;

    assert_eq!(
        sorted_names(cached.read_dir("/docs", None, None, None, None).await.unwrap()),
        vec!["a.txt", "b.txt"]
    );
    assert_eq!(cached.metrics().snapshot().query_misses, 2);
}

/// Verifies PathLock sidecar files are filtered before query results are cached.
#[tokio::test]
async fn pathlock_files_are_not_stored_in_query_cache() {
    let backend = MemFileSystem::new();
    backend.mkdir("/docs", 0o755).await.unwrap();
    backend
        .write("/docs/visible.txt", b"visible", 0, WriteFlag::Create)
        .await
        .unwrap();
    backend
        .write("/docs/.path.ovlock", b"lock", 0, WriteFlag::Create)
        .await
        .unwrap();
    backend
        .write(
            "/docs/.exact.ovlock.visible.txt.1234",
            b"lock",
            0,
            WriteFlag::Create,
        )
        .await
        .unwrap();
    let runtime = CacheRuntime::memory_with_provider(Arc::new(MemoryMockProvider::new()));
    let cached = CachedFileSystem::with_runtime(Box::new(backend), Arc::clone(&runtime), SERVICE_NAME);

    assert_eq!(
        sorted_names(cached.read_dir("/docs", None, None, None, None).await.unwrap()),
        vec!["visible.txt"]
    );
    assert_eq!(
        sorted_names(cached.read_internal_dir("/docs").await.unwrap()),
        vec![
            ".exact.ovlock.visible.txt.1234",
            ".path.ovlock",
            "visible.txt",
        ]
    );

    let (request, query_hash) = canonical_query_request(json!({
        "path": "/docs",
        "offset": null,
        "limit": null,
        "sort_by": null,
        "sort_order": null,
    }))
    .unwrap();
    let bucket_key = query_bucket_key(SERVICE_NAME, "/docs", "read_dir");
    let cached_value = runtime
        .hget(&bucket_key, &query_hash)
        .await
        .unwrap()
        .expect("read_dir result should be cached");
    let envelope = QueryCacheEnvelope::<Vec<FileInfo>>::decode(&cached_value).unwrap();
    assert!(envelope.request_matches(&request));
    assert_eq!(sorted_names(envelope.into_payload()), vec!["visible.txt"]);

    cached.stat("/docs/.path.ovlock").await.unwrap();
    let (_, internal_hash) = canonical_query_request(json!({
        "path": "/docs/.path.ovlock",
    }))
    .unwrap();
    let internal_bucket = query_bucket_key(SERVICE_NAME, "/docs/.path.ovlock", "stat");
    assert_eq!(runtime.hget(&internal_bucket, &internal_hash).await.unwrap(), None);
}
