# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: AGPL-3.0
"""Regression tests for traversal-style URI rejection in VikingFS."""

import contextvars
from unittest.mock import AsyncMock, MagicMock

import pytest

from openviking.pyagfs.exceptions import AGFSInvalidOperationError
from openviking.storage.viking_fs import VikingFS
from openviking_cli.exceptions import PermissionDeniedError


def _make_viking_fs() -> VikingFS:
    """Create a VikingFS instance with a mocked AGFS backend."""
    fs = VikingFS.__new__(VikingFS)
    fs.agfs = MagicMock()
    fs.query_embedder = None
    fs.rerank_config = None
    fs.vector_store = None
    fs._encryptor = None
    fs._bound_ctx = contextvars.ContextVar("vikingfs_bound_ctx_test", default=None)
    return fs


class TestVikingFSURITraversalGuard:
    """Traversal-style URI components should be rejected before any AGFS I/O."""

    @pytest.mark.parametrize(
        ("base_path", "match_file", "expected"),
        [
            ("/local/default/resources/docs", ".", "/local/default/resources/docs"),
            (
                "/local/default/resources/docs",
                "sub/a.md",
                "/local/default/resources/docs/sub/a.md",
            ),
        ],
    )
    def test_resolve_grep_match_agfs_path(
        self, base_path: str, match_file: str, expected: str
    ) -> None:
        fs = _make_viking_fs()

        assert fs._resolve_grep_match_agfs_path(base_path, match_file) == expected

    @pytest.mark.parametrize(
        "uri",
        [
            "viking://resources/../_system/users.json",
            "viking://resources/../../_system/accounts.json",
            "/resources/../_system/users.json",
            "viking://resources/..\\..\\_system\\users.json",
            "viking://resources/C:\\Windows\\System32",
        ],
    )
    def test_rejects_unsafe_uri_components(self, uri: str) -> None:
        fs = _make_viking_fs()

        with pytest.raises(PermissionDeniedError, match="Unsafe URI"):
            fs._normalized_uri_parts(uri)

    @pytest.mark.asyncio
    async def test_read_file_rejects_traversal_before_agfs_read(self) -> None:
        fs = _make_viking_fs()

        with pytest.raises(PermissionDeniedError, match="Unsafe URI"):
            await fs.read_file("viking://resources/../_system/users.json")

        fs.agfs.read.assert_not_called()

    @pytest.mark.asyncio
    async def test_write_rejects_traversal_before_agfs_write(self) -> None:
        fs = _make_viking_fs()

        with pytest.raises(PermissionDeniedError, match="Unsafe URI"):
            await fs.write("viking://resources/../../_system/accounts.json", "pwned")

        fs.agfs.write.assert_not_called()

    @pytest.mark.asyncio
    async def test_rm_rejects_traversal_before_side_effects(self) -> None:
        fs = _make_viking_fs()
        fs._collect_uris = AsyncMock(return_value=[])
        fs._delete_from_vector_store = AsyncMock()

        with pytest.raises(PermissionDeniedError, match="Unsafe URI"):
            await fs.rm("viking://resources/../../other_account/_system/users.json")

        fs._collect_uris.assert_not_called()
        fs._delete_from_vector_store.assert_not_called()
        fs.agfs.rm.assert_not_called()

    @pytest.mark.asyncio
    @pytest.mark.parametrize(
        ("old_uri", "new_uri"),
        [
            ("viking://resources/../_system/users.json", "viking://resources/safe.txt"),
            ("viking://resources/safe.txt", "viking://resources/../../victim/_system/users.json"),
        ],
    )
    async def test_mv_rejects_traversal_in_source_or_target(
        self, old_uri: str, new_uri: str
    ) -> None:
        fs = _make_viking_fs()
        fs._collect_uris = AsyncMock(return_value=[])
        fs._update_vector_store_uris = AsyncMock()
        fs._delete_from_vector_store = AsyncMock()

        with pytest.raises(PermissionDeniedError, match="Unsafe URI"):
            await fs.mv(old_uri, new_uri)

        fs._collect_uris.assert_not_called()
        fs._update_vector_store_uris.assert_not_called()
        fs._delete_from_vector_store.assert_not_called()
        fs.agfs.mv.assert_not_called()

    @pytest.mark.asyncio
    async def test_read_file_keeps_valid_uri_behavior(self) -> None:
        fs = _make_viking_fs()
        fs.agfs.stat = MagicMock(return_value=MagicMock())
        fs.agfs.read = MagicMock(return_value=b"hello")

        content = await fs.read_file("viking://resources/docs/guide.md")
        assert content == "hello"
        fs.agfs.stat.assert_called_once_with("/local/default/resources/docs/guide.md")
        fs.agfs.read.assert_called_once_with("/local/default/resources/docs/guide.md")

    @pytest.mark.asyncio
    async def test_grep_propagates_agfs_errors_instead_of_falling_back(self) -> None:
        fs = _make_viking_fs()
        fs._encryptor = None
        fs._ensure_access = MagicMock()
        fs._grep_with_agfs = AsyncMock(side_effect=AGFSInvalidOperationError("invalid regex"))
        fs._grep_encrypted = AsyncMock(
            return_value={"matches": [], "count": 0, "match_count": 0, "files_scanned": 0}
        )

        with pytest.raises(AGFSInvalidOperationError, match="invalid regex"):
            await fs.grep("viking://resources/docs", "(")

        fs._grep_with_agfs.assert_awaited_once()
        fs._grep_encrypted.assert_not_awaited()
