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
| Dockerfile path | Both | `api_release/Dockerfile`, `Dockerfile` (at repo root) |
| Docker build context | Both | `api_release/`, `.` (repo root) |
| Any `sed` replacements needed | Deploy | `125.67.215.88:30800 → 172.16.29.88:30800` |
| Build args (`--build-arg`) | Deploy | `VITE_PRODUCT=gpu`, `VITE_API_ENDPOINT=...` |
| Pre-build steps | Deploy | `npm ci && npm run build`, Go compilation |
| Image name | Both | `<short_name>:latest` for deploy, `gpu_<short_name>:<version>` for release |
| Registry replacement | Both | Replace external registry with internal (if needed) |

## Environment Variables Available

Scripts receive these env vars from ru_deployer's Rust process:

| Variable | Deploy | Release | Value |
|---|---|---|---|
| `SRC_DIR` | ✅ | ✅ | Absolute path to cloned code |
| `GITLAB_PROJECT` | ✅ | ❌ | `"dev-team/api"` |
| `GITLAB_BRANCH` | ✅ | ❌ | `"main"` |
| `GITLAB_COMMIT` | ✅ | ❌ | Full SHA |
| `GITLAB_AUTHOR` | ✅ | ❌ | Username |
| `GITLAB_EVENT_ID` | ✅ | ❌ | Event ID |
| `VERSION` | ❌ | ✅ | Release tag name (e.g., `"v0.0.70"`) |
| `SCRIPTS_DIR` | ✅ | ✅ | Absolute path to scripts directory |
| `HARBOR_PASSWORD` | ✅ (via push_harbor.sh) | ✅ (via push_harbor.sh) | Harbor password |

## Template: `{project}_deploy.sh`

```bash
#!/bin/bash
# {project}_deploy.sh — 部署 dev-team/{project}
set -e

cd "${SRC_DIR}"
COMMIT=$(git rev-parse --short HEAD)

echo "[{project}] commit=${COMMIT}"

# Dockerfile registry replacement (if applicable)
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' {dockerfile_relative_path}

# Optional: other sed replacements
# sed -i 's|FROM debian:bookworm-slim|FROM ubuntu:24.04|g' {dockerfile_relative_path}

# Optional: pre-build steps
# npm ci && npm run build

# Docker build
docker build -t {project}:latest -f {dockerfile_relative_path} {build_context}

# Restart container (uncomment when deploying to target server)
# docker compose up -d {project}

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

**Pre-build frontend** (e.g., api-manager-platform):
```bash
# Compile frontend first
FRONTEND_DIR="$(find . -maxdepth 2 -name "package.json" -not -path "*/node_modules/*" | head -1 | xargs dirname 2>/dev/null || echo "")"
if [ -n "${FRONTEND_DIR}" ] && [ -f "${FRONTEND_DIR}/package.json" ]; then
    (cd "${FRONTEND_DIR}" && npm ci --production=false 2>/dev/null || npm install 2>/dev/null || true)
    (cd "${FRONTEND_DIR}" && npm run build 2>/dev/null || true)
fi
```

## Template: `{project}_release.sh`

```bash
#!/bin/bash
# {project}_release.sh — Release 构建 dev-team/{project}
# 环境变量: SRC_DIR, VERSION, HARBOR_PASSWORD, SCRIPTS_DIR
set -e

SCRIPT_DIR="${SCRIPTS_DIR:-$(cd "$(dirname "$0")" && pwd)}"

cd "${SRC_DIR}"
echo "[{project}_release] version=${VERSION}"

# Dockerfile registry replacement (if applicable)
sed -i 's|125\.67\.215\.88:30800|172.16.29.88:30800|g' {dockerfile_relative_path}

# Docker build with version tag
docker build -t "gpu_{project}:${VERSION}" -f {dockerfile_relative_path} {build_context}

# Push to Harbor (version tag + latest tag)
"${SCRIPT_DIR}/push_harbor.sh" "gpu_{project}:${VERSION}" "gpu_{project}" "${VERSION}"

echo "[{project}_release] done"
```

## Key Conventions

1. **Naming**: `<short_name>_deploy.sh` and `<short_name>_release.sh` — Rust code constructs these names from the project path's last segment
2. **No git operations**: Rust handles clone/fetch/checkout, script just uses `$SRC_DIR`
3. **No hardcoded tokens**: Harbor auth goes through `push_harbor.sh` which reads `$HARBOR_PASSWORD`
4. **`set -e`**: Always use, exit on first error
5. **Echo prefix**: Use `[{project}]` or `[{project}_release]` for easy log identification
6. **docker compose**: Commented out on dev machines; the comment `# 重启容器 (部署到测试机时取消注释)` marks it for uncommenting at deploy time
7. **No `push_harbor.sh` in deploy scripts**: Deploy scripts only build and restart; release scripts build, push to Harbor (version + latest), but do NOT restart
8. **File location**: All scripts go in `scripts/` directory at repo root
9. **Permissions**: `chmod +x` after creation
10. **docker-compose.yml**: Must be present in `scripts/` directory; when adding a new project, add its service definition there

## Filter Configuration

After creating scripts, update `filter.toml`:

```toml
[[repos]]
project = "dev-team/{project}"
branches = ["main"]        # branches to monitor for deploy
emails = ["team@do.top"]   # notification fallback
```

## Complete Checklist for Adding a New Project

1. [ ] Gather project info: Dockerfile path, build context, build args, registry replacements
2. [ ] Create `scripts/{project}_deploy.sh` using template above
3. [ ] Create `scripts/{project}_release.sh` using template above (if release needed)
4. [ ] `chmod +x scripts/{project}_deploy.sh scripts/{project}_release.sh`
5. [ ] Add service definition to `scripts/docker-compose.yml`
6. [ ] Add project to `filter.toml` with branches and emails
7. [ ] Verify: no hardcoded tokens, no git operations, `set -e`, correct Dockerfile paths
8. [ ] Test: run `cargo run -- --mode multi` and push to the project to verify detection and script execution

## Examples

### Simple project (api)
- Deploy: `scripts/api_deploy.sh` — single Dockerfile, registry sed, compose restart
- Release: `scripts/api_release.sh` — Dockerfile, push_harbor.sh with version tag

### Frontend project (horizon)
- Deploy: `scripts/horizon_deploy.sh` — Dockerfile with 7 `--build-arg`, no registry replacement
- No release script (frontend-only project)

### Complex project (api-manager-platform)
- Deploy: `scripts/api-manager-platform_deploy.sh` — Go binary + npm frontend, then Dockerfile
- No release script
