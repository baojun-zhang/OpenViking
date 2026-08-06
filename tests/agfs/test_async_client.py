# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: AGPL-3.0

import asyncio
from unittest.mock import AsyncMock

import pytest

import openviking.pyagfs.async_client as async_client
from openviking.pyagfs import AsyncAGFSClient
from openviking.pyagfs.async_client import PathLockKeepaliveError


class _SyncAGFS:
    """Minimal synchronous binding stub used by the async adapter tests."""

    def read(self, path, **kwargs):
        """Return read call arguments."""
        return ("read", path, kwargs)

    def write(self, path, data, **kwargs):
        """Return write call arguments."""
        return ("write", path, data, kwargs)

    def rm(self, path, **kwargs):
        """Return remove call arguments."""
        return ("rm", path, kwargs)

    def pathlock_is_locked(self, ctx, path, ignore_stale):
        """Return pathlock query arguments."""
        return ("pathlock_is_locked", ctx, path, ignore_stale)


@pytest.mark.asyncio
async def test_async_agfs_client_hides_threadpool(monkeypatch):
    to_thread_calls = []

    async def fake_to_thread(func, *args, **kwargs):
        to_thread_calls.append((func.__name__, args, kwargs))
        return func(*args, **kwargs)

    monkeypatch.setattr(async_client.asyncio, "to_thread", fake_to_thread)

    sync_agfs = _SyncAGFS()
    agfs = AsyncAGFSClient(sync_agfs)

    assert agfs._client is sync_agfs
    assert await agfs.write("/tasks/1", b"data") == (
        "write",
        "/tasks/1",
        b"data",
        {"ctx": {"account_id": "_system"}},
    )
    assert await agfs.read("/queue/dequeue") == (
        "read",
        "/queue/dequeue",
        {"ctx": {"account_id": "_system"}},
    )
    assert await agfs.rm("/redo/id", recursive=True) == (
        "rm",
        "/redo/id",
        {"recursive": True, "ctx": {"account_id": "_system"}},
    )

    assert to_thread_calls == [
        ("write", ("/tasks/1", b"data"), {"ctx": {"account_id": "_system"}}),
        ("read", ("/queue/dequeue",), {"ctx": {"account_id": "_system"}}),
        (
            "rm",
            ("/redo/id",),
            {"recursive": True, "ctx": {"account_id": "_system"}},
        ),
    ]


@pytest.mark.asyncio
async def test_hold_pathlock_tree_refreshes_and_releases():
    """Keepalive helper should refresh in the background and always release."""
    agfs = AsyncAGFSClient(_SyncAGFS())
    lease = {"lease_ref": "lease-1", "owner_id": "owner-1", "owned": True}
    refreshed = asyncio.Event()
    agfs.pathlock_acquire_tree = AsyncMock(return_value=lease)
    agfs.pathlock_release = AsyncMock()

    async def _refresh(_lease):
        refreshed.set()
        return "refreshed"

    agfs.pathlock_refresh = AsyncMock(side_effect=_refresh)

    async with agfs.hold_pathlock_tree(
        "/local/default/data/session", refresh_interval_secs=0.01
    ) as held:
        assert held == lease
        await asyncio.wait_for(refreshed.wait(), timeout=0.2)

    agfs.pathlock_acquire_tree.assert_awaited_once()
    agfs.pathlock_release.assert_awaited_once_with(lease)


@pytest.mark.asyncio
async def test_hold_pathlock_tree_raises_when_refresh_fails():
    """Keepalive helper should abort the guarded block after refresh failure."""
    agfs = AsyncAGFSClient(_SyncAGFS())
    lease = {"lease_ref": "lease-1", "owner_id": "owner-1", "owned": True}
    agfs.pathlock_acquire_tree = AsyncMock(return_value=lease)
    agfs.pathlock_release = AsyncMock()
    agfs.pathlock_refresh = AsyncMock(return_value="failed")

    with pytest.raises(PathLockKeepaliveError, match="failed"):
        async with agfs.hold_pathlock_tree(
            "/local/default/data/session",
            refresh_interval_secs=0.01,
        ):
            await asyncio.sleep(0.05)

    agfs.pathlock_release.assert_awaited_once_with(lease)
