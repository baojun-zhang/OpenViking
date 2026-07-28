# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: AGPL-3.0

import pytest

from openviking.storage.viking_fs import VikingFS


class _StatAGFS:
    def __init__(self):
        self.pathlock_is_locked_calls = []

    async def pathlock_is_locked(self, path):
        self.pathlock_is_locked_calls.append(path)
        return True

    def stat(self, path):
        return {"name": path.rsplit("/", 1)[-1], "isDir": False}


@pytest.mark.asyncio
async def test_stat_uses_async_lock_lookup(monkeypatch):
    agfs = _StatAGFS()

    fs = VikingFS(agfs=agfs)
    result = await fs.stat("viking://resources/file.txt")

    assert result["isLocked"] is True
    assert agfs.pathlock_is_locked_calls == ["/local/default/resources/file.txt"]
