# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: Apache-2.0
import json
from dataclasses import asdict
from typing import Any, Dict, Optional, Union

from .named_queue import NamedQueue
from .embedding_msg import EmbeddingMsg
from openviking.utils.logger import get_logger

logger = get_logger(__name__)

class EmbeddingQueue(NamedQueue):
    """EmbeddingQueue: Named queue specifically for processing EmbeddingMsg.

    Supports direct enqueue and dequeue of EmbeddingMsg objects.
    """

    async def enqueue(self, msg: EmbeddingMsg) -> str:
        """Serialize EmbeddingMsg object and store in queue."""
        return await super().enqueue(msg.to_dict())

    async def dequeue(self) -> Optional[EmbeddingMsg]:
        """Get message from queue and deserialize to EmbeddingMsg object."""
        data_dict = await super().dequeue()
        if not data_dict:
            return None
        if "data" in data_dict and isinstance(data_dict["data"], str):
            try:
                return EmbeddingMsg.from_json(data_dict["data"])
            except Exception as e:
                logger.debug(f"[EmbeddingQueue] Failed to parse message data: {e}")
                return None
        # Otherwise try to convert directly from dict
        try:
            return EmbeddingMsg.from_dict(data_dict)
        except Exception as e:
            logger.debug(f"[EmbeddingQueue] Failed to create EmbeddingMsg from dict: {e}")
            return None

    async def peek(self) -> Optional[EmbeddingMsg]:
        """Peek at head message in queue."""
        data_dict = await super().peek()
        if not data_dict:
            return None
            
        if "data" in data_dict and isinstance(data_dict["data"], str):
            try:
                return EmbeddingMsg.from_json(data_dict["data"])
            except Exception:
                return None
                
        try:
            return EmbeddingMsg.from_dict(data_dict)
        except Exception:
            return None
