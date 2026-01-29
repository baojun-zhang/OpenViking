# FAQ

Frequently asked questions about OpenViking.

## General

### What is OpenViking?

OpenViking is a context database for AI Agents. It follows the "Data in, Context out" philosophy - you add various data sources, and OpenViking provides optimized context for your AI applications.

### How is OpenViking different from vector databases?

| Feature | Vector Database | OpenViking |
|---------|-----------------|------------|
| Storage | Vectors only | Content + Vectors |
| Retrieval | Similarity search | Multi-layer context |
| Output | Raw chunks | Structured context (L0/L1/L2) |
| Memory | Not supported | Built-in memory management |
| Sessions | Not supported | Conversation tracking |

### Is OpenViking open source?

Yes, OpenViking is fully open source under the Apache 2.0 license.

## Installation

### What Python version is required?

Python 3.10 or higher is required.

### How do I install OpenViking?

```bash
pip install openviking
```

### What are the required dependencies?

OpenViking requires:
- An embedding model (e.g., Volcengine Doubao)
- Optional: VLM for multimodal content
- Optional: Rerank model for improved retrieval

## Usage

### How do I initialize OpenViking?

```python
import openviking as ov

client = ov.AsyncOpenViking(path="./my_data")
await client.initialize()
```

### What file formats are supported?

- **Text**: `.txt`, `.md`, `.json`, `.yaml`
- **Code**: `.py`, `.js`, `.ts`, `.go`, `.java`, etc.
- **Documents**: `.pdf`, `.docx`
- **Images**: `.png`, `.jpg`, `.jpeg`, `.gif`, `.webp`
- **Video**: `.mp4`, `.mov`, `.avi`
- **Audio**: `.mp3`, `.wav`, `.m4a`

### How do I add resources?

```python
# Single file
await client.add_resource("./document.pdf")

# Directory
await client.add_resource("./docs/", recursive=True)

# URL
await client.add_resource("https://example.com/page")
```

### What's the difference between `find` and `search`?

- `find`: Simple semantic search, returns matching contexts
- `search`: Session-aware search with intent analysis and query rewriting

### How do I use sessions?

```python
session = client.session()
await session.add_message("user", [{"type": "text", "text": "Hello"}])
await session.add_message("assistant", [{"type": "text", "text": "Hi!"}])
await session.commit()  # Extract memories
```

## Architecture

### What is the L0/L1/L2 model?

- **L0 (Abstract)**: ~100 tokens, brief summary
- **L1 (Overview)**: ~2000 tokens, detailed overview
- **L2 (Content)**: Full original content

### What is a Viking URI?

Viking URI is a unified resource identifier: `viking://{scope}/{path}`

Scopes:
- `resources/` - Knowledge base
- `user/memories/` - User memories
- `agent/memories/` - Agent memories
- `skills/` - Available skills

### What is AGFS?

AGFS (Agent File System) is OpenViking's content storage layer that organizes data in a hierarchical structure with metadata.

## Performance

### How much data can OpenViking handle?

OpenViking is designed to handle large knowledge bases. Performance depends on:
- Hardware resources
- Embedding model speed
- Vector index configuration

### How do I improve retrieval quality?

1. Use a rerank model
2. Provide meaningful `reason` when adding resources
3. Organize resources with appropriate `target` URIs
4. Use sessions for context-aware search

## Troubleshooting

### Resources are not being indexed

1. Check if `wait_processed()` was called
2. Verify embedding model configuration
3. Check logs for processing errors

### Search returns no results

1. Verify resources were added and processed
2. Check the `target_uri` filter
3. Try broader search terms

### Memory extraction not working

1. Ensure `session.commit()` was called
2. Check LLM configuration for memory extraction
3. Verify conversation has meaningful content

## Related Documentation

- [Introduction](getting-started/instruction.md)
- [Quick Start](getting-started/quickstart.md)
- [Architecture](concepts/01-architecture.md)
