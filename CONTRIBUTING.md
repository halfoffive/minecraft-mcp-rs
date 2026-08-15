# Contributing / 参与贡献

Bilingual contribution guide for minecraft-mcp-rs. Covers the branch model
and the two-channel release pipeline (pre-release on `release`, stable on
`master`).

本项目贡献指南（双语）：分支模型与双通道发布流水线（`release` 预发布、`master` 稳定）。

## Branch model / 分支模型

| Branch / 分支 | Purpose / 用途 |
|---------------|----------------|
| `master` | Stable channel. Accepts merges from `release` only. Tagging `vX.Y.Z` here publishes a **stable** release: GitHub "Latest Release" + npm dist-tag `latest`. / 稳定通道。只接受来自 `release` 的合并。在此打 `vX.Y.Z` tag 发布**稳定版**：GitHub Latest Release + npm `latest`。 |
| `develop` | Integration branch for all `feat/*` / `fix/*` / `docs/*` pull requests. / 集成分支，所有功能/修复/文档 PR 的目标。 |
| `release` | Pre-release channel. Carries a pre-release Cargo version (`X.Y.Z-rc.N`); **every push auto-publishes a pre-release** (GitHub Release marked prerelease + npm dist-tag `next`). / 预发布通道。Cargo 版本带预发布后缀（`X.Y.Z-rc.N`）；**每次 push 自动发布预发布版**（GitHub Release 标记 prerelease + npm `next`）。 |

> Git forbids a `release` branch and `release/<X.Y.Z>` branches from
> coexisting (refs namespace conflict). Version-specific prep branches must
> use a different prefix (e.g. `hotfix/<X.Y.Z>`); the old `release/1.2.0`
> branch was deleted when this model was adopted.
>
> Git 不允许 `release` 分支与 `release/<X.Y.Z>` 分支共存（refs 命名空间冲突）。按版本
> 的准备分支需换前缀（如 `hotfix/<X.Y.Z>`）；启用本模型时已删除旧 `release/1.2.0` 分支。

## Workflow / 开发流程

1. **Feature work / 功能开发** — create `feat/<slug>` / `fix/<slug>` /
   `docs/<slug>` from `develop` (or a synced `master`) → open a PR targeting
   `develop` → **user review** → merge into `develop`. Never commit to
   `master` or `release` directly. / 从 `develop`（或已同步的 `master`）切功能分支 →
   开 PR（目标 `develop`）→ **用户审阅** → 合并进 `develop`。禁止直接提交 `master`/`release`。
2. **Pre-release / 预发布** — open a PR from `develop` into `release`. After
   review, on `release` bump `Cargo.toml` to `X.Y.Z-rc.N`, run
   `node npm/scripts/sync-versions.mjs`, add a CHANGELOG `[X.Y.Z-rc.N]`
   section, commit and push. `release.yml` then **automatically** builds all
   platforms, creates a GitHub Release (`prerelease: true`, never "Latest")
   and publishes the npm packages under dist-tag `next`. / 从 `develop` 向 `release`
   开 PR；审阅后，在 `release` 上把 `Cargo.toml` 改为 `X.Y.Z-rc.N`、运行
   `node npm/scripts/sync-versions.mjs`、添加 CHANGELOG `[X.Y.Z-rc.N]` 节，提交并
   push。`release.yml` 会**自动**构建全平台、创建 GitHub Release（prerelease，不占
   Latest）并以 `next` dist-tag 发布 npm 包。
   - Install a pre-release: `npm install -g minecraft-mcp-rs@next` /
     安装预发布：`npm install -g minecraft-mcp-rs@next`
3. **Stable release / 稳定发布** — open the release PR from `release` into
   `master`; set the final version `X.Y.Z` (re-run sync-versions), update the
   markdown `@<version>` pins and the MC compatibility table, finalize the
   CHANGELOG section → merge → on `master` create and push tag `vX.Y.Z` →
   `release.yml` publishes the **stable** release: GitHub "Latest Release" +
   npm dist-tag `latest`. Afterwards fast-forward `master` back into `develop`.
   / 从 `release` 向 `master` 开发布 PR；把版本定为正式 `X.Y.Z`（重跑 sync-versions）、
   更新 markdown 引脚与 MC 兼容表、定稿 CHANGELOG → 合并 → 在 `master` 打并推送
   `vX.Y.Z` → 发布**稳定版**（GitHub Latest + npm `latest`）。随后把 `master`
   快进合并回 `develop`。

## Release channel rules / 发布通道规则

- **Pre-release ⇔ `release` branch** — the version/tag must contain a hyphen
  (e.g. `v1.3.0-rc.1`); npm dist-tag `next`; GitHub Release marked
  `prerelease: true` and never "Latest". / 预发布 ⇔ `release` 分支：版本/tag 必须含
  连字符（如 `v1.3.0-rc.1`）；npm `next`；GitHub prerelease；不占据 Latest。
- **Stable ⇔ `master` tag `vX.Y.Z`** — pure semver with no hyphen; npm
  dist-tag `latest`; GitHub "Latest Release". / 稳定版 ⇔ `master` 上的纯 `vX.Y.Z`
  tag；npm `latest`；GitHub Latest Release。
- `workflow_dispatch` on `release.yml` still supports manual runs with a
  `prerelease` override. / `release.yml` 的 `workflow_dispatch` 仍支持手动运行与
  prerelease 覆盖。
- Idempotency guards: re-pushing `release` with an unchanged version skips
  the npm publish (version already published) and the GitHub Release
  (already exists). / 幂等守卫：版本未变时重复 push `release`，npm 跳过已发布版本、
  GitHub Release 跳过已存在版本。

## Repository settings (manual, recommended) / 仓库设置（人工，建议）

- Protect `master` and `release` under GitHub → Settings → Branches: require
  a PR with review for `master`; restrict pushes to `release` to maintainers
  (a push to `release` IS a publish action). / 在 GitHub 仓库设置中保护 `master`
  （要求 PR + 审阅）与 `release`（仅维护者可 push——对 `release` 的 push 即发布动作）。

## Conventions / 约定

- All code changes follow AGENTS.md (fmt → test → clippy → README/CHANGELOG/
  AGENTS updates; atomic commits; PR with user review). / 所有代码改动遵循
  AGENTS.md（fmt → test → clippy → 更新 README/CHANGELOG/AGENTS；原子提交；
  PR 经用户审阅）。
- Workflow-only changes require no cargo gates; validate the YAML syntax and
  review the trigger/mode logic by hand. / 仅工作流改动不跑 cargo 门禁；需校验
  YAML 语法并人工审阅触发/模式逻辑。
