# agent.md

面向 AI 编码助手 / 贡献者的仓库指南。先读这个文件，再动代码。

## 项目概况

`svnui` 是一个受 [gitui](https://github.com/gitui-org/gitui) 启发的 SVN 终端 UI 客户端，
用 Rust + ratatui 编写。

**关键约束（务必遵守）：**

- 这是 **SVN** 项目，不是 git。所有版本控制操作通过 `svn` CLI 完成（见 `src/svn/mod.rs`），
  代码里绝不能出现 `git` 命令或 git 语义（索引、HEAD、rebase 等）。
- SVN **没有暂存区**。"暂存"被实现为**提交集**（`StatusTreeComponent::staged: HashSet<String>`）：
  只有加入提交集的路径才会随下一次 `svn commit` 提交；未版本化（`?`）文件暂存时自动
  `svn add`；提交集为空时拒绝提交（确认弹窗前与 `perform_confirmed` 双重拦截）。
- 禁止使用 `unsafe`。新代码必须通过 `cargo clippy --all-targets -- -D warnings`（CI 门禁）。

## 目录结构

```
src/
├── main.rs           入口：终端初始化（raw mode + alternate screen + bracketed paste）、
│                     事件循环（crossbeam select 多路复用输入/异步/时钟；输入线程丢弃
│                     KeyEventKind::Release 事件）
├── app.rs            App 状态机：标签页、弹窗栈、异步通知分发、绘制入口
├── queue.rs          InternalEvent 队列（组件→App 的通信）+ NeedsUpdate 位标志
├── status.rs         Status 标签页聚合（文件树 + Diff 面板 + 提交输入栏 + 焦点切换）
├── keys.rs           快捷键集中定义（key_match(ev, KeyAction) -> bool）
├── strings.rs        用户可见字符串（集中管理）
├── svn/
│   ├── mod.rs        Svn 客户端：所有命令在后台线程执行，经 channel 回传 AsyncSvnNotification
│   ├── parser.rs     svn status/log/blame/diff 纯文本解析器（纯函数、无 IO）
│   └── models.rs     数据结构：StatusEntry / LogEntry / BlameLine / ParsedDiff ...
├── components/
│   ├── mod.rs        DrawableComponent trait、EventState、Context
│   ├── status_tree.rs  文件树（目录折叠、暂存、路径过滤）——最大的组件
│   ├── diff_view.rs  可滚动 Diff 视图（状态页与全屏弹窗共用；h/l 横向滚动看长行）
│   ├── commit.rs     提交信息输入栏（tui-textarea-2，Tab 回填最近提交信息）
│   ├── log.rs        日志标签页（修订列表滚动分页 + 详情 + 关键字筛选 + 多选合并 diff）
│   ├── log_search.rs 提交搜索弹窗（输入时实时筛选，回车 svn log --search 全历史搜索）
│   ├── status_filter.rs 状态文件过滤弹窗（输入时实时筛选，回车保留，Esc 还原；Esc 在状态页清除筛选）
│   ├── file_log.rs   单文件历史弹窗（svn log -v -- path）
│   ├── file_finder.rs  模糊文件搜索弹窗（数据源 svn list -R .@HEAD，fuzzy-matcher 匹配）
│   ├── blame.rs      blame 弹窗（/ 增量搜索 + n/N 跳转 + 光标行 Enter 看该修订 diff + h/l 横滚）
│   ├── patches.rs    补丁标签页（列出 patch 目录、预览/应用/删除补丁；patch_dir() 解析存储目录，
│   │                 SVNUI_PATCH_DIR 可覆盖）
│   ├── text_search.rs 可复用的增量搜索状态（diff/blame 弹窗共用）
│   └── help.rs       帮助弹窗
├── popups/           确认 / 消息 / 输出查看 / 全屏 Diff 弹窗（enum Popup 分发，无 downcast）
├── ui/               渲染辅助（滚动、行绘制、弹窗矩形）+ 主题
└── test_support.rs   测试支撑：临时 SVN 仓库（TestRepo）+ TestBackend 渲染辅助
.github/workflows/     ci.yml（fmt/clippy/test/coverage/build/stress——压测 job 浅克隆轮换的
                       开源仓库经 git2svn 转换后无头驱动）+ bump.yml（push 自动 bump
                       补丁版本并发版）+ release.yml（Tag 触发发布，也供 bump.yml 调用）
```

## 架构要点（与 gitui 的对应关系）

| svnui | gitui |
|---|---|
| `src/app.rs` | `App` + `Gitui` |
| `src/queue.rs` | `Queue` + `NeedsUpdate` |
| `src/svn/`（线程 + channel） | `asyncgit` crate |
| `src/components/*` | `components/` |
| `src/popups/*` | `popups/` |
| `src/keys.rs` | `keys/` |

**事件流：**
1. 输入事件 → `App::handle_input` → 弹窗栈优先 → 当前标签页 → App 级快捷键；
2. 组件把动作 push 到 `Queue`（`InternalEvent`），App 在 `handle_queue_events` 中消费；
3. SVN 操作全部异步：`Svn::status()` 等在后台线程跑 `svn`，结果经 `AsyncSvnNotification`
   回到 `App::handle_async`；`pending` 计数驱动 spinner。

## 性能（超大型工作副本）

目标：10 万+ 文件的项目不卡死。核心措施：

- `build_tree` 为 O(n)：一次性 HashMap 建节点 + 索引装配 + 排序；
- 虚拟化渲染：文件树 / Diff / Blame 每次 draw 只构建可见窗口（O(屏幕高度)）；
- `dir_staged_counts` 缓存：按目录的暂存计数只在暂存集合或条目变化时重算；
- `cargo test perf` 是 CI 强制的带时间预算回归测试，复杂度回归会被门禁拦下；
  `cargo bench --bench tree`（criterion，release）提供本地精确数据。

## 常用命令

```bash
cargo build                 # 开发构建
cargo test                  # 全量测试（部分用例会创建真实临时 SVN 仓库）
cargo bench --bench tree    # criterion 性能基准
cargo test <name>           # 跑单个测试
cargo llvm-cov --fail-under-lines 80   # 覆盖率门禁（与 CI 一致）
cargo clippy --all-targets -- -D warnings   # 零警告门禁
cargo fmt --all --check     # 格式门禁
```

## 编码约定

- 错误处理：内部使用 `Result<_, String>`（svn stderr 文本），UI 层显示在消息弹窗。
- 组件实现 `DrawableComponent`（`draw(&self, ...)` + `event(&mut self, ...)`）；
  `draw` 是 `&self`——需要可变状态（滚动偏移等）用 `Cell`。
- 文本输入用 `tui-textarea-2`，不要手写字符宽度计算。
- 组件不应直接持有 `Svn`；通过 `Context.queue` push `InternalEvent`，由 App 执行命令。
- 新增快捷键：在 `keys.rs` 加 `KeyAction` 变体 + `key_match` 分支，并更新 `all_binding_groups()`（帮助页，按上下文分组）。
- 新增用户可见文本：加到 `strings.rs`。
- 弹窗加进 `popups/mod.rs` 的 `enum Popup`，不要在 App 里用 downcast。
- 依赖策略：简单逻辑手写，不引入新 crate；只有复杂且易错的逻辑才引入流行的纯 Rust
  库——需无 unsafe、无 build script、维护活跃。
- 测试也不要碰进程环境变量（edition 2024 下 `env::set_var` 是 unsafe fn，本仓库禁用
  unsafe）：像 `patch_dir` 这类依赖环境的逻辑要拆出纯函数（如 `resolve_patch_dir`），
  测试直接给纯函数传参。

## SVN 输出格式要点（解析器已处理，改代码前先读）

- **最低版本 1.8**(`svn::MIN_SVN_VERSION`)：由用到的最新特性决定——`svn log --search`
  (1.8)、`svn patch`(1.7)。启动时 `App::start` 异步发 `check_info` + `version` 两个请求，
  版本低于门槛时弹非致命警告（`AsyncSvnNotification::Version` 失败本身不致命，svn 缺失由
  check_info 致命报错）。客户端版本同时显示在 `i` 仓库概览弹窗里。
- 所有 svn 子进程（含测试辅助）只能从 `Svn::run_in` / `test_support::svn` 发起，
  它们固定 `env_remove("LC_ALL")` + `LC_MESSAGES=C` 保证英文输出。**禁止改成 `LC_ALL=C`**
  （会把 native 编码固定为 ASCII，破坏非 ASCII 的提交与日志显示）。
- `svn commit` 必须带 `--encoding UTF-8`（`-m` 消息是 UTF-8 字节）。
- `svn status`：7 列状态字符后接路径（路径从第 8 个字符开始，验证于 svn 1.14.5）。
- `svn log`：必须显式传范围 `-r HEAD:0 -l N`（`HEAD:1` 在 r0 空仓库上报 E160006）。
- `svn log --search` **不能带 `-l`**：与 `--search` 同用时 `-l N` 限制的是**扫描的修订数**
  （最新 N 条），而不是返回的匹配数——带上它搜索就退化成"只搜最近 N 条"。
  `--search` 是 glob 语法、大小写不敏感，匹配作者/日期/提交信息/变更路径。
- `svn diff` 对未版本化文件返回 E155010 错误而不是空输出——`Svn::diff` 回退为直接读文件内容。
- `svn blame` 格式：`%6s %10s %s`——修订号右对齐**至少** 6 字节（7 位修订号会溢出右移），
  作者字段**恰好** 10 字节（超长直接字节级截断，可能切断多字节字符）。必须按固定字节列
  解析（`parse_blame(&[u8])`)，否则含空格的作者名会被当成内容；各字段单独 lossy 解码，
  截断乱码只影响作者不影响内容。作者真名靠同线程追加的 `svn blame --xml` 合并
  （`parse_blame_xml` + `merge_blame_authors`;XML 里没有行内容，所以两份都要）。
