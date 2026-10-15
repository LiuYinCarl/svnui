# 本机开发 / 演示环境（维护者机器）

本文记录**本机**（`/home/kenshin`）为 svnui 的演示录制与压测一次性搭好的工具、目录和镜像，
以及换机器时的重建步骤。它**不属于仓库功能**：只影响 `scripts/record_demos.sh` /
`scripts/stress_test.sh` 这类本机脚本能否直接跑。

流程说明看 `agent.md`（「演示素材录制」「压力测试」两节）；本文是环境清单。

## 一次性工具（保留在本机，勿删）

| 工具 | 位置 | 说明 |
| --- | --- | --- |
| asciinema 3.2.1 | `~/.local/bin/asciinema` | **静态 musl** 二进制（GitHub release `asciinema-x86_64-unknown-linux-musl`）。不能装 PyPI 上的 2.4：2.x 不支持 `record_demos.sh` 用的 `--headless` / `--window-size` |
| agg 1.9.0 | `~/.local/bin/agg` | **静态 musl** 二进制（release `agg-x86_64-unknown-linux-musl`）。同页的 glibc 版要求 `GLIBC_2.38`，本机 Ubuntu 22.04 只有 2.35，会报 `version 'GLIBC_2.38' not found` |
| git2svn | `~/github/git2svn/git2svn` | 独立 clone（`LiuYinCarl/git2svn`）后 `go build -o git2svn .`；演示源仓库和压测的转换都靠它。注意 `scripts/stress_test.sh` 默认找 `~/dev/github/git2svn`，本机要用 `GIT2SVN_DIR=$HOME/github/git2svn` 覆盖 |
| spdlog 演示源仓库 | `~/dev/spdlog-svn-repo` | spdlog 的 git `v1.x` 分支（2978 个 first-parent 修订）经 `git2svn dump` 转成的 SVN 仓库，`scripts/setup_demo_env.sh` 的默认 `SVNUI_DEMO_SRC_REPO` |
| spdlog git 克隆 | `target/tmp/spdlog-git` | 转换素材；可随时删掉重克隆（在 gitignore 的 `target/` 里） |
| 演示工作副本 | `/tmp/svnui-demo` | `setup_demo_env.sh` 生成（`repo/` 热拷贝、`wc/`、`wc-base/`、`patches/`、`out/`）；可删，下次录制自动重建 |
| Python Pillow | 系统 `python3`（9.0.1） | `record_demos.sh` 用它把 snap 点转成 PNG；已可用，无需额外装 |

演示录制的系统依赖：`svn` / `svnadmin`、`rsync`、`git`、`go`、等宽字体
（fc-match monospace → DejaVu Sans Mono）。演示文案保持纯英文——等宽主字体没有 CJK
字形，中文注释/消息会走字体回退，不保证渲染效果。

## crates.io 镜像

本机直连 crates.io 会超时。项目根目录有一个**未跟踪**的 `.cargo/config.toml`
（已写进 `.git/info/exclude`，不进仓库）：

```toml
[source.crates-io]
replace-with = "ustc"

[source.ustc]
registry = "sparse+https://mirrors.ustc.edu.cn/crates.io-index/"
```

- 跑 `cargo build / test / clippy / install` 保持这个文件即可；
- **不要提交它**（会给其他贡献者强加国内镜像）；
- 注意 `cargo --config '...' clippy` 不生效：`cargo-clippy` 是外部子命令，不转发
  `--config`（`cargo fetch` 这类内建命令才吃）。要全局生效就放到 `~/.cargo/config.toml`；
- 曾观察到 `cargo fetch` 走替换源后，旧的 `~/.cargo/registry/cache/index.crates.io-*`
  目录被清理；依赖从镜像重下即可，不影响。

## 重建步骤（换机器 / 清理后）

```bash
# 1) asciinema + agg：GitHub release 的 musl 静态包（不要用 pip / glibc 版）
gh release download -R asciinema/asciinema \
    --pattern 'asciinema-x86_64-unknown-linux-musl' --output /tmp/asciinema
install -m 755 /tmp/asciinema ~/.local/bin/asciinema
gh release download -R asciinema/agg \
    --pattern 'agg-x86_64-unknown-linux-musl' --output /tmp/agg
install -m 755 /tmp/agg ~/.local/bin/agg

# 2) git2svn
gh repo clone LiuYinCarl/git2svn ~/github/git2svn
(cd ~/github/git2svn && go build -o git2svn .)

# 3) spdlog -> SVN 演示源仓库（HTTPS 到 github.com 在本机偶发 TLS 重置，
#    失败就重试或改用 SSH 克隆）
git clone git@github.com:gabime/spdlog.git target/tmp/spdlog-git
~/github/git2svn/git2svn dump target/tmp/spdlog-git ~/dev/spdlog-svn-repo v1.x

# 4) 构建 + 搭建演示环境 + 录制
cargo build --release
scripts/setup_demo_env.sh          # 重置仓库热拷贝和 wc（重录前建议先跑）
scripts/record_demos.sh            # 全部 8 个演示
SVNUI_DEMO_ONLY=svnignore scripts/record_demos.sh   # 只重录单个
```

录制相关环境变量：`SVNUI_DEMO_DIR`、`SVNUI_DEMO_SRC_REPO`、`SVNUI_DEMO_SIZE`、
`AGG_THEME`、`AGG_SPEED`、`SVNUI_DEMO_ONLY`（见 `scripts/record_demos.sh` 头部）。
运行前确保 `~/.local/bin` 和 `~/.cargo/bin` 在 `PATH` 里。

本机压测（转换 + 检出 + tree parity + 无头驱动，默认 200 轮）：

```bash
STRESS_GIT_REPO=target/tmp/spdlog-git STRESS_GIT_BRANCH=v1.x \
GIT2SVN_DIR="$HOME/github/git2svn" scripts/stress_test.sh
# 快速验证： SVNUI_STRESS_ROUNDS=30 ... （压测脚本会自己重建 wc）
```
