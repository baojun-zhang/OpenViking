# 常见问题

关于 OpenViking 的常见问题解答。

## 基础问题

### OpenViking 是什么？

OpenViking 是一个面向 AI Agent 的上下文数据库。它遵循 "Data in, Context out" 理念 - 你添加各种数据源，OpenViking 为你的 AI 应用提供优化的上下文。

### OpenViking 和向量数据库有什么区别？

| 特性 | 向量数据库 | OpenViking |
|------|-----------|------------|
| 存储 | 仅向量 | 内容 + 向量 |
| 检索 | 相似度搜索 | 多层上下文 |
| 输出 | 原始分块 | 结构化上下文 (L0/L1/L2) |
| 记忆 | 不支持 | 内置记忆管理 |
| 会话 | 不支持 | 对话追踪 |

### OpenViking 是开源的吗？

是的，OpenViking 完全开源，采用 Apache 2.0 许可证。

## 安装问题

### 需要什么 Python 版本？

需要 Python 3.10 或更高版本。

### 如何安装 OpenViking？

```bash
pip install openviking
```

### 有哪些必需的依赖？

OpenViking 需要：
- Embedding 模型（如火山引擎 Doubao）
- 可选：VLM 用于多模态内容
- 可选：Rerank 模型用于提升检索质量

## 使用问题

### 如何初始化 OpenViking？

```python
import openviking as ov

client = ov.AsyncOpenViking(path="./my_data")
await client.initialize()
```

### 支持哪些文件格式？

- **文本**：`.txt`、`.md`、`.json`、`.yaml`
- **代码**：`.py`、`.js`、`.ts`、`.go`、`.java` 等
- **文档**：`.pdf`、`.docx`
- **图片**：`.png`、`.jpg`、`.jpeg`、`.gif`、`.webp`
- **视频**：`.mp4`、`.mov`、`.avi`
- **音频**：`.mp3`、`.wav`、`.m4a`

### 如何添加资源？

```python
# 单个文件
await client.add_resource("./document.pdf")

# 目录
await client.add_resource("./docs/", recursive=True)

# URL
await client.add_resource("https://example.com/page")
```

### `find` 和 `search` 有什么区别？

- `find`：简单语义搜索，返回匹配的上下文
- `search`：会话感知搜索，包含意图分析和查询重写

### 如何使用会话？

```python
session = client.session()
await session.add_message("user", [{"type": "text", "text": "你好"}])
await session.add_message("assistant", [{"type": "text", "text": "你好！"}])
await session.commit()  # 提取记忆
```

## 架构问题

### 什么是 L0/L1/L2 模型？

- **L0（摘要）**：约 100 tokens，简要摘要
- **L1（概览）**：约 2000 tokens，详细概览
- **L2（内容）**：完整原始内容

### 什么是 Viking URI？

Viking URI 是统一资源标识符：`viking://{scope}/{path}`

作用域：
- `resources/` - 知识库
- `user/memories/` - 用户记忆
- `agent/memories/` - Agent 记忆
- `skills/` - 可用技能

### 什么是 AGFS？

AGFS（Agent File System）是 OpenViking 的内容存储层，以层级结构组织数据和元数据。

## 性能问题

### OpenViking 能处理多少数据？

OpenViking 设计用于处理大型知识库。性能取决于：
- 硬件资源
- Embedding 模型速度
- 向量索引配置

### 如何提升检索质量？

1. 使用 Rerank 模型
2. 添加资源时提供有意义的 `reason`
3. 使用适当的 `target` URI 组织资源
4. 使用会话进行上下文感知搜索

## 故障排除

### 资源没有被索引

1. 检查是否调用了 `wait_processed()`
2. 验证 Embedding 模型配置
3. 检查日志中的处理错误

### 搜索没有返回结果

1. 验证资源已添加并处理完成
2. 检查 `target_uri` 过滤条件
3. 尝试更宽泛的搜索词

### 记忆提取不工作

1. 确保调用了 `session.commit()`
2. 检查用于记忆提取的 LLM 配置
3. 验证对话包含有意义的内容

## 相关文档

- [简介](getting-started/instruction.md)
- [快速开始](getting-started/quickstart.md)
- [架构概述](concepts/01-architecture.md)
