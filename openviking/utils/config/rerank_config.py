# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: Apache-2.0
import os
from typing import Optional

from pydantic import BaseModel, Field, model_validator


class RerankConfig(BaseModel):
    """Configuration for VikingDB Rerank API."""

    ak: Optional[str] = Field(default=None, description="VikingDB Access Key")
    sk: Optional[str] = Field(default=None, description="VikingDB Secret Key")
    host: str = Field(
        default="api-vikingdb.vikingdb.cn-beijing.volces.com", description="VikingDB API host"
    )
    model_name: str = Field(default="doubao-seed-rerank", description="Rerank model name")
    model_version: str = Field(default="251028", description="Rerank model version")
    threshold: float = Field(
        default=0.1, description="Relevance threshold (score > threshold is relevant)"
    )

    @model_validator(mode="before")
    @classmethod
    def apply_env_defaults(cls, data):
        """Apply environment variable defaults if manual config not provided."""
        if isinstance(data, dict):
            env_mapping = {
                "ak": "OPENVIKING_RERANK_AK",
                "sk": "OPENVIKING_RERANK_SK",
                "host": "OPENVIKING_RERANK_HOST",
            }
            for field, env_var in env_mapping.items():
                if data.get(field) is None:
                    env_val = os.getenv(env_var)
                    if env_val is not None:
                        data[field] = env_val
        return data

    def is_available(self) -> bool:
        """Check if rerank is configured."""
        return self.ak is not None and self.sk is not None
