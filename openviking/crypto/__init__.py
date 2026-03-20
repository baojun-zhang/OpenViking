# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: Apache-2.0
"""
OpenViking Encryption Module

Provides multi-tenant encryption functionality, including:
- Envelope Encryption
- Multiple key providers (Local File, Vault, Volcengine KMS)
- API Key hashing storage (Argon2id)
"""

from openviking.crypto.providers import (
    RootKeyProvider,
    LocalFileProvider,
    VaultProvider,
    VolcengineKMSProvider,
    create_root_key_provider,
)
from openviking.crypto.encryptor import FileEncryptor
from openviking.crypto.config import (
    validate_encryption_config,
    bootstrap_encryption,
)
from openviking.crypto.exceptions import (
    EncryptionError,
    InvalidMagicError,
    CorruptedCiphertextError,
    AuthenticationFailedError,
    KeyMismatchError,
)

__all__ = [
    "RootKeyProvider",
    "LocalFileProvider",
    "VaultProvider",
    "VolcengineKMSProvider",
    "create_root_key_provider",
    "FileEncryptor",
    "validate_encryption_config",
    "bootstrap_encryption",
    "EncryptionError",
    "InvalidMagicError",
    "CorruptedCiphertextError",
    "AuthenticationFailedError",
    "KeyMismatchError",
]
