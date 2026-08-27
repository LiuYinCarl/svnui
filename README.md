# svnui

一个受 [gitui](https://github.com/gitui-org/gitui) 启发的 SVN (Subversion) 终端 UI 客户端，使用 Rust + [ratatui](https://github.com/ratatui/ratatui) 编写。

![svnui 演示](https://github.com/LiuYinCarl/svnui/releases/download/assets/svnui.gif)

## 功能

覆盖日常最常用的 SVN 操作：

| 功能 | 说明 |
| --- | --- |
| **状态视图** | `svn status` 结果按目录树展示，M/A/D/C/? 状态彩色标识，目录可折叠/展开 |
| **分支信息** | 状态栏常驻显示当前分支，提交确认弹窗显示目的分支与将提交的文件列表 |
| **Diff 面板** | 选中文件自动显示 `svn diff`，带行号与 +/− 高亮；未版本化文件直接显示内容 |
| **暂存/提交** | `space` 暂存（加入提交集），`A` 全部暂存，`U` 全部取消；暂存未版本化文件会自动 `svn add`；提交集为空时拒绝提交。输入框支持中文/宽字符与多行粘贴，`Tab` 可回填最近提交信息 |
| **日志视图** | `svn log -v` 修订列表 + 变更路径与提交信息详情；`/` 按关键字搜索提交；`space` 标记多个修订后 `d`/`Enter` 查看合并 diff |
| **文件历史** | `t` 查看选中文件的 `svn log` 历史，弹窗内 `Enter` 直接看该修订 diff |
| **文件搜索** | `Ctrl+p` fzf 式模糊搜索文件，命中字符高亮，回车跳转到该文件的历史 |
| **Blame** | `svn blame` 按修订号着色显示 |
| **还原** | `svn revert`（带确认） |
| **更新** | `svn update`、更新到指定修订（`svn update -r N`，均带确认） |
| **冲突解决** | `svn resolve --accept=working`（带确认） |
| **过滤** | `/` 按路径过滤文件（状态页）/ 搜索提交（日志页） |
| **帮助** | `?` 查看全部快捷键 |
| **异步执行** | 所有 svn 命令在后台线程执行，UI 不卡顿，带 spinner 指示 |

## 安装与运行

```bash
# 需要 Rust 工具链和 svn 客户端
cargo build --release

# 在 SVN 工作副本中运行
svnui
# 或指定目录
svnui /path/to/working-copy
```

## 快捷键

| 按键 | 功能 |
| --- | --- |
| `q` | 退出 |
| `j` / `↓` / `k` / `↑` | 移动选择 |
| `h` / `←` / `l` / `→` | 折叠 / 展开目录 |
| `g` / `G` | 跳到第一项 / 最后一项 |
| `PgUp` / `PgDn` | 翻页 |
| `space` | 暂存 / 取消暂存（切换提交集） |
| `A` / `U` | 全部暂存 / 全部取消暂存 |
| `a` | `svn add` 选中的未版本化文件 |
| `r` | `svn revert` 选中的文件（确认后执行） |
| `x` | 解决冲突（采用工作副本版本） |
| `c` | 聚焦提交信息输入框 |
| `Enter` | 提交（输入框内） |
| `Tab` | 提交输入框内：列出最近提交信息，选中回填 |
| `u` | `svn update` |
| `d` | 全屏 Diff |
| `b` | Blame 文件 |
| `t` | 查看选中文件的提交历史 |
| `Ctrl+p` | 模糊搜索文件（回车查看文件历史） |
| `/` | 过滤文件（状态页）/ 搜索提交（日志页） |
| `F5` / `R` | 刷新状态 |
| `Tab` / `Shift+Tab` | 切换面板焦点 |
| `1` / `2` | 状态 / 日志 标签页 |
| `Enter` / `d` | 日志页：查看所选（或标记的多个）修订的 diff |
| `space` | 日志页：标记 / 取消标记修订 |
| `o` | 日志页：更新到所选修订 |
| `?` | 帮助 |
| `Esc` | 关闭弹窗 / 取消 |

## CI/CD

[![CI](https://github.com/LiuYinCarl/svnui/actions/workflows/ci.yml/badge.svg)](https://github.com/LiuYinCarl/svnui/actions/workflows/ci.yml)
[![Release](https://github.com/LiuYinCarl/svnui/actions/workflows/release.yml/badge.svg)](https://github.com/LiuYinCarl/svnui/actions/workflows/release.yml)

`.github/workflows/` 包含两条流水线：

- **ci.yml** — push / PR 时运行：fmt、clippy（零警告门禁）、Linux/macOS 全量测试、覆盖率门禁（≥ 80%）、三平台 release 构建。
- **release.yml** — 推送 `v*` 标签时运行：校验标签与 `Cargo.toml` 版本一致，在 Linux (x86_64)、macOS (arm64)、Windows (x86_64) 上构建 release 二进制并创建 GitHub Release。

### 发布新版本（Tag 触发 Release）

```bash
# 1. 更新 Cargo.toml 中的 version
cargo set-version 0.2.0          # 或手动编辑
# 2. 提交并打标签（标签 vX.Y.Z 必须与 Cargo.toml 版本一致）
git add -A && git commit -m "chore: release v0.2.0"
git tag v0.2.0
git push origin master --tags
```

## 性能（超大型 SVN 项目）

针对 10 万+ 文件的工作副本做了针对性优化：

- 文件树构建为 O(n)（一次 HashMap 装配）
- 树 / Diff / Blame 渲染虚拟化：每次绘制只处理可见窗口
- 目录暂存计数缓存，导航时零重算
- `cargo bench --bench tree`（criterion 基准）+ `cargo test perf`（CI 时间门禁，防止复杂度回归）

## 设计说明

架构参考了 gitui：

- `src/main.rs` — 终端初始化、事件循环（`crossbeam_channel::select` 多路复用输入 / 异步 svn 结果 / spinner 时钟）
- `src/app.rs` — 应用状态、标签页、弹窗栈、异步操作分发（对应 gitui 的 `App`/`Gitui`）
- `src/queue.rs` — 组件间事件队列（对应 gitui 的 `Queue` + `NeedsUpdate`）
- `src/svn/` — svn 命令行封装与输出解析（对应 gitui 的 `asyncgit`）：所有操作在线程中执行，通过通道回传结果
- `src/components/` — 文件树、Diff、日志、Blame、提交输入、帮助等组件（对应 gitui 的 `components/`）
- `src/popups/` — 确认、消息、输出查看、全屏 Diff 等弹窗（对应 gitui 的 `popups/`）
- `src/keys.rs` — 快捷键集中定义（对应 gitui 的 `keys/`）

SVN 没有 git 那样的暂存区，因此「暂存」被实现为**提交集**：标记要随下一次提交一起提交的文件；未版本化文件暂存时自动 `svn add`；提交集为空时拒绝提交。

## 测试

```bash
cargo test                 # 单元/集成测试（部分用例会创建真实临时 SVN 仓库）
cargo llvm-cov             # 覆盖率报告（需要 cargo-llvm-cov + llvm-tools-preview）
cargo clippy --all-targets # 零警告
```

测试策略：

- **解析器/模型**：构造 svn 输出样本做单元测试；
- **UI 组件**：用 `ratatui::TestBackend` 离屏渲染 + 合成 crossterm 事件驱动交互；
- **svn 命令层**：测试中 `svnadmin create` 临时仓库，真实执行 status/diff/log/blame/add/revert/commit/update/resolve；
- **App 状态机**：直接喂入 `AsyncSvnNotification` 与 `InternalEvent` 覆盖全部分支，含错误路径；
- **事件循环**：用 `run()` 泛型化 + TestBackend 驱动到退出。

已在 macOS (svn 1.14.5) 上验证完整流程。
