# Configuration

OpenViking uses a JSON configuration file (`~/.openviking/ov.conf`) for settings.

## Configuration File

Create `~/.openviking/ov.conf` in your project directory:

```json
{
  "embedding": {
    "dense": {
      "provider": "volcengine",
      "api_key": "your-api-key",
      "model": "doubao-embedding-vision-250615",
      "dimension": 1024
    }
  },
  "vlm": {
    "provider": "volcengine",
    "api_key": "your-api-key",
    "model": "doubao-seed-1-8-251228"
  },
  "rerank": {
    "provider": "volcengine",
    "api_key": "your-api-key",
    "model": "doubao-rerank-250615"
  },
  "storage": {
    "agfs": {
      "backend": "local",
      "path": "./data"
    }
  }
}
```

## Configuration Examples

<details>
<summary><b>Volcengine (Doubao Models)</b></summary>

```json
{
  "embedding": {
    "dense": {
      "api_base" : "https://ark.cn-beijing.volces.com/api/v3",
      "api_key"  : "your-volcengine-api-key",
      "provider" : "volcengine",
      "dimension": 1024,
      "model"    : "doubao-embedding-vision-250615",
      "input": "multimodal"
    }
  },
  "vlm": {
    "api_base" : "https://ark.cn-beijing.volces.com/api/v3",
    "api_key"  : "your-volcengine-api-key",
    "provider" : "volcengine",
    "model"    : "doubao-seed-1-8-251228"
  }
}
```

</details>

<details>
<summary><b>OpenAI Models</b></summary>

```json
{
  "embedding": {
    "dense": {
      "api_base" : "https://api.openai.com/v1",
      "api_key"  : "your-openai-api-key",
      "provider" : "openai",
      "dimension": 3072,
      "model"    : "text-embedding-3-large"
    }
  },
  "vlm": {
    "api_base" : "https://api.openai.com/v1",
    "api_key"  : "your-openai-api-key",
    "provider" : "openai",
    "model"    : "gpt-4-vision-preview"
  }
}
```

</details>

## Configuration Sections

### embedding

Embedding model configuration for vector search, supporting dense, sparse, and hybrid modes.

#### Dense Embedding

```json
{
  "embedding": {
    "dense": {
      "provider": "volcengine",
      "api_key": "your-api-key",
      "model": "doubao-embedding-vision-250615",
      "dimension": 1024,
      "input": "multimodal"
    }
  }
}
```

**Parameters**

| Parameter | Type | Description |
|-----------|------|-------------|
| `provider` | str | `"volcengine"`, `"openai"`, `"vikingdb"`, or `"jina"` |
| `api_key` | str | API key |
| `model` | str | Model name |
| `dimension` | int | Vector dimension |
| `input` | str | Input type: `"text"` or `"multimodal"` |
| `batch_size` | int | Batch size for embedding requests |

**Available Models**

| Model | Dimension | Input Type | Notes |
|-------|-----------|------------|-------|
| `doubao-embedding-vision-250615` | 1024 | multimodal | Recommended |
| `doubao-embedding-250615` | 1024 | text | Text only |

With `input: "multimodal"`, OpenViking can embed text, images (PNG, JPG, etc.), and mixed content.

**Supported providers:**
- `openai`: OpenAI Embedding API
- `volcengine`: Volcengine Embedding API
- `vikingdb`: VikingDB Embedding API
- `jina`: Jina AI Embedding API

**vikingdb provider example:**

```json
{
  "embedding": {
    "dense": {
      "provider": "vikingdb",
      "model": "bge_large_zh",
      "ak": "your-access-key",
      "sk": "your-secret-key",
      "region": "cn-beijing",
      "dimension": 1024
    }
  }
}
```

**jina provider example:**

```json
{
  "embedding": {
    "dense": {
      "provider": "jina",
      "api_key": "jina_xxx",
      "model": "jina-embeddings-v5-text-small",
      "dimension": 1024
    }
  }
}
```

Available Jina models:
- `jina-embeddings-v5-text-small`: 677M params, 1024 dim, max seq 32768 (default)
- `jina-embeddings-v5-text-nano`: 239M params, 768 dim, max seq 8192

