# ru_deployer Script Writing Skill

## Purpose

Guide agents to automatically generate `*_deploy.sh` and `*_release.sh` scripts for new projects in the ru_deployer system.

## When to Use

When adding a new project to ru_deployer's filter and it needs deploy/release automation scripts.

## Prerequisites

Before writing scripts, gather this information from the user or the project's repository:

| Information | Required for | Example |
|---|---|---|
| Project short name | Both | `api`, `flint`, `horizon` |
| Dockerfile path | Both | `api_release/Dockerfile.test`, `Dockerfile` (at repo root) |
| Docker build context | Both | `api_release/`, `.` (repo root) |
| Any `sed` replacements needed | Deploy | `125.67.215.88:30800 → 172.16.29.88:30800` |
| Build args (`--build-arg`) | Deploy | `VITE_PRODUCT=gpu`, `VITE_API_ENDPOINT=...` |
| Pre-build steps | Deploy | `npm ci && npm run build`, Go compilation |
| Image name | Both | `<short_name>:latest` for deploy, `gpu_<short_name>:<version>` for release |
| Registry replacement | Both | Replace external registry with internal (if needed) |

## Environment Variables Available

Rust injects ONLY these env vars into scripts (T1: config-driven, no hardcoding):

| Variable | Deploy | Release | Value |
|---|---|---|---|
| `GIT_BRANCH` | ✅ | ✅ (same as VERSION) | Branch name (push) / tag name (release), no `refs/heads/` prefix |
| `VERSION` | ✅ (same as GIT_BRANCH) | ✅ | Release tag name (e.g., `"v0.0.70"`) |
| `HARBOR_REGISTRY` | ✅ (via push_harbor.sh) | ✅ (via push_harbor.sh) | From `config.toml [harbor].registry` |
| `HARBOR_PROJECT` | ✅ (via push_harbor.sh) | ✅ (via push_harbor.sh) | From `config.toml [harbor].project` |
| `HARBOR_USER` | ✅ (via push_harbor.sh) | ✅ (via push_harbor.sh) | From `config.toml [harbor].user` |
| `HARBOR_PASSWORD` | ✅ (via push_harbor.sh) | ✅ (via push_harbor.sh) | From `[harbor].password` / `RU_DEPLOYER_HARBOR_PASSWORD` / `HARBOR_PASSWORD`; empty → skip push |

> **Important**: Rust does NOT inject `SRC_DIR` / `GITLAB_PROJECT` / `GITLAB_COMMIT` / `SCRIPTS_DIR` anymore (simplified in commit 349918e). Scripts must derive the source directory themselves:
> ```bash
> SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
> ROOT_DIR="$(dirname "$SCRIPT_DIR")"
> PROJECT="<short_name>"
> SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}"   # deploy; release 用 ${VERSION}
> ```

## Template: `{project}_deploy.sh`

```bash
#!/bin/bash
# {project}_deploy.sh — 部署 dev-team/{project}
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="{project}"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${GIT_BRANCH}"

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)
echo "[{project}] branch=${GIT_BRANCH} commit=${COMMIT}"

# Dockerfile registry replacement (if applicable)
# sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' {dockerfile_relative_path}

# Optional: other sed replacements
# sed -i 's|FROM debian:bookworm-slim|FROM ubuntu:24.04|g' {dockerfile_relative_path}

# Optional: pre-build steps
# npm ci && npm run build

# Docker build
docker build -t {project}:latest -f {dockerfile_relative_path} {build_context}

# Restart container (uncomment when deploying to target server)
# docker compose -p ru_deployer -f "${SCRIPT_DIR}/docker-compose.yml" up -d {project}

echo "[{project}] done"
```

### Special Cases

**Multiple `--build-arg`** (e.g., frontend projects like horizon):
```bash
docker build -t {project}:latest -f Dockerfile \
    --build-arg KEY1="value1" \
    --build-arg KEY2="value2" \
    .
```

**Pre-build frontend + Go** (e.g., api-manager-platform):
```bash
# Compile frontend first (fixed frontend/ dir)
if [ -f "frontend/package.json" ]; then
    (cd frontend && npm ci 2>/dev/null || npm install 2>/dev/null || true)
    (cd frontend && npm run build 2>/dev/null || true)
fi
# Compile Go backend
CGO_ENABLED=0 go build -ldflags="-s -w" -o server ./cmd/server
# Dockerfile.local: 本地先编译再打包
docker build -t {project}:latest -f Dockerfile.local .
```

