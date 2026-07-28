# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: AGPL-3.0

from __future__ import annotations

import pytest

from openviking.storage.errors import LockAcquisitionError
from openviking.storage.queuefs.semantic_dag import DagStats
from openviking.storage.queuefs.semantic_lock import SemanticLockScope
from openviking.storage.queuefs.semantic_msg import SemanticMsg
from openviking.storage.queuefs.semantic_processor import SemanticProcessor


class _FakePathLock:
    """Mock for _async_agfs pathlock operations."""

    def __init__(self, *, adopt_raises=False):
        self._next_id = 0
        self.acquire_tree_calls: list[str] = []
        self.release_calls: list[str] = []
        self._adopt_raises = adopt_raises

    def _new_lease(self):
        self._next_id += 1
        return {"id": f"lock-{self._next_id}"}

    async def pathlock_as_borrowed(self, caller_lock):
        return dict(caller_lock)

    async def pathlock_adopt(self, lock_handoff):
        if self._adopt_raises:
            raise LockAcquisitionError("lock handle is no longer active")
        return self._new_lease()

    async def pathlock_acquire_tree(self, lock_path):
        self.acquire_tree_calls.append(lock_path)
        return self._new_lease()

    async def pathlock_release(self, lease):
        self.release_calls.append(lease["id"])


class _FakeVikingFS:
    def __init__(self, pathlock=None):
        self._async_agfs = pathlock or _FakePathLock()

    async def exists(self, uri, ctx=None):
        del uri, ctx
        return False

    async def ls(self, uri, node_limit=None, ctx=None):
        del uri, node_limit, ctx
        return []

    def _uri_to_path(self, uri, ctx=None):
        del ctx
        return f"/fake/{uri.replace('://', '/').strip('/')}"


@pytest.mark.asyncio
async def test_semantic_processor_borrows_caller_owned_lock(monkeypatch):
    processor = SemanticProcessor()
    pathlock = _FakePathLock()
    caller_lease = {"id": "lock-1"}

    class _FakeDagExecutor:
        def __init__(self, **kwargs):
            self.lock = kwargs["lock"]
            self.stale = False

        async def run(self, root_uri):
            assert root_uri == "viking://resources/demo"
            assert self.lock["id"] == "lock-1"

        def get_stats(self):
            return DagStats()

    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_processor.get_viking_fs",
        lambda: _FakeVikingFS(pathlock),
    )

    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_processor.SemanticDagExecutor",
        lambda **kwargs: _FakeDagExecutor(**kwargs),
    )

    await processor.on_dequeue(
        SemanticMsg(
            uri="viking://resources/demo",
            context_type="resource",
            recursive=False,
        ).to_dict(),
        lock=caller_lease,
    )

    assert pathlock.release_calls == []


@pytest.mark.asyncio
async def test_memory_semantic_directory_does_not_release_borrowed_lock(monkeypatch):
    processor = SemanticProcessor()
    pathlock = _FakePathLock()
    borrowed_lease = {"id": "borrowed-lock", "owned": False}

    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_processor.get_viking_fs",
        lambda: _FakeVikingFS(pathlock),
    )

    await processor._process_memory_directory(
        SemanticMsg(
            uri="viking://memory/demo",
            context_type="memory",
            recursive=False,
        ),
        lock=borrowed_lease,
    )

    assert pathlock.release_calls == []


@pytest.mark.asyncio
async def test_semantic_processor_recovers_stale_non_tree_handoff(monkeypatch):
    processor = SemanticProcessor()
    pathlock = _FakePathLock(adopt_raises=True)

    class _FakeDagExecutor:
        def __init__(self, **kwargs):
            self.lock = kwargs["lock"]
            self.stale = False

        async def run(self, root_uri):
            assert root_uri == "viking://resources/demo"
            assert self.lock["id"] == "lock-1"

        def get_stats(self):
            return DagStats()

    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_processor.get_viking_fs",
        lambda: _FakeVikingFS(pathlock),
    )
    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_lock.get_viking_fs",
        lambda: _FakeVikingFS(pathlock),
    )
    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_processor.SemanticDagExecutor",
        lambda **kwargs: _FakeDagExecutor(**kwargs),
    )

    await processor.on_dequeue(
        SemanticMsg(
            uri="viking://resources/demo",
            context_type="resource",
            recursive=False,
            lock_handoff={
                "handle_id": "stale-lock",
                "lock_paths": ["/fake/viking/resources/.mw_exact_demo.deadbeef"],
            },
        ).to_dict()
    )

    assert pathlock.acquire_tree_calls == ["/fake/viking/resources/demo"]
    assert pathlock.release_calls == ["lock-1"]


