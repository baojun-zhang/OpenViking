# OpenViking 测试分类与执行说明

本文档用于说明 OpenViking 仓库中需要分别执行的测试模块，以及对应的本地执行命令。命令中的项目路径均使用绝对路径。

## 总原则

不要在仓库根目录直接执行全量：

```bash
cd /Users/bytedance/project/OpenViking
python3 -m pytest
```

当前仓库中存在多个带独立配置和导入路径假设的测试工程，例如：

- `/Users/bytedance/project/OpenViking/tests/api_test`
- `/Users/bytedance/project/OpenViking/tests/oc2ov_test`
- `/Users/bytedance/project/OpenViking/tests/cli`

这些目录应按各自入口分别执行。根目录全量收集容易触发 `conftest.py` 同名模块冲突、子工程 `tests` 包名冲突、以及缺少子工程专属配置等问题。

## Rust 测试

Rust workspace 位于：

```text
/Users/bytedance/project/OpenViking/Cargo.toml
```

当前 workspace 成员：

- `/Users/bytedance/project/OpenViking/crates/ragfs`
- `/Users/bytedance/project/OpenViking/crates/ragfs-python`
- `/Users/bytedance/project/OpenViking/crates/ov_cli`

### 1. Rust Workspace 全量测试

注意：当前 GitHub Actions 没有把 `cargo test --workspace` 作为 Rust CI 入口。仓库现有 CI 对 `ragfs-python` 采用 `cargo check -p ragfs-python` 和 `maturin build`，因此下面命令更适合作为本地扩展验证。

```bash
cd /Users/bytedance/project/OpenViking
cargo test --workspace
```

### 2. ragfs 核心库测试

```bash
cd /Users/bytedance/project/OpenViking
cargo test -p ragfs
```

仅做编译检查：

```bash
cd /Users/bytedance/project/OpenViking
cargo check -p ragfs
```

### 3. ragfs-python PyO3 绑定检查

当前 CI 对该包执行的是编译检查：

```bash
cd /Users/bytedance/project/OpenViking
cargo check -p ragfs-python
```

说明：当前 CI 不执行 `cargo test -p ragfs-python`。

```bash
cd /Users/bytedance/project/OpenViking
cargo test -p ragfs-python
```

如需构建 Python native extension：

```bash
cd /Users/bytedance/project/OpenViking/crates/ragfs-python
python -m maturin build --release --features s3
```

### 4. ov_cli Rust CLI 测试

```bash
cd /Users/bytedance/project/OpenViking
cargo test -p ov_cli
```

仅构建 CLI：

```bash
cd /Users/bytedance/project/OpenViking
cargo build -p ov_cli
```

说明：

- GitHub Actions 中的 `rust-cli.yml` 主要做多平台构建，不是主测试入口。
- 若 CLI 行为涉及服务端交互，应配合 `/Users/bytedance/project/OpenViking/tests/cli` 下的 Python CLI 集成测试一起验证。

## Python 测试

### 1. 主工程快速 Smoke 测试

用途：对应当前 GitHub Actions 中 `_test_lite.yml` 和 `_test_full.yml` 实际执行的轻量验证入口。

```bash
cd /Users/bytedance/project/OpenViking
uv run python /Users/bytedance/project/OpenViking/tests/integration/test_quick_start_lite.py
```

说明：

- 这是当前 CI 中替代全量 pytest 的主工程 smoke 测试。
- 适合改动后快速确认基础流程可用。

### 2. 主工程 Python 单测与轻集成

用途：运行主工程内不依赖 API 子工程、OC2OV 子工程、CLI 子工程的 Python 测试。

```bash
cd /Users/bytedance/project/OpenViking
python3 -m pytest \
  /Users/bytedance/project/OpenViking/tests \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test \
  --ignore=/Users/bytedance/project/OpenViking/tests/oc2ov_test \
  --ignore=/Users/bytedance/project/OpenViking/tests/cli
```

可按模块缩小范围：