- `svn info` 的 `Revision` 是工作副本根目录的 BASE 修订（SVN 是混合修订工作副本）；
  分支名取 `Relative URL`（去掉 `^/`）。
- `svn list -R` 必须带 `.@HEAD` peg，否则按 wc 根目录的 BASE 修订列举。
- 文件名含 `@`（如 systemd 的 `foo@.service`）：log/blame/info/add/revert/resolve/commit
  都必须给路径追加空 peg（`path@`，见 `Svn::peg`），否则报 E205000/E200009；
  **`svn diff` 例外**——它根本不接受带 peg 的 wc 目标（E155010），但不带 peg 时会把
  无效的 `@xxx` 后缀当普通路径处理，所以 diff 不传 peg（验证于 svn 1.14.5）。
- `svn commit -m "docs"` 这类"像路径"的消息会报 E205005，需要 `--force-log` 或用更长文本。
- `svn revert` 一个刚 add 的文件：取消 add，文件留在磁盘上（变成 `?`）。

## 测试策略

- **解析器/模型**：构造输出样本做单元测试。
- **UI 组件**：`test_support::render` 用 `ratatui::TestBackend` 离屏绘制 + 合成 crossterm 事件。
- **svn 命令层**：`TestRepo` 用 `svnadmin create` 建临时仓库，真实执行
  status/diff/log/blame/add/revert/commit/update/resolve（本机无 svn 时测试自动跳过）。