@pytest.mark.asyncio
async def test_semantic_lock_scope_reacquires_tree_lock_when_handoff_handle_is_stale(
    monkeypatch,
):
    pathlock = _FakePathLock(adopt_raises=True)
    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_lock.get_viking_fs",
        lambda: _FakeVikingFS(pathlock),
    )

    scope = await SemanticLockScope.resolve(
        {
            "handle_id": "stale-lock",
            "lock_paths": ["/local/default/resources/CONTRIBUTING_CN_3/.path.ovlock"],
        },
        fallback_path_factory=lambda: (_ for _ in ()).throw(
            AssertionError("tree handoffs must not evaluate the fallback path")
        ),
    )

    try:
        assert scope.lock["id"] == "lock-1"
        assert pathlock.acquire_tree_calls == ["/local/default/resources/CONTRIBUTING_CN_3"]
    finally:
        await scope.close()

    assert pathlock.release_calls == ["lock-1"]


@pytest.mark.asyncio
async def test_semantic_lock_scope_reacquires_fallback_path_for_stale_non_tree_handoff(
    monkeypatch,
):
    pathlock = _FakePathLock(adopt_raises=True)
    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_lock.get_viking_fs",
        lambda: _FakeVikingFS(pathlock),
    )

    scope = await SemanticLockScope.resolve(
        {
            "handle_id": "stale-lock",
            "lock_paths": ["/local/default/resources/.mw_exact_demo.deadbeef"],
        },
        fallback_path_factory=lambda: "/local/default/resources/demo",
    )

    try:
        assert scope.lock["id"] == "lock-1"
        assert pathlock.acquire_tree_calls == ["/local/default/resources/demo"]
    finally:
        await scope.close()

    assert pathlock.release_calls == ["lock-1"]


@pytest.mark.asyncio
async def test_semantic_processor_lock_error_requeues_without_circuit_breaker(monkeypatch):
    processor = SemanticProcessor()
    reenqueue_calls = []
    success_called = False
    requeue_called = False
    error_called = False

    async def _reenqueue(msg):
        reenqueue_calls.append(msg.uri)

    def on_success():
        nonlocal success_called
        success_called = True

    def on_requeue():
        nonlocal requeue_called
        requeue_called = True

    def on_error(error_msg, error_data=None):
        del error_msg, error_data
        nonlocal error_called
        error_called = True

    def _record_failure(error):
        raise AssertionError(f"lock errors must not trip circuit breaker: {error}")

    processor.set_callbacks(on_success, on_requeue, on_error)
    processor._circuit_breaker.record_failure = _record_failure

    monkeypatch.setattr(processor, "_reenqueue_semantic_msg", _reenqueue)
    monkeypatch.setattr(
        "openviking.storage.queuefs.semantic_processor.SemanticLockScope.resolve",
        lambda *args, **kwargs: (_ for _ in ()).throw(
            LockAcquisitionError("lock handle is no longer active")
        ),
    )

    msg = SemanticMsg(
        uri="viking://resources/CONTRIBUTING_CN_3",
        context_type="resource",
        recursive=True,
        lock_handoff={
            "handle_id": "stale-lock",
            "lock_paths": ["/local/default/resources/CONTRIBUTING_CN_3/.path.ovlock"],
        },
    )

    await processor.on_dequeue(msg.to_dict())

    assert reenqueue_calls == ["viking://resources/CONTRIBUTING_CN_3"]
    assert requeue_called is True
    assert success_called is True
    assert error_called is False