```bash
cd /Users/bytedance/project/OpenViking
python3 -m pytest /Users/bytedance/project/OpenViking/tests/unit
python3 -m pytest /Users/bytedance/project/OpenViking/tests/session
python3 -m pytest /Users/bytedance/project/OpenViking/tests/storage
python3 -m pytest /Users/bytedance/project/OpenViking/tests/server
python3 -m pytest /Users/bytedance/project/OpenViking/tests/client
python3 -m pytest /Users/bytedance/project/OpenViking/tests/transaction
python3 -m pytest /Users/bytedance/project/OpenViking/tests/resource
python3 -m pytest /Users/bytedance/project/OpenViking/tests/retrieve
python3 -m pytest /Users/bytedance/project/OpenViking/tests/parse
python3 -m pytest /Users/bytedance/project/OpenViking/tests/misc
python3 -m pytest /Users/bytedance/project/OpenViking/tests/observability
python3 -m pytest /Users/bytedance/project/OpenViking/tests/telemetry
python3 -m pytest /Users/bytedance/project/OpenViking/tests/metrics
```

说明：

- 这些目录使用仓库根目录 `/Users/bytedance/project/OpenViking/pyproject.toml` 中的 pytest 配置。
- 某些 integration 测试依赖外部服务、API key 或本地配置，失败时应按具体 fixture 再拆分确认。

### 3. API 集成测试

用途：测试 OpenViking HTTP API、资源、检索、会话、系统状态等端到端接口。

必须进入 API 测试工程目录执行：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
python3 -m pytest /Users/bytedance/project/OpenViking/tests/api_test
```

更贴近 `.github/workflows/api_test.yml` 的 Unix API 测试命令分为两种。

有 secrets 时，CI 先串行运行 filesystem 和 resources retrieval 场景，再并行运行其余非 slow 测试：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
export OPENVIKING_API_KEY=test-root-api-key
export OPENVIKING_ROOT_API_KEY=test-root-api-key
export SERVER_URL=http://127.0.0.1:1933
uv run python -m pytest \
  /Users/bytedance/project/OpenViking/tests/api_test/filesystem \
  /Users/bytedance/project/OpenViking/tests/api_test/scenarios/resources_retrieval \
  -v --tb=short \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/filesystem/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios/resources_retrieval/slow
uv run python -m pytest /Users/bytedance/project/OpenViking/tests/api_test -v -n 4 \
  --html=api-test-report.html --self-contained-html \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios/resources_retrieval_slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/filesystem \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios/resources_retrieval \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/common/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/content/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/resources/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/retrieval/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/sessions/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/tasks/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/skills/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios/stability_error/slow
```

无 secrets 时，CI 先串行运行 filesystem 基础测试，再并行运行 basic 子集：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
export OPENVIKING_API_KEY=test-root-api-key
export OPENVIKING_ROOT_API_KEY=test-root-api-key
export SERVER_URL=http://127.0.0.1:1933
uv run python -m pytest /Users/bytedance/project/OpenViking/tests/api_test/filesystem -v --tb=short \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/retrieval \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/resources \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/admin \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/skills \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/system \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/filesystem/slow
uv run python -m pytest /Users/bytedance/project/OpenViking/tests/api_test -v -n 4 \
  --html=api-test-report.html --self-contained-html \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/retrieval \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/resources/test_pack.py \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/resources/test_wait_processed.py \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/admin \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/skills \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/system/test_system_status.py \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/system/test_is_healthy.py \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/system/test_system_wait.py \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/scenarios \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/filesystem \
  -k "not test_observer" \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/common/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/content/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/resources/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/sessions/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/tasks/slow \
  --ignore=/Users/bytedance/project/OpenViking/tests/api_test/skills/slow
```

有服务端依赖时，先启动本地服务：

```bash
cd /Users/bytedance/project/OpenViking
export ROOT_API_KEY=test-root-api-key
export SERVER_PORT=1933
uv run python -m openviking.server.bootstrap
```

另一个终端执行：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
export OPENVIKING_API_KEY=test-root-api-key
export OPENVIKING_ROOT_API_KEY=test-root-api-key
export SERVER_URL=http://127.0.0.1:1933
python3 -m pytest /Users/bytedance/project/OpenViking/tests/api_test
```

说明：

