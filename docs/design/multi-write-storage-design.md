# Multi-Write Storage Design

## Background

OpenViking stores user content through RAGFS. Existing deployments usually mount a single AGFS backend such as local storage, S3-compatible object storage, or an in-memory backend for tests.

Single-backend storage is simple, but it cannot cover these production needs well:

- Keep a near-real-time copy of content in another storage system.
- Route read traffic to a lower-latency backend without changing public APIs.
- Migrate from one backend to another while reducing the window for data loss.
- Store selected files in a different backend according to path, extension, or size policies.

Multi-write storage adds a primary/backup model inside RAGFS. VikingFS and Python APIs continue to use the same file-system abstraction; they do not need to know which backend stores the physical bytes.

## Goals

- Support one primary backend and one or more backup backends under `storage.agfs`.
- Keep the existing single-backend configuration fully compatible.
- Support asynchronous and synchronous write replication.
- Support read routing through explicitly enabled backup backends.
- Support redirect and exclude policies for selected files.
- Keep encryption transparent to Python and API callers.
- Persist synchronization metadata so failed backup writes can be retried.

## Non-Goals

- This design does not expose backend-specific application-layer capabilities.
- This design does not make Python or public APIs aware of encryption internals.
- This design does not introduce an external task queue such as Kafka or RabbitMQ.
- This design does not guarantee multi-process writes to the same primary path unless a distributed metadata lock provider is added later.
- This design does not automatically backfill historical files that existed before multi-write was enabled.

## Configuration Model

The top-level `storage.agfs.backend` remains the primary backend. Backup backends are configured under `storage.agfs.backups.items`.

```json
{
  "storage": {
    "workspace": "./data",
    "agfs": {
      "backend": "local",
      "backups": {
        "sync_type": "async",
        "items": [
          {
            "name": "local-backup",
            "backend": "local",
            "local": {
              "local_dir": "./data/backup"
            }
          }
        ]
      }
    }
  }
}
```

Compatibility rules:

- If `backups` is absent or empty, RAGFS uses the existing single-backend path.
- The top-level backend is always the primary backend.
- Each `backups.items[]` entry is a backup backend.
- Backup `name` values must be unique because metadata refers to backends by name.
- Backup items reuse normal backend plugin configuration fields.

## Roles

| Role | Location | Responsibility |
| --- | --- | --- |
| Primary | `storage.agfs.backend` | Authoritative write target and final read fallback |
| Backup | `storage.agfs.backups.items[]` | Receives replicated writes and may optionally serve reads |

The primary backend always participates in writes. Backups participate in writes by default unless their `operations` configuration removes write support.

Backups do not participate in reads by default. A backup must explicitly declare a read operation with a priority before it can serve reads.

## Write Routing

For regular writes, RAGFS writes to the primary first and then replicates the operation to write-enabled backups.

```text
client write
  -> MultiWriteWrappedFS
  -> primary backend
  -> write-enabled backup backends
```

Write-class operations include file content writes, directory creation, removal, rename, truncation, permission changes, and parent directory creation.

### Asynchronous Mode

`sync_type: "async"` is the default mode.

Behavior:

- The primary write must succeed.
- The client receives success after the primary write succeeds.
- Backup writes run in background tasks.
- Backup failures are recorded in `.sync_log.json` and retried later.

This mode favors latency and eventual consistency. During the replication window, a read-enabled backup can briefly lag behind the primary.

### Synchronous Mode

`sync_type: "sync"` waits for backup acknowledgements before returning.

```json
{
  "backups": {
    "sync_type": "sync",
    "write_ack_count": 1,
    "write_ack_timeout_ms": 5000,
    "items": []
  }
}
```

Behavior:

- The primary write must succeed.
- RAGFS waits for at least `write_ack_count` backup acknowledgements.
- Backups that fail or time out remain behind and are repaired by retry.
- If the primary succeeds but the required backup quorum is not reached, the client can receive an error even though primary data already exists.

The last point is intentional: file-system writes are not transactional across independent backends. The metadata records the latest version so retry can continue repairing backup state.

## Read Routing

Read routing uses a deterministic fallback chain:

```text
1. Read-enabled backups, ordered by ascending priority
2. Primary backend
3. Redirect targets recorded in primary metadata
4. NotFound
```