Get your API key at https://jina.ai

**Local deployment (GGUF/MLX):** Jina embedding models are open-weight and available in GGUF and MLX formats on [Hugging Face](https://huggingface.co/jinaai). You can run them locally with any OpenAI-compatible server (e.g. llama.cpp, MLX, vLLM) and point the `api_base` to your local endpoint:

```json
{
  "embedding": {
    "dense": {
      "provider": "jina",
      "api_key": "local",
      "api_base": "http://localhost:8080/v1",
      "model": "jina-embeddings-v5-text-nano",
      "dimension": 768
    }
  }
}
```

#### Sparse Embedding

```json
{
  "embedding": {
    "sparse": {
      "provider": "volcengine",
      "api_key": "your-api-key",
      "model": "bm25-sparse-v1"
    }
  }
}
```

#### Hybrid Embedding

Two approaches are supported:

**Option 1: Single hybrid model**

```json
{
  "embedding": {
    "hybrid": {
      "provider": "volcengine",
      "api_key": "your-api-key",
      "model": "doubao-embedding-hybrid",
      "dimension": 1024
    }
  }
}
```

**Option 2: Combine dense + sparse**

```json
{
  "embedding": {
    "dense": {
      "provider": "volcengine",
      "api_key": "your-api-key",
      "model": "doubao-embedding-vision-250615",
      "dimension": 1024
    },
    "sparse": {
      "provider": "volcengine",
      "api_key": "your-api-key",
      "model": "bm25-sparse-v1"
    }
  }
}
```

### vlm

Vision Language Model for semantic extraction (L0/L1 generation).

```json
{
  "vlm": {
    "api_key": "your-api-key",
    "model": "doubao-seed-1-8-251228",
    "base_url": "https://ark.cn-beijing.volces.com/api/v3"
  }
}
```

**Parameters**

| Parameter | Type | Description |
|-----------|------|-------------|
| `api_key` | str | API key |
| `model` | str | Model name |
| `base_url` | str | API endpoint (optional) |

**Available Models**

| Model | Notes |
|-------|-------|
| `doubao-seed-1-8-251228` | Recommended for semantic extraction |
| `doubao-pro-32k` | For longer context |

When resources are added, VLM generates:

1. **L0 (Abstract)**: ~100 token summary
2. **L1 (Overview)**: ~2k token overview with navigation

If VLM is not configured, L0/L1 will be generated from content directly (less semantic), and multimodal resources may have limited descriptions.

### rerank

Reranking model for search result refinement.

```json
{
  "rerank": {
    "provider": "volcengine",
    "api_key": "your-api-key",
    "model": "doubao-rerank-250615"
  }
}
```

| Parameter | Type | Description |
|-----------|------|-------------|
| `provider` | str | `"volcengine"` |
| `api_key` | str | API key |
| `model` | str | Model name |

If rerank is not configured, search uses vector similarity only.

### storage

Storage backend configuration.

```json
{
  "storage": {
    "workspace": "./data",
    "agfs": {
      "port": 1833,
      "log_level": "warn",
      "backend": "local",
      "timeout": 10,
      "retry_times": 3,
      "s3": {
        "bucket": null,
        "region": null,
        "access_key": null,
        "secret_key": null,
        "endpoint": null,
        "prefix": "",
        "use_ssl": true
      }
    },
    "vectordb": {
      "name": "context",
      "backend": "local",
      "project": "default",
      "volcengine": {
        "region": "cn-beijing",
        "ak": null,
        "sk": null
      }
    }
  }
}
```

**Root Configuration**

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `workspace` | str | Local data storage path (primary), overrides `agfs.path` and `vectordb.path` | "./data" |
| `params` | dict | Additional storage-specific parameters | {} |

**agfs Configuration**

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `path` | str | [Deprecated, use workspace] AGFS data storage path | null |
| `port` | int | AGFS service port | 1833 |
| `log_level` | str | AGFS log level | "warn" |
| `url` | str | AGFS service URL for service mode | "http://localhost:1833" |
| `backend` | str | AGFS storage backend: 'local' 's3' 'memory' | "local" |
| `timeout` | int | AGFS request timeout (seconds) | 10 |
| `retry_times` | int | AGFS retry times on failure | 3 |
| `use_ssl` | bool | Enable/Disable SSL (HTTPS) for AGFS service | true |
| `s3` | object | S3 backend configuration (when backend is 's3') | - |

**agfs.s3 Configuration**

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `bucket` | str | S3 bucket name | null |
| `region` | str | AWS region where the bucket is located (e.g., us-east-1, cn-beijing) | null |
| `access_key` | str | S3 access key ID | null |
| `secret_key` | str | S3 secret access key | null |
| `endpoint` | str | Custom S3 endpoint URL | null |
| `prefix` | str | Optional key prefix for namespace isolation | "" |
| `use_ssl` | bool | Enable/Disable SSL (HTTPS) for S3 connections | true |
| `use_path_style` | bool | true for PathStyle (MinIO), false for VirtualHostStyle (TOS) | true |

<details>
<summary><b>Volcengine TOS</b></summary>

```json
{
 "agfs": {
      "port": 8080,
      "log_level": "warn",
      "path": "/local",
      "backend": "s3",
      "url": "http://localhost:8080",
      "timeout": 10,
      "retry_times": 3,
      "s3": {
        "bucket": "ov-bucket",
        "endpoint": "http://tos-s3-cn-beijing.volces.com",
        "region": "cn-beijing",
        "access_key": "your-access-key",
        "secret_key": "your-secret-key",
        "prefix": "",
        "use_ssl": false,
        "use_path_style": false
      }
  }
}
```
</details>

**vectordb Configuration**

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `backend` | str | VectorDB backend type: 'local' (file-based), 'http' (remote service), 'volcengine' (VikingDB), or 'vikingdb' (private deployment) | "local" |
| `name` | str | Collection name for VectorDB | "context" |
| `path` | str | [Deprecated, use workspace] Local storage path for 'local' type | null |
| `url` | str | Remote service URL for 'http' type (e.g., 'http://localhost:5000') | null |
| `project_name` | str | Project name (alias project) | "default" |
| `distance_metric` | str | Distance metric for vector similarity search (e.g., 'cosine', 'l2', 'ip') | "cosine" |
| `dimension` | int | Dimension of vector embeddings | 0 |
| `sparse_weight` | float | Sparse weight for hybrid vector search | 0.0 |
| `volcengine` | object | VikingDB configuration for 'volcengine' type | - |
| `vikingdb` | object | Private deployment configuration for 'vikingdb' type | - |

**vectordb.volcengine Configuration**

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `ak` | str | Volcengine access key | null |
| `sk` | str | Volcengine secret key | null |
| `region` | str | Volcengine region (e.g., 'cn-beijing') | null |
| `host` | str | Volcengine VikingDB host (optional) | null |

**vectordb.vikingdb Configuration**

| Parameter | Type | Description | Default |
|-----------|------|-------------|---------|
| `host` | str | VikingDB service host | null |
| `headers` | dict | Custom headers for requests | {} |

## Config Files

OpenViking uses two config files:

| File | Purpose | Default Path |
|------|---------|-------------|
| `ov.conf` | SDK embedded mode + server config | `~/.openviking/ov.conf` |
| `ovcli.conf` | HTTP client and CLI connection to remote server | `~/.openviking/ovcli.conf` |

When config files are at the default path, OpenViking loads them automatically — no additional setup needed.

If config files are at a different location, there are two ways to specify:

```bash
# Option 1: Environment variable
export OPENVIKING_CONFIG_FILE=/path/to/ov.conf
export OPENVIKING_CLI_CONFIG_FILE=/path/to/ovcli.conf

# Option 2: Command-line argument (serve command only)
python -m openviking serve --config /path/to/ov.conf
```

### ov.conf

The config sections documented above (embedding, vlm, rerank, storage) all belong to `ov.conf`. SDK embedded mode and server share this file.

### ovcli.conf

Config file for the HTTP client (`SyncHTTPClient` / `AsyncHTTPClient`) and CLI to connect to a remote server:

```json
{
  "url": "http://localhost:1933",
  "api_key": "your-secret-key",
  "agent_id": "my-agent",
  "output": "table"
}
```

| Field | Description | Default |
|-------|-------------|---------|
| `url` | Server address | (required) |
| `api_key` | API key for authentication (root key or user key) | `null` (no auth) |
| `agent_id` | Agent identifier for agent space isolation | `null` |
| `output` | Default output format: `"table"` or `"json"` | `"table"` |

See [Deployment](./03-deployment.md) for details.

## server Section

When running OpenViking as an HTTP service, add a `server` section to `ov.conf`:

```json
{
  "server": {
    "host": "0.0.0.0",
    "port": 1933,
    "root_api_key": "your-secret-root-key",
    "cors_origins": ["*"]
  }
}
```

| Field | Type | Description | Default |
|-------|------|-------------|---------|
| `host` | str | Bind address | `0.0.0.0` |
| `port` | int | Bind port | `1933` |
| `root_api_key` | str | Root API key for multi-tenant auth, disabled if not set | `null` |
| `cors_origins` | list | Allowed CORS origins | `["*"]` |

When `root_api_key` is configured, the server enables multi-tenant authentication. Use the Admin API to create accounts and user keys. When not set, the server runs in dev mode with no authentication.

For startup and deployment details see [Deployment](./03-deployment.md), for authentication see [Authentication](./04-authentication.md).

## Full Schema

```json
{
  "embedding": {
    "dense": {
      "provider": "volcengine",
      "api_key": "string",
      "model": "string",
      "dimension": 1024,
      "input": "multimodal"
    }
  },
  "vlm": {
    "provider": "string",
    "api_key": "string",
    "model": "string",
    "base_url": "string"
  },
  "rerank": {
    "provider": "volcengine",
    "api_key": "string",
    "model": "string"
  },
  "storage": {
    "workspace": "string",
    "params": {},
    "agfs": {
      "backend": "local|s3|memory",
      "path": "string",
      "port": 1833,
      "log_level": "warn",
      "url": "string",
      "timeout": 10,
      "retry_times": 3,
      "use_ssl": true,
      "s3": {
        "bucket": "string",
        "region": "string",
        "access_key": "string",
        "secret_key": "string",
        "endpoint": "string",
        "prefix": "string",
        "use_ssl": true,
        "use_path_style": true
      }
    },
    "vectordb": {
      "backend": "local|http|volcengine|vikingdb",
      "name": "string",
      "path": "string",
      "url": "string",
      "project": "string",
      "distance_metric": "string",
      "dimension": 0,
      "sparse_weight": 0.0,
      "volcengine": {
        "ak": "string",
        "sk": "string",
        "region": "string",
        "host": "string"
      },
      "vikingdb": {
        "host": "string",
        "headers": {}
      }
    }
  },
  "server": {
    "host": "0.0.0.0",
    "port": 1933,
    "root_api_key": "string",
    "cors_origins": ["*"]
  }
}
```

Notes:
- `storage.vectordb.sparse_weight` controls hybrid (dense + sparse) indexing/search. It only takes effect when you use a hybrid index; set it > 0 to enable sparse signals.

## Troubleshooting

### API Key Error

```
Error: Invalid API key
```

Check your API key is correct and has the required permissions.

### Vector Dimension Mismatch

```
Error: Vector dimension mismatch
```

Ensure the `dimension` in config matches the model's output dimension.

### VLM Timeout

```
Error: VLM request timeout
```

- Check network connectivity
- Increase timeout in config
- Try a smaller model

### Rate Limiting

```
Error: Rate limit exceeded
```

Volcengine has rate limits. Consider batch processing with delays or upgrading your plan.

## Related Documentation

- [Volcengine Purchase Guide](./volcengine-purchase-guide.md) - API key setup
- [API Overview](../api/01-overview.md) - Client initialization
- [Server Deployment](./03-deployment.md) - Server configuration
- [Context Layers](../concepts/03-context-layers.md) - L0/L1/L2