## Template: `{project}_release.sh`

```bash
#!/bin/bash
# {project}_release.sh — Release 构建 dev-team/{project}
# Rust 注入: GIT_BRANCH/VERSION（tag 名）+ HARBOR_REGISTRY/PROJECT/USER/PASSWORD
set -e

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ROOT_DIR="$(dirname "$SCRIPT_DIR")"
PROJECT="{project}"
SRC_DIR="${ROOT_DIR}/src/${PROJECT}/${VERSION}"

cd "${SRC_DIR}"
echo "[{project}_release] version=${VERSION}"

# Dockerfile registry replacement (if applicable)
# sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' {dockerfile_relative_path}

# Docker build with version + latest tags
docker build -t "gpu_{project}:${VERSION}" -t "gpu_{project}:latest" -f {dockerfile_relative_path} {build_context}

# Push to Harbor (version + latest); 认证经 HARBOR_* 环境变量，无密码时静默跳过
"${SCRIPT_DIR}/push_harbor.sh" "gpu_{project}:${VERSION}" "gpu_{project}" "${VERSION}"

echo "[{project}_release] done"
```

## Key Conventions

1. **Naming**: `<short_name>_deploy.sh` and `<short_name>_release.sh` — Rust code constructs these names from the project path's last segment
2. **No git operations**: Rust handles clone/fetch/checkout, script derives `SRC_DIR` from `ROOT_DIR` + `GIT_BRANCH`/`VERSION`
3. **No hardcoded tokens/passwords**: Harbor auth goes through `push_harbor.sh` which reads `HARBOR_PASSWORD` (no default; empty → skip). Never hardcode credentials in scripts or compose files
4. **`set -e`**: Always use, exit on first error
5. **Echo prefix**: Use `[{project}]` or `[{project}_release]` for easy log identification
6. **docker compose**: Commented out on dev machines; the comment `# 重启容器 (部署到测试机时取消注释)` marks it for uncommenting at deploy time. Always use `docker compose -p ru_deployer -f "${SCRIPT_DIR}/docker-compose.yml" up -d <service>`
7. **No `push_harbor.sh` in deploy scripts**: Deploy scripts only build and restart; release scripts build, push to Harbor (version + latest), but do NOT restart
8. **File location**: All scripts go in `scripts/` directory at repo root; `chmod +x` after creation
9. **docker-compose.yml**: Must be present in `scripts/` directory; when adding a new project, add its service definition there
10. **`.gitignore`**: When adding a new project, append `src/<short_name>/` to `.gitignore` (git working dirs live under `src/` and must not be tracked)

## Filter Configuration

After creating scripts, update `filter.toml`:

```toml
[[repos]]
project = "dev-team/{project}"
branches = ["main"]        # branches to monitor for deploy
emails = ["team@do.top"]   # notification fallback (author email missing 时兜底)
```

## Complete Checklist for Adding a New Project

1. [ ] Gather project info: Dockerfile path, build context, build args, registry replacements
2. [ ] Create `scripts/{project}_deploy.sh` using template above
3. [ ] Create `scripts/{project}_release.sh` using template above (if release needed)
4. [ ] `chmod +x scripts/{project}_deploy.sh scripts/{project}_release.sh`
5. [ ] Add service definition to `scripts/docker-compose.yml`
6. [ ] Add project to `filter.toml` with branches and emails
7. [ ] Append `src/{project}/` to `.gitignore`
8. [ ] Verify: no hardcoded tokens, no git operations, `set -e`, correct Dockerfile paths
9. [ ] Test: run `cargo run -- --mode multi` and push to the project to verify detection and script execution

## Examples

### Simple project (api)
- Deploy: `scripts/api_deploy.sh` — single Dockerfile, registry sed, compose restart
- Release: `scripts/api_release.sh` — Dockerfile.test, version+latest tags, push_harbor.sh

### Frontend project (horizon)
- Deploy: `scripts/horizon_deploy.sh` — Dockerfile with 7 `--build-arg`, no registry replacement
- Release: `scripts/horizon_release.sh` — 同上 + `VITE_APP_VERSION=${VERSION}` build-arg, push_harbor.sh

### Complex project (api-manager-platform)
- Deploy: `scripts/api-manager-platform_deploy.sh` — npm frontend + Go binary, Dockerfile.local
- Release: `scripts/api-manager-platform_release.sh` — 同上 + version tags, push_harbor.sh
