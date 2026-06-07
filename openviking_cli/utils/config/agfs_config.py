# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: AGPL-3.0
from __future__ import annotations

from enum import Enum
from typing import Any, List, Optional

from pydantic import BaseModel, Field, model_validator

from openviking_cli.utils.logger import get_logger

logger = get_logger(__name__)


class DirectoryMarkerMode(str, Enum):
    """How S3 directory markers should be persisted."""

    NONE = "none"
    EMPTY = "empty"
    NONEMPTY = "nonempty"


class S3Config(BaseModel):
    """Configuration for S3 backend."""

    bucket: Optional[str] = Field(default=None, description="S3 bucket name")

    region: Optional[str] = Field(
        default=None,
        description="AWS region where the bucket is located (e.g., us-east-1, cn-beijing)",
    )

    access_key: Optional[str] = Field(
        default=None,
        description="S3 access key ID. If not provided, RAGFS may attempt to use environment variables or IAM roles.",
    )

    secret_key: Optional[str] = Field(
        default=None,
        description="S3 secret access key corresponding to the access key ID.",
    )

    endpoint: Optional[str] = Field(
        default=None,
        description="Custom S3 endpoint URL. Required for S3-compatible services like MinIO or LocalStack. "
        "Leave empty for standard AWS S3.",
    )

    prefix: Optional[str] = Field(
        default="",
        description="Optional key prefix for namespace isolation. All objects will be stored under this prefix.",
    )

    use_ssl: bool = Field(
        default=True,
        description="Enable/Disable SSL (HTTPS) for S3 connections. Set to False for local testing without HTTPS.",
    )

    use_path_style: bool = Field(
        default=True,
        description="true represent UsePathStyle for MinIO and some S3-compatible services; false represent VirtualHostStyle for TOS  and some S3-compatible services.",
    )

    directory_marker_mode: DirectoryMarkerMode = Field(
        default=DirectoryMarkerMode.EMPTY,
        description="How to persist S3 directory markers: 'none' skips marker creation, 'empty' writes a zero-byte marker, and 'nonempty' writes a non-empty marker payload. Defaults to 'empty'.",
    )

    disable_batch_delete: bool = Field(
        default=False,
        description="Disable batch delete (DeleteObjects) and use sequential single-object deletes instead. "
        "Required for S3-compatible services like Alibaba Cloud OSS that require a Content-MD5 header "
        "for DeleteObjects but AWS SDK v2 does not send it by default. Defaults to False.",
    )

    normalize_encoding_chars: str = Field(
        default="?#%+@",
        description="Characters to escape in S3 object keys as !HH hexadecimal bytes. "
        "Set to an empty string to disable key normalization. Defaults to ?#%+@.",
    )

    model_config = {"extra": "forbid"}

    def validate_config(self):
        """Validate S3 configuration completeness"""
        missing = []
        if not self.bucket:
            missing.append("bucket")
        if not self.endpoint:
            missing.append("endpoint")
        if not self.region:
            missing.append("region")
        if not self.access_key:
            missing.append("access_key")
        if not self.secret_key:
            missing.append("secret_key")

        if missing:
            raise ValueError(f"S3 backend requires the following fields: {', '.join(missing)}")

        return self


class QueueFSConfig(BaseModel):
    """Configuration for QueueFS backend."""

    mode: str = Field(
        default="shared",
        description="QueueFS namespace mode: 'shared' | 'worker'",
    )

    backend: str = Field(
        default="sqlite",
        description="QueueFS backend: 'memory' | 'sqlite' | 'sqlite3'",
    )

    db_path: Optional[str] = Field(
        default=None,
        description="SQLite database path for QueueFS when backend is 'sqlite' or 'sqlite3'.",
    )

    recover_stale_sec: int = Field(
        default=0,
        description="Recover processing messages older than this many seconds on startup (0 = recover all).",
    )

    busy_timeout_ms: int = Field(
        default=5000,
        description="SQLite busy timeout for QueueFS in milliseconds.",
    )

    model_config = {"extra": "forbid"}

    @model_validator(mode="after")
    def validate_config(self):
        valid_modes = {"shared", "worker"}
        if self.mode not in valid_modes:
            raise ValueError("queuefs mode must be one of: 'shared', 'worker'")

        valid_backends = {"memory", "sqlite", "sqlite3"}
        if self.backend not in valid_backends:
            raise ValueError("queuefs backend must be one of: 'memory', 'sqlite', 'sqlite3'")
        if self.recover_stale_sec < 0:
            raise ValueError("queuefs recover_stale_sec must be >= 0")
        if self.busy_timeout_ms < 0:
            raise ValueError("queuefs busy_timeout_ms must be >= 0")
        return self


def _expect_dict(value: Any, field_name: str) -> dict[str, Any]:
    """Return one config object as a dict or raise a targeted shape error."""
    if not isinstance(value, dict):
        raise ValueError(f"{field_name} must be an object")
    return value


def _expect_list(value: Any, field_name: str) -> list[Any]:
    """Return one config value as a list or raise a targeted shape error."""
    if not isinstance(value, list):
        raise ValueError(f"{field_name} must be a list")
    return value


def _expect_non_empty_str(value: Any, field_name: str) -> str:
    """Return one config value as a non-empty string."""
    if not isinstance(value, str) or not value:
        raise ValueError(f"{field_name} must be a non-empty string")
    return value


