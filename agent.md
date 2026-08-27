# agent.md

面向 AI 编码助手 / 贡献者的仓库指南。先读这个文件，再动代码。

## 项目概况

`svnui` 是一个受 [gitui](https://github.com/gitui-org/gitui) 启发的 SVN 终端 UI 客户端，
用 Rust + ratatui 编写，约 5300 行实现 + 约 6800 行测试（行覆盖率 ~95%）。

**关键约束（务必遵守）：**

- 这是 **SVN** 项目，不是 git。所有版本控制操作通过 `svn` CLI 完成（见 `src/svn/mod.rs`），
  代码里绝不能出现 `git` 命令或 git 语义（索引、HEAD、rebase 等）。
- SVN **没有暂存区**。"暂存"被实现为**提交集**（`StatusTreeComponent::staged: HashSet<String>`）：
  标记哪些路径随下一次 `svn commit` 一起提交。未版本化（`?`）文件暂存时自动执行 `svn add`。
- 禁止使用 `unsafe`。新代码必须通过 `cargo clippy --all-targets -- -D warnings`（CI 门禁）。

## 目录结构

```
src/
├── main.rs           入口：终端初始化（raw mode + alternate screen + bracketed paste）、
│                     事件循环（crossbeam select 多路复用输入/异步/时钟；输入线程丢弃
│                     KeyEventKind::Release 事件，避免 Windows/kitty 终端下字符重复输入）
├── app.rs            App 状态机：标签页、弹窗栈、异步通知分发、绘制入口
├── queue.rs          InternalEvent 队列（组件→App 的通信）+ NeedsUpdate 位标志
├── status.rs         Status 标签页聚合（文件树 + Diff 面板 + 提交输入栏 + 焦点切换）
├── keys.rs           快捷键集中定义（key_match(ev, KeyAction) -> bool）
├── strings.rs        用户可见字符串（集中管理）
├── svn/
│   ├── mod.rs        Svn 客户端：所有命令在后台线程执行，经 channel 回传 AsyncSvnNotification
│   ├── parser.rs     svn status/log/blame/diff 纯文本解析器（有单元测试）
│   └── models.rs     数据结构：StatusEntry / LogEntry / BlameLine / ParsedDiff ...
├── components/
│   ├── mod.rs        DrawableComponent trait、EventState、Context
│   ├── status_tree.rs  文件树（目录折叠、暂存、过滤）——最大的组件
│   ├── diff_view.rs  可滚动 Diff 视图（状态页与全屏弹窗共用）
│   ├── commit.rs     提交信息输入栏（基于 tui-textarea-2，unicode-width 光标，
│   │                 Tab 弹出最近 10 条提交信息快速填充，支持 bracketed paste）
│   ├── log.rs        日志标签页（修订列表 + 详情 + `/` 关键字搜索 + space 多选合并 diff）
│   ├── file_log.rs   单文件历史弹窗（svn log -v -- path）
│   ├── file_finder.rs  fzf 式模糊文件搜索弹窗（数据源 svn list -R .@HEAD，
│   │                   匹配用 fuzzy-matcher crate = skim 的 SkimMatcherV2）
│   ├── blame.rs      blame 弹窗
│   └── help.rs       帮助弹窗
├── popups/           确认 / 消息 / 输出查看 / 全屏 Diff 弹窗（enum Popup 分发，无 downcast）
├── ui/               渲染辅助（滚动、行绘制、弹窗矩形）+ 主题
└── test_support.rs   测试支撑：临时 SVN 仓库（TestRepo）+ TestBackend 渲染辅助
.github/workflows/     ci.yml（fmt/clippy/test/coverage/build）+ release.yml（Tag 触发发布）
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

目标：10 万+ 文件的项目不卡死。已落实的措施：

- **`build_tree` 为 O(n)**：一次性 HashMap 建节点 + 索引装配 + 排序。
  曾因逐层线性查找导致 10 万文件单目录下耗时 **8.8s**（卡死），现 release 约 **47ms**。
- **虚拟化渲染**：文件树 / Diff / Blame 每次 draw 只构建可见窗口
  （O(屏幕高度)），10 万条目绘制 ~80µs。
- **`dir_staged_counts` 缓存**：按目录的暂存计数只在暂存集合或条目变化时重算，
  导航/绘制时零重算。
- 日志固定 `-l 50`，提交集为 HashSet 查找 O(1)。

验证方式：
- `cargo bench --bench tree`（criterion，release）：本地精确数据；
- `cargo test perf`（CI 强制）：7 个带时间预算的回归测试，debug 下也运行；
  O(n²) 回归会让 100k 用例从 ~0.2s 变成 30s+，立刻被门禁拦下。

基准（release, Apple Silicon）：
```
status_tree/update_wide_100k     ~47 ms
status_tree/update_deep_100k     ~83 ms
status_tree/draw_wide_100k       ~82 µs
parsers/parse_status_100k        ~155 ms
parsers/parse_diff_50k           ~1.5 ms
```

## 常用命令

```bash
cargo build                 # 开发构建
cargo test                  # 152 个测试（含 7 个性能门禁；部分会创建真实临时 SVN 仓库）
cargo bench --bench tree     # criterion 性能基准（超大型工作副本）
cargo test <name>           # 跑单个测试
cargo llvm-cov              # 覆盖率报告（需 cargo-llvm-cov + llvm-tools-preview）
cargo llvm-cov --fail-under-lines 80   # 覆盖率门禁（与 CI 一致）
cargo clippy --all-targets -- -D warnings   # 零警告门禁
cargo fmt --all --check     # 格式门禁
```

## 编码约定

- 错误处理：内部使用 `Result<_, String>`（svn stderr 文本），UI 层显示在消息弹窗。
- 组件实现 `DrawableComponent`（`draw(&self, ...)` + `event(&mut self, ...)`）。
- 文本输入用 `tui-textarea-2`（与 gitui 同思路）：光标按 unicode 单元格渲染、退格按字符安全删除、自带横向滚动。不要手写字符宽度计算。
  注意 `draw` 是 `&self`——需要可变状态（滚动偏移等）用 `Cell`。
- 组件不应直接持有 `Svn`；通过 `Context.queue` push `InternalEvent`，由 App 执行命令。
- 新增快捷键：在 `keys.rs` 加 `KeyAction` 变体 + `key_match` 分支，并更新 `all_bindings()`（帮助页）。
- 新增用户可见文本：加到 `strings.rs`。
- 弹窗加进 `popups/mod.rs` 的 `enum Popup`，不要在 App 里用 downcast。
- 解析器（`svn/parser.rs`）保持纯函数、无 IO，方便单元测试。
- 依赖策略：简单逻辑手写，不引入新 crate；只有复杂且易错的逻辑（如 fuzzy 匹配打分，
  用 `fuzzy-matcher`）才引入流行的纯 Rust 库——需无 unsafe、无 build script、维护活跃。
  已刻意移除的依赖：`clap`（单个可选位置参数，`main.rs` 里手写 `parse_args` 即可）、
  `bitflags`（`NeedsUpdate` 只有 3 个标志位，`queue.rs` 里手写位运算）。

## SVN 输出格式要点（解析器已处理，改代码前先读）

- **所有 svn 子进程（含测试辅助）都固定英文消息**（`Svn::run_in` / `test_support::svn`：
  `env_remove("LC_ALL")` + `LC_MESSAGES=C`）：svn 用 gettext 本地化输出（如中文 locale 下
  `Path:` 变 `路径:`），解析器依赖英文文本。**不能用 `LC_ALL=C`**——那会把 native 编码
  固定为 ASCII，导致非 ASCII 的 log 输出变成 `{U+XXXX}` 转义、中文 `-m` 提交报 E000022；
  `LC_MESSAGES` 只控制消息目录语言，字符编码仍跟随用户 locale（LC_CTYPE/LANG）。
  新增解析逻辑时不要绕开这两个入口直接起 svn 进程。
- `svn commit` 必须带 `--encoding UTF-8`：`-m` 消息是 UTF-8 字节，不显式声明时 svn 按
  native 编码解释（非 UTF-8 locale 下中文消息会被拒）。

- `svn status`：7 列状态字符后接路径（路径从第 8 个字符开始，实际验证于 svn 1.14.5）。
- `svn log`：**必须显式传范围** `-r HEAD:0 -l N`（`HEAD:1` 在 r0 空仓库上报 E160006），否则可能无输出。
- `svn diff` 对未版本化文件返回 **E155010 错误**而不是空输出——`Svn::diff` 已做回退
  （读文件内容直接显示）。
- `svn blame` 格式：`%6s %10s %s`（修订号右对齐 6、作者右对齐 10、内容从第 18 列开始，
  前导缩进要保留）。
- `svn info` 的 `Revision` 是工作副本根目录的 BASE 修订：SVN 是混合修订工作副本，
  子文件提交后根目录修订在 `svn update` 前不会变。分支名取 `Relative URL`（去掉 `^/`）。
- `svn list -R` 必须带 `.@HEAD` peg：不带时按 wc 根目录的 BASE 修订列举，
  提交后未 update 时可能列出旧内容甚至为空。
- `svn commit -m "docs"` 这类"像路径"的消息会报 E205005，需要 `--force-log` 或用更长文本。
- `svn revert` 一个刚 add 的文件：取消 add，文件留在磁盘上（变成 `?`）。

## 测试策略（覆盖率 ~95% 的构成）

- **解析器/模型**：构造输出样本做单元测试。
- **UI 组件**：`test_support::render` 用 `ratatui::TestBackend` 离屏绘制 + 合成 crossterm 事件。
- **svn 命令层**：`TestRepo` 用 `svnadmin create` 建临时仓库，真实执行
  status/diff/log/blame/add/revert/commit/update/resolve（本机无 svn 时测试自动跳过）。
- **App 状态机**：直接构造 `AsyncSvnNotification` / `InternalEvent` 喂给
  `handle_async` / `handle_queue_events`，覆盖 Ok/Err 全部分支。
- **事件循环**：`run()` 已泛型化，可用 `TestBackend` 驱动到退出。

## CI/CD

- **ci.yml**：fmt → clippy(-D warnings) → test(Linux/macOS) → coverage(≥80%) → 三平台 release 构建。
  修改任何代码都必须让这些 job 全绿；CI 门禁零警告零错误。
- **release.yml**：推送 `v*` 标签触发；先校验标签与 Cargo.toml 版本一致，
  再构建 Linux/macOS/Windows 二进制并用 `GITHUB_TOKEN` 创建 Release（无 Release Bot）。
- 发版流程见 README「发布新版本」。
