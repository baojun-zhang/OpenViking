# Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
# SPDX-License-Identifier: Apache-2.0
"""测试新的 serializable 装饰器"""
import unittest
from openviking.storage.vectordb.store.data import CandidateData, DeltaRecord, TTLData


class TestSerializableDecorator(unittest.TestCase):
    def test_candidate_data_serialization(self):
        """测试 CandidateData 的序列化和反序列化"""
        data = CandidateData(
            label=123,
            vector=[1.0, 2.0, 3.0],
            sparse_raw_terms=["term1", "term2"],
            sparse_values=[0.5, 0.8],
            fields='{"name": "test"}',
            expire_ns_ts=1234567890,
        )

        # 序列化
        serialized = data.serialize()
        self.assertIsInstance(serialized, bytes)

        # 反序列化
        restored = CandidateData.from_bytes(serialized)
        self.assertEqual(restored.label, 123)
        self.assertEqual(restored.vector, [1.0, 2.0, 3.0])
        self.assertEqual(restored.sparse_raw_terms, ["term1", "term2"])
        self.assertEqual(len(restored.sparse_values), 2)
        self.assertAlmostEqual(restored.sparse_values[0], 0.5, places=5)
        self.assertAlmostEqual(restored.sparse_values[1], 0.8, places=5)
        self.assertEqual(restored.fields, '{"name": "test"}')
        self.assertEqual(restored.expire_ns_ts, 1234567890)

    def test_delta_record_serialization(self):
        """测试 DeltaRecord 的序列化和反序列化"""
        record = DeltaRecord(
            type=DeltaRecord.Type.UPSERT,
            label=456,
            vector=[4.0, 5.0],
            sparse_raw_terms=["a", "b"],
            sparse_values=[0.1, 0.2],
            fields="new",
            old_fields="old",
        )

        serialized = record.serialize()
        restored = DeltaRecord.from_bytes(serialized)

        self.assertEqual(restored.type, DeltaRecord.Type.UPSERT)
        self.assertEqual(restored.label, 456)
        self.assertEqual(restored.vector, [4.0, 5.0])
        self.assertEqual(restored.fields, "new")
        self.assertEqual(restored.old_fields, "old")

    def test_ttl_data_serialization(self):
        """测试 TTLData 的序列化和反序列化"""
        ttl = TTLData(label=789)

        serialized = ttl.serialize()
        restored = TTLData.from_bytes(serialized)

        self.assertEqual(restored.label, 789)

    def test_default_values(self):
        """测试默认值"""
        data = CandidateData()

        serialized = data.serialize()
        restored = CandidateData.from_bytes(serialized)

        self.assertEqual(restored.label, 0)
        self.assertEqual(restored.vector, [])
        self.assertEqual(restored.fields, "")

    def test_empty_bytes(self):
        """测试空字节反序列化"""
        restored = CandidateData.from_bytes(b"")
        self.assertEqual(restored.label, 0)

    def test_schema_auto_generation(self):
        """测试 schema 自动生成"""
        # 验证 schema 和 bytes_row 已自动创建
        self.assertTrue(hasattr(CandidateData, "schema"))
        self.assertTrue(hasattr(CandidateData, "bytes_row"))

        # 验证字段数量
        self.assertEqual(len(CandidateData.schema.field_metas), 6)

        # 验证字段类型映射正确
        self.assertEqual(
            CandidateData.schema.field_metas["label"].data_type.name, "uint64"
        )
        self.assertEqual(
            CandidateData.schema.field_metas["vector"].data_type.name, "list_float32"
        )
        self.assertEqual(
            CandidateData.schema.field_metas["sparse_raw_terms"].data_type.name,
            "list_string",
        )


if __name__ == "__main__":
    unittest.main()