class AGFSConfig(BaseModel):
    """Configuration for RAGFS (Rust-based AGFS)."""

    name: str = Field(
        default="primary",
        description="Logical backend name, globally unique across primary and all backups",
    )

    path: Optional[str] = Field(
        default=None,
        description="[Deprecated in favor of `storage.workspace`] RAGFS data storage path. This will be ignored if `storage.workspace` is set.",
    )

    port: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS service port. Ignored by RAGFS.",
    )

    log_level: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS log level. Ignored by RAGFS.",
    )

    url: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS service URL. Ignored by RAGFS.",
    )

    mode: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS client mode. Ignored by RAGFS.",
    )

    impl: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS binding implementation selector. Ignored by RAGFS.",
    )

    backend: str = Field(
        default="local", description="RAGFS storage backend: 'local' | 's3' | 'memory'"
    )

    timeout: int = Field(default=10, description="RAGFS request timeout (seconds)")

    queue_db_path: Optional[str] = Field(
        default=None,
        description="Override path of the queuefs sqlite database file. "
        "Defaults to '{storage.workspace}/_system/queue/queue.db' when not set. "
        "Useful when the workspace volume does not support sqlite (e.g. some network filesystems).",
    )

    queuefs: QueueFSConfig = Field(
        default_factory=QueueFSConfig,
        description="QueueFS configuration.",
    )

    retry_times: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS retry count. Ignored by RAGFS.",
    )

    use_ssl: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS SSL switch. Ignored by RAGFS.",
    )

    lib_path: Any = Field(
        default=None,
        exclude=True,
        description="[Deprecated] Legacy AGFS binding library path. Ignored by RAGFS.",
    )

    # S3 backend configuration
    # These settings are used when backend is set to 's3'.
    # RAGFS will act as a gateway to the specified S3 bucket.
    s3: S3Config = Field(default_factory=lambda: S3Config(), description="S3 backend configuration")

    # Multi-write configuration
    backups: Optional[dict[str, Any]] = Field(
        default=None, description="Multi-write backups configuration. None = single backend mode."
    )
    redirects: Optional[List[dict[str, Any]]] = Field(
        default=None, description="Primary redirect policies."
    )

    model_config = {"extra": "forbid"}

    @model_validator(mode="after")
    def validate_config(self):
        """Validate configuration completeness and consistency"""
        deprecated_fields = (
            "port",
            "log_level",
            "url",
            "mode",
            "impl",
            "retry_times",
            "use_ssl",
            "lib_path",
        )
        for field_name in deprecated_fields:
            if field_name in self.model_fields_set:
                logger.warning(
                    "AGFSConfig: 'storage.agfs.%s' is deprecated and ignored after the RAGFS migration.",
                    field_name,
                )

        if self.backend not in ["local", "s3", "memory"]:
            raise ValueError(
                f"Invalid RAGFS backend: '{self.backend}'. Must be one of: 'local', 's3', 'memory'"
            )

        if self.backend == "local":
            pass

        elif self.backend == "s3":
            # Validate S3 configuration
            self.s3.validate_config()

        if self.queue_db_path is not None and self.queuefs.db_path is None:
            logger.warning(
                "AGFSConfig: 'storage.agfs.queue_db_path' is deprecated; "
                "prefer 'storage.agfs.queuefs.db_path'."
            )

        if self.queuefs.backend == "memory":
            if self.queuefs.db_path is not None or self.queue_db_path is not None:
                logger.warning(
                    "AGFSConfig: QueueFS backend is 'memory'; "
                    "db_path/queue_db_path will be ignored."
                )

        if self.redirects is not None and self.backups is None:
            raise ValueError(
                "redirects requires backups; single-backend mode does not support redirects"
            )

        if self.backups is not None:
            backups = _expect_dict(self.backups, "backups")
            items = _expect_list(backups.get("items"), "backups.items")
            sync_type = backups.get("sync_type", "async")
            if sync_type not in {"sync", "async"}:
                raise ValueError("backups.sync_type must be one of: 'sync', 'async'")
            if sync_type == "sync" and backups.get("write_ack_count") is None:
                raise ValueError("backups.write_ack_count is required when sync_type is 'sync'")

            names_seen: set[str] = {self.name}
            backup_names: set[str] = set()
            for index, item in enumerate(items):
                item_dict = _expect_dict(item, f"backups.items[{index}]")
                item_name = _expect_non_empty_str(
                    item_dict.get("name"), f"backups.items[{index}].name"
                )
                backend_type = _expect_non_empty_str(
                    item_dict.get("backend"), f"backups.items[{index}].backend"
                )

                if "backups" in item_dict:
                    raise ValueError("extra field 'backups' is not allowed in backup items")
                if item_name == "primary":
                    raise ValueError("backup backend name 'primary' is reserved")
                if item_name in names_seen:
                    raise ValueError(
                        f"Duplicate backend name '{item_name}': all backend names "
                        f"(primary + backups) must be globally unique"
                    )

                names_seen.add(item_name)
                backup_names.add(item_name)

                if backend_type == "s3" and "s3" in item_dict and item_dict["s3"] is not None:
                    S3Config.model_validate(item_dict["s3"]).validate_config()

            if self.redirects is not None:
                for index, policy in enumerate(self.redirects):
                    policy_dict = _expect_dict(policy, f"redirects[{index}]")
                    targets = policy_dict.get("target")
                    if not targets:
                        raise ValueError("Redirect target must not be empty")
                    for target_name in _expect_list(targets, f"redirects[{index}].target"):
                        if target_name not in backup_names:
                            raise ValueError(
                                f"Redirect target '{target_name}' not found in backups. "
                                f"Available backups: {sorted(backup_names)}"
                            )

        return self