- `/Users/bytedance/project/OpenViking/tests/api_test/pytest.ini` 设置了 `pythonpath = .`，因此不能从仓库根目录混跑。
- 当前 API 测试内部存在 `from conftest import ...` 的裸导入，执行全量时可能受多个同名 `conftest.py` 影响；必要时应按子目录拆跑。
- `.github/workflows/api_test.yml` 中 Windows 分支也执行 API 测试，但会跳过 filesystem，本文本地命令以 Unix/Mac 执行为准。

### 4. API Slow / Effect 测试

用途：覆盖更重的 API 场景，例如深度资源处理、复杂检索、长会话、稳定性场景。

Light slow：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
python3 -m pytest \
  /Users/bytedance/project/OpenViking/tests/api_test/common/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/tasks/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/skills/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/retrieval/slow \
  -v -n 2 --tb=short --durations=0
```

Heavy slow：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
python3 -m pytest \
  /Users/bytedance/project/OpenViking/tests/api_test/content/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/resources/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/sessions/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/scenarios/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/scenarios/stability_error/slow \
  /Users/bytedance/project/OpenViking/tests/api_test/scenarios/resources_retrieval_slow \
  -v --tb=short --durations=0
```

说明：

- Heavy slow 建议串行执行，避免资源冲突。
- 这些测试通常需要本地 OpenViking server、模型配置或外部依赖。

### 5. CLI 集成测试

用途：测试 `ov` / `openviking` CLI 与服务端交互。

```bash
cd /Users/bytedance/project/OpenViking/tests/cli
export OPENVIKING_CLI_BIN=/Users/bytedance/project/OpenViking/openviking/bin/ov
export OPENVIKING_URL=http://127.0.0.1:1933
export OPENVIKING_API_KEY=test-root-api-key
export OPENVIKING_ROOT_API_KEY=test-root-api-key
uv run python -m pytest /Users/bytedance/project/OpenViking/tests/cli/test_cli_compatibility.py -v --tb=short -m cli_remote
```

CLI 集成主集合：

```bash
cd /Users/bytedance/project/OpenViking/tests/cli
export OPENVIKING_CLI_BIN=/Users/bytedance/project/OpenViking/openviking/bin/ov
export OPENVIKING_URL=http://127.0.0.1:1933
export OPENVIKING_API_KEY=test-root-api-key
export OPENVIKING_ROOT_API_KEY=test-root-api-key
uv run python -m pytest \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_filesystem.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_content.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_resources.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_search.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_sessions.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_relations.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_system.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_skills.py \
  /Users/bytedance/project/OpenViking/tests/cli/test_cli_observer.py \
  -v --tb=short -m cli_remote \
  --html=cli-test-report.html --self-contained-html
```

说明：

- CLI 测试需要可执行 CLI 二进制和正在运行的 OpenViking server。
- `/Users/bytedance/project/OpenViking/tests/cli/pytest.ini` 是独立配置，建议进入该目录执行。

### 6. OC2OV 测试

用途：OpenClaw 到 OpenViking 的记忆链路测试，属于独立测试工程。

首次运行前创建配置：

```bash
cp /Users/bytedance/project/OpenViking/tests/oc2ov_test/config/settings.example.py \
  /Users/bytedance/project/OpenViking/tests/oc2ov_test/config/settings.py
```

P0 测试：

```bash
cd /Users/bytedance/project/OpenViking/tests/oc2ov_test
export PYTHONPATH=/Users/bytedance/project/OpenViking:/Users/bytedance/project/OpenViking/tests/oc2ov_test
python3 -m pytest /Users/bytedance/project/OpenViking/tests/oc2ov_test/tests/p0 -v
```

其他 OC2OV 子集：

```bash
cd /Users/bytedance/project/OpenViking/tests/oc2ov_test
export PYTHONPATH=/Users/bytedance/project/OpenViking:/Users/bytedance/project/OpenViking/tests/oc2ov_test
python3 -m pytest /Users/bytedance/project/OpenViking/tests/oc2ov_test/tests/session -v
python3 -m pytest /Users/bytedance/project/OpenViking/tests/oc2ov_test/tests/skill -v
python3 -m pytest /Users/bytedance/project/OpenViking/tests/oc2ov_test/tests/advanced -v
python3 -m pytest /Users/bytedance/project/OpenViking/tests/oc2ov_test/tests/long_term -v
```

说明：

