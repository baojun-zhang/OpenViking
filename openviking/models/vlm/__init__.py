# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: Apache-2.0
"""VLM (Vision-Language Model) module"""

from .base import VLMBase, VLMFactory
from .backends.openai_vlm import OpenAIVLM
from .backends.volcengine_vlm import VolcEngineVLM

__all__ = [
    "VLMBase",
    "VLMFactory",
    "OpenAIVLM",
    "VolcEngineVLM",
]