Only backups that explicitly configure `operations: [{"operation": "read", "priority": N}]` are considered for read routing.

This avoids an unsafe default where cold backups accidentally serve stale reads.

## Redirect Policies

Redirect policies are configured on the primary backend. A matching file is written to the configured target backup instead of the primary.

```json
{
  "storage": {
    "agfs": {
      "backend": "local",
      "redirects": [
        {
          "type": "FileExtensionPolicy",
          "extensions": ["(pdf|ppt)"],
          "target": ["object-store"]
        }
      ],
      "backups": {
        "items": [
          {
            "name": "object-store",
            "backend": "s3",
            "s3": {
              "bucket": "openviking-backup",
              "endpoint": "https://s3.example.com"
            }
          }
        ]
      }
    }
  }
}
```

Supported policies:

| Policy | Purpose |
| --- | --- |
| `FileExtensionPolicy` | Match files by extension regex |
| `FileOverSizePolicy` | Match files above a configured size |

Redirect mappings are stored in primary metadata so reads, `stat`, `exists`, `read_dir`, and search-like traversal can still present a consistent file-system view.

## Exclude Policies

Exclude policies are configured on backup backends. A matching file is skipped for that backup.

```json
{
  "name": "cache-backend",
  "backend": "memfs",
  "excludes": [
    {
      "type": "FileOverSizePolicy",
      "max_size_mb": 500
    }
  ]
}
```

If a file is redirected to a backup that also excludes it, the configuration is contradictory. RAGFS does not silently choose another backend because that would hide configuration errors.

## Internal Metadata

Multi-write storage uses two internal metadata files under the primary backend:

| File | Purpose |
| --- | --- |
| `.redirect.json` | Records files physically stored in redirect targets |
| `.sync_log.json` | Records latest sequence and per-backend acknowledgement progress |

These files are internal system files:

- They are hidden from public directory listings.
- Users cannot operate on them through normal content APIs.
- They are stored under the primary namespace.
- If primary encryption is enabled, they are encrypted like other primary files.

Metadata updates must be serialized per directory so `.redirect.json` and `.sync_log.json` cannot overwrite each other during concurrent writes.

## Encryption Boundary

Encryption stays inside RAGFS wrappers.

The public API and Python layer only pass configuration. They do not encrypt, decrypt, or expose encryption-specific file operations.

Rules:

- If global file encryption is disabled, primary and backups store plaintext.
- If global file encryption is enabled, the primary backend is encrypted.
- If global file encryption is enabled, each backup can decide whether to enable its own encryption.
- Internal metadata must always go through the wrapped primary backend.
- There must be no unencrypted side path for writing `.redirect.json` or `.sync_log.json`.

This keeps the security model consistent: all files in the primary namespace, including multi-write metadata, follow the primary encryption policy.

## Failure Recovery

Backup failures are represented by lagging acknowledgement state in `.sync_log.json`.

The retry loop periodically:

- Scans synchronization metadata.
- Computes the current write-enabled backup targets.
- Finds backends whose `acked_seq` is missing or behind `latest_seq`.
- Replays the latest operation.
- Updates acknowledgement state after success.

Both asynchronous and synchronous modes use this recovery path. Synchronous mode still needs retry because quorum may be reached while some backups remain behind.

## Migration

Enabling multi-write does not automatically replicate historical files that already exist in the primary backend.

Recommended migration flow:

1. Export or copy existing data to the future backup backend.
2. Verify the copied data.
3. Enable multi-write configuration.
4. Let new writes replicate through RAGFS.

OVPack can be used for full content migration before multi-write is enabled. A dedicated backfill command can be added later for environments that need RAGFS-managed historical synchronization.

## Observability

Multi-write should expose operational visibility through existing metrics and future system commands:

- Backup write success and failure counts.
- Backup write latency.
- Retry attempts.
- Synchronization lag.
- Metadata size and update errors.

Health checks should report primary availability and backup availability separately so operators can distinguish degraded backup replication from primary data unavailability.

## Limitations

- Backup reads can be stale in asynchronous mode.
- Metadata locking is process-local unless a distributed lock provider is configured in the future.
- Hot directories can suffer metadata write amplification because directory-level metadata is rewritten.
- Redirected files require metadata to reconstruct directory views.
- Historical data requires an explicit migration or backfill step.