- 该工程依赖 `/Users/bytedance/project/OpenViking/tests/oc2ov_test/config/settings.py`。
- 需要同时包含项目根目录和 OC2OV 测试目录到 `PYTHONPATH`。

### 7. Bot 测试

用途：测试 `vikingbot` 相关逻辑。该部分不在当前主 GitHub Actions 测试流中，需要按 bot 依赖单独执行。

```bash
cd /Users/bytedance/project/OpenViking
python3 -m pytest /Users/bytedance/project/OpenViking/bot/tests
```

如需测试 vikingbot 包内部测试：

```bash
cd /Users/bytedance/project/OpenViking
python3 -m pytest /Users/bytedance/project/OpenViking/bot/vikingbot/tests
```

说明：

- Bot 测试可能需要安装 `openviking[bot]` 或相关 bot extra 依赖。
- 涉及外部 provider 的用例可能需要额外环境变量。

## C++ Engine 测试

虽然本文重点区分 Python 和 Rust，但仓库中仍保留了 C++ engine 测试。

```bash
cd /Users/bytedance/project/OpenViking/tests/engine
cmake -S /Users/bytedance/project/OpenViking/tests/engine \
  -B /Users/bytedance/project/OpenViking/tests/engine/build
cmake --build /Users/bytedance/project/OpenViking/tests/engine/build
/Users/bytedance/project/OpenViking/tests/engine/build/test_index_engine
```

## 推荐本地执行顺序

日常开发建议按以下顺序逐步扩大范围：

1. 快速 smoke：

```bash
cd /Users/bytedance/project/OpenViking
uv run python /Users/bytedance/project/OpenViking/tests/integration/test_quick_start_lite.py
```

2. 变更相关的主工程 Python 测试：

```bash
cd /Users/bytedance/project/OpenViking
python3 -m pytest /Users/bytedance/project/OpenViking/tests/<related-module>
```

3. 变更相关的 Rust package：

```bash
cd /Users/bytedance/project/OpenViking
cargo test -p <package-name>
```

4. 涉及 HTTP API 时，单独跑 API 测试：

```bash
cd /Users/bytedance/project/OpenViking/tests/api_test
python3 -m pytest /Users/bytedance/project/OpenViking/tests/api_test/<related-module>
```

5. 涉及 CLI 时，单独跑 CLI 测试：

```bash
cd /Users/bytedance/project/OpenViking/tests/cli
python3 -m pytest /Users/bytedance/project/OpenViking/tests/cli/<related-test-file>.py -m cli_remote
```

6. 涉及 OpenClaw/Memory 迁移链路时，单独跑 OC2OV：

```bash
cd /Users/bytedance/project/OpenViking/tests/oc2ov_test
export PYTHONPATH=/Users/bytedance/project/OpenViking:/Users/bytedance/project/OpenViking/tests/oc2ov_test
python3 -m pytest /Users/bytedance/project/OpenViking/tests/oc2ov_test/tests/p0 -v
```

## 常见问题

### 根目录 pytest 为什么失败

根目录 pytest 会收集所有 `tests/` 下的测试，包括独立子测试工程。常见失败原因：

- `/Users/bytedance/project/OpenViking/tests/api_test` 依赖自己的 `pytest.ini` 和 `pythonpath = .`。
- `/Users/bytedance/project/OpenViking/tests/oc2ov_test` 依赖自己的 `settings.py` 和 `PYTHONPATH`。
- 多个目录下存在同名 `conftest.py`，裸导入 `from conftest import ...` 可能解析到错误模块。
- 部分测试需要服务端、CLI 二进制、模型 API key、外部服务或本地配置。

### API 测试为什么需要 cd 到 tests/api_test

因为 `/Users/bytedance/project/OpenViking/tests/api_test/pytest.ini` 中配置了：

```ini
pythonpath = .
testpaths = .
```

从其他目录执行时，`from api.client import ...`、`from config import ...` 等导入可能无法按预期解析。

### OC2OV 测试为什么需要单独 PYTHONPATH

OC2OV 测试内部既需要访问项目根目录，又需要访问自己的 `config`、`utils`、`tests` 包。因此 CI 中使用：

```bash
export PYTHONPATH=/Users/bytedance/project/OpenViking:/Users/bytedance/project/OpenViking/tests/oc2ov_test
```

本地执行也应保持一致。
