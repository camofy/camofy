#!/usr/bin/env sh

set -eu

REPO_OWNER="camofy"
REPO_NAME="camofy"

DEFAULT_ROUTER_ROOT="/jffs/camofy"
DEFAULT_LINUX_ROOT="${HOME:-}/.local/share/camofy"

print_usage() {
  cat <<EOF
Camofy installer

用法：
  curl -fsSL https://mirror.camofy.app/${REPO_OWNER}/${REPO_NAME}/raw/refs/heads/main/install.sh | sh

可选参数（通过 -s -- 传给脚本，例如: ... | sh -s -- --data-root /custom/path）：
  --data-root PATH   指定数据根目录（默认：路由器为 /jffs/camofy，其他为 \$HOME/.local/share/camofy）
  --bin-url URL      指定 camofy 二进制下载地址（默认：根据架构使用 GitHub Releases latest）
  --help             显示本帮助

注意：
  - 该脚本假定你使用的是类 Linux 系统。
  - 在检测到 /jffs 分区时，会自动将数据根目录设置为 /jffs/camofy，并配置
    /jffs/scripts/services-start 以实现开机自启动（如文件已存在则追加片段）。
  - GitHub Releases 中 camofy 二进制命名需符合 camofy-<arch> 格式，
    例如 camofy-linux-amd64 / camofy-linux-arm64 等。
EOF
}

DATA_ROOT=""
BIN_URL=""

while [ "$#" -gt 0 ]; do
  case "$1" in
    --data-root)
      if [ "$#" -lt 2 ]; then
        echo "缺少 --data-root 参数值" >&2
        exit 1
      fi
      DATA_ROOT="$2"
      shift 2
      ;;
    --bin-url)
      if [ "$#" -lt 2 ]; then
        echo "缺少 --bin-url 参数值" >&2
        exit 1
      fi
      BIN_URL="$2"
      shift 2
      ;;
    --help|-h)
      print_usage
      exit 0
      ;;
    *)
      echo "未知参数: $1" >&2
      print_usage
      exit 1
      ;;
  esac
done

detect_arch_tag() {
  uname_m=$(uname -m 2>/dev/null || echo "unknown")

  case "$uname_m" in
    x86_64|amd64)
      echo "linux-amd64"
      ;;
    aarch64|arm64)
      echo "linux-arm64"
      ;;
    armv7*|armv7l)
      echo "linux-armv7"
      ;;
    armv8*|armv8l)
      echo "linux-armv8"
      ;;
    mipsel|mipsle)
      echo "linux-mipsle"
      ;;
    mips*)
      echo "linux-mips"
      ;;
    *)
      echo ""
      ;;
  esac
}

detect_data_root() {
  if [ -n "$DATA_ROOT" ]; then
    echo "$DATA_ROOT"
    return
  fi

  if [ -d "/jffs" ] && [ -w "/jffs" ]; then
    echo "$DEFAULT_ROUTER_ROOT"
  else
    echo "$DEFAULT_LINUX_ROOT"
  fi
}

detect_bin_url() {
  if [ -n "$BIN_URL" ]; then
    echo "$BIN_URL"
    return
  fi

  arch_tag=$(detect_arch_tag)
  if [ -z "$arch_tag" ]; then
    echo "无法自动识别当前架构，请使用 --bin-url 指定 camofy 二进制下载地址。" >&2
    exit 1
  fi

  asset_name="camofy-${arch_tag}"
  echo "https://mirror.camofy.app/${REPO_OWNER}/${REPO_NAME}/releases/latest/download/${asset_name}"
}

download_binary() {
  url="$1"
  dest="$2"

  echo "从 ${url} 下载 camofy 二进制..."

  tmp="${TMPDIR:-/tmp}/camofy-download-$$"

  if command -v curl >/dev/null 2>&1; then
    if ! curl -fsSL "$url" -o "$tmp"; then
      echo "下载 camofy 失败（curl）" >&2
      rm -f "$tmp" 2>/dev/null || true
      exit 1
    fi
  elif command -v wget >/dev/null 2>&1; then
    if ! wget -qO "$tmp" "$url"; then
      echo "下载 camofy 失败（wget）" >&2
      rm -f "$tmp" 2>/dev/null || true
      exit 1
    fi
  else
    echo "未找到 curl 或 wget，请先安装其一。" >&2
    exit 1
  fi

  mv "$tmp" "$dest"
  chmod +x "$dest"
}

ensure_dirs() {
  root="$1"
  mkdir -p "$root" || {
    echo "创建目录失败: $root" >&2
    exit 1
  }

  for sub in config core log tmp; do
    mkdir -p "$root/$sub" || {
      echo "创建子目录失败: $root/$sub" >&2
      exit 1
    }
  done
}

configure_services_start() {
  install_root="$1"

  if [ ! -d "/jffs" ]; then
    return
  fi

  scripts_dir="/jffs/scripts"
  services_file="${scripts_dir}/services-start"

  mkdir -p "$scripts_dir" || {
    echo "创建目录失败: $scripts_dir" >&2
    return
  }

  if [ ! -f "$services_file" ]; then
    echo "#!/bin/sh" >"$services_file"
    echo "" >>"$services_file"
  fi

  if grep -q "camofy auto-start" "$services_file" 2>/dev/null; then
    echo "检测到 /jffs/scripts/services-start 已包含 camofy 配置，跳过追加。"
  else
    cat <<EOF >>"$services_file"

# camofy auto-start
CAMOFY_ROOT="${install_root}"
CAMOFY_BIN="\$CAMOFY_ROOT/camofy"

if [ -x "\$CAMOFY_BIN" ]; then
  mkdir -p "\$CAMOFY_ROOT/log"
  CAMOFY_HOST=0.0.0.0 \\
  CAMOFY_PORT=3000 \\
  "\$CAMOFY_BIN" >>"\$CAMOFY_ROOT/log/boot.log" 2>&1 &
fi
EOF
    echo "已更新 /jffs/scripts/services-start 以在开机时自动启动 camofy。"
  fi

  chmod +x "$services_file" || {
    echo "设置执行权限失败: $services_file" >&2
  }
}

main() {
  data_root=$(detect_data_root)
  bin_url=$(detect_bin_url)

  echo "数据根目录：${data_root}"
  echo "camofy 下载地址：${bin_url}"

  ensure_dirs "$data_root"

  bin_path="${data_root}/camofy"
  download_binary "$bin_url" "$bin_path"

  if echo "$data_root" | grep -q "^/jffs/"; then
    configure_services_start "$data_root"
    echo "安装完成。路由器重启后 camofy 将自动启动。"
  else
    cat <<EOF
安装完成。

camofy 二进制路径：${bin_path}
数据根目录：${data_root}

请在你的 Linux 系统中通过 systemd、supervisord 或其它方式配置 camofy 为守护进程，
启动时可设置：

  CAMOFY_HOST=127.0.0.1 \\
  CAMOFY_PORT=3000 \\
  ${bin_path} &

EOF
  fi
}

main "$@"