- **App 状态机**：直接构造 `AsyncSvnNotification` / `InternalEvent` 喂给
  `handle_async` / `handle_queue_events`，覆盖 Ok/Err 全部分支。
- **事件循环**：`run()` 已泛型化，可用 `TestBackend` 驱动到退出。

## 压力测试（stress harness）

`scripts/stress_test.sh` + `tests/stress.rs`：用 git2svn 把真实 git 仓库（默认
`~/dev/github/openless` 当前分支；`STRESS_GIT_URL` 可 shallow 克隆远程仓库，CI 的
stress job 用它在 redis/clap/slugify 间轮换）转成 `target/tmp/stress/svn-repo`（约 1-2
分钟，先 wipe 再重建），检出 `target/tmp/stress/wc`，然后以无头方式驱动真实的 `App`：
合成 crossterm 按键走与 main.rs 完全相同的泵（input → handle_queue_events →
maybe_request_diff；异步通知 → handle_async → …），确定性 PRNG（xorshift64*，
无 rand 依赖）随机执行滚动/提交信息/修订 diff + 页内搜索/文件查找 + blame + 搜索/
日志全历史搜索/改文件 + 提交或还原/存删补丁/F5 刷新（偶发 svn update）/repo info(i)/
日志标记(space)/文件历史(t)。每轮断言：无 panic、`pending` 在超时内归零、无意外
错误弹窗、无残留弹窗；失败信息带 seed 可复现。harness 挑选修改目标时跳过符号链接
（append 会穿透到目标文件，破坏逐文件追踪）。

```bash
scripts/stress_test.sh                    # 完整跑（默认 200 轮）
SVNUI_STRESS_ROUNDS=30 scripts/stress_test.sh   # 快速验证
```

环境变量：`STRESS_GIT_REPO` / `STRESS_GIT_BRANCH`（转换哪个 git 仓库/分支）、
`SVNUI_STRESS_ROUNDS`、`SVNUI_STRESS_SEED`、`GIT2SVN_DIR`。
**CI 不会运行它**：`tests/stress.rs` 仅在 `SVNUI_STRESS=1` 且 `SVNUI_STRESS_WC`
指向合法工作副本时才真正执行，否则直接跳过（pass）。

注意事项：
- 曾发现 go-git 并发读 packfile 竞争导致转换偶发 "object not found"，
  已在 git2svn 侧修复（f7011b6，每个 worker 独立的 Repository 句柄）；
  若用旧版 git2svn，可加 `GOMAXPROCS=1` 规避。
- 想要"恰好约 1000 提交"的仓库，可从本地大仓库截断历史（示例：
  `git clone --no-local file://<repo> dst && cd dst && b=$(git rev-list
  --first-parent HEAD | sed -n 1000p) && git replace --graft $b &&
  git filter-branch -f -- --all && git replace -d $b && rm -rf refs/original`），
  浅克隆（--depth）go-git 无法读取，不要用。

## CI/CD

- **ci.yml**：fmt → clippy(-D warnings) → test(Linux/macOS) → coverage(≥80%) → 三平台 release 构建。
  修改任何代码都必须让这些 job 全绿。
- **bump.yml**：每次 push 到 master/main 自动 bump 补丁版本：改 `Cargo.toml` version +1、
  提交（`[skip ci]` 防自触发）、打 `vX.Y.Z` 标签推送，然后调用 release.yml 发布。
  同分支多次 push 按 concurrency 串行排队，任务开始时 checkout 分支最新 tip 并
  `git reset --hard origin/<branch>`，避免排队任务算出过期版本号。
- **release.yml**：推送 `v*` 标签触发（也供 bump.yml 复用）；先校验标签与 Cargo.toml 版本一致，
  再构建 Linux/macOS/Windows 二进制并创建 Release。发版流程见 README「发布新版本」。
