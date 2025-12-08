# Camofy

路由器版 Mihomo Web 管理面板。后端使用 Rust + Axum，前端使用 React + Vite，并在构建时打包进单个二进制中，方便在华硕路由器等环境运行。

详细设计文档参见：`DESIGN.md`。

---

## 安装与升级

Camofy 提供一键安装脚本，推荐在路由器或 Linux 服务器上通过 curl 直接安装。

### 一键安装（推荐）

> 仓库：`camofy/camofy`  
> 该脚本会自动检测架构并从 GitHub Releases 下载对应二进制，仅支持 Linux 平台。

```sh
curl -fsSL https://mirror.camofy.app/camofy/camofy/raw/refs/heads/main/install.sh | sh
```

脚本行为：

- 自动检测运行环境：
  - 若存在可写 `/jffs`：视为路由器环境，使用 `/jffs/camofy` 作为数据根目录。
  - 否则视为普通 Linux 环境，使用 `$HOME/.local/share/camofy` 作为数据根目录。
- 自动识别架构并选择下载地址：
  - 目前支持：
    - `x86_64-unknown-linux-musl` → `camofy-linux-amd64`
    - `armv7-unknown-linux-musleabihf` → `camofy-linux-armv7`
  - 从 GitHub Releases 最新版本下载：
    - `https://mirror.camofy.app/camofy/camofy/releases/latest/download/camofy-<arch_tag>`
- 在数据根目录下创建必要的子目录：
  - `config/`、`core/`、`log/`、`tmp/`
- 将二进制保存到：
  - `<DATA_ROOT>/camofy` 并赋予执行权限。

#### Asus /jffs 路由器环境

在检测到 `/jffs` 时，安装脚本会额外配置开机自启动：

- 自动创建或更新 `/jffs/scripts/services-start`，追加如下片段（带有 `camofy auto-start` 标记，避免重复追加）：

  ```sh
  # camofy auto-start
  CAMOFY_ROOT="/jffs/camofy"
  CAMOFY_BIN="$CAMOFY_ROOT/camofy"

  if [ -x "$CAMOFY_BIN" ]; then
    mkdir -p "$CAMOFY_ROOT/log"
    CAMOFY_HOST=0.0.0.0 \
    CAMOFY_PORT=3000 \
    "$CAMOFY_BIN" >>"$CAMOFY_ROOT/log/boot.log" 2>&1 &
  fi
  ```

- 设置脚本可执行：`chmod +x /jffs/scripts/services-start`
- 路由器重启后，Camofy 会自动启动，默认监听 `0.0.0.0:3000`。

#### 普通 Linux 环境

在普通 Linux 环境下（不检测到 `/jffs`）：

- 安装脚本只负责：
  - 下载并安装二进制到：`$HOME/.local/share/camofy/camofy`
  - 初始化数据目录：`$HOME/.local/share/camofy/{config,core,log,tmp}`
- 不会自动配置 systemd / supervisord，请根据自身环境配置守护进程，例如：

```sh
CAMOFY_ROOT="$HOME/.local/share/camofy"
CAMOFY_BIN="$CAMOFY_ROOT/camofy"

chmod +x "$CAMOFY_BIN"

CAMOFY_HOST=127.0.0.1 \
CAMOFY_PORT=3000 \
"$CAMOFY_BIN" &
```

你可以为其编写 systemd service 单元，将上述命令写入 `ExecStart` 以实现开机启动。

### 高级参数

安装脚本支持通过参数定制安装行为，可在使用 curl 时通过 `-s --` 传参：

- `--data-root PATH`  
  指定数据根目录（默认：路由器为 `/jffs/camofy`，普通 Linux 为 `$HOME/.local/share/camofy`）。
- `--bin-url URL`  
  指定 camofy 二进制下载地址（默认从 `camofy/camofy` Releases latest 自动推导）。
- `--help`  
  查看脚本帮助。

示例：

```sh
# 自定义数据根目录
curl -fsSL https://mirror.camofy.app/camofy/camofy/raw/refs/heads/main/install.sh | \
  sh -s -- --data-root /opt/camofy

# 使用自定义二进制下载地址
curl -fsSL https://mirror.camofy.app/camofy/camofy/raw/refs/heads/main/install.sh | \
  sh -s -- --bin-url https://example.com/camofy-linux-amd64
```
