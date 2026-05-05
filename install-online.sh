#!/usr/bin/env bash

# 三术在线安装脚本（macOS / Linux）
# 中文说明：该脚本不依赖本地 Rust/Node 构建环境，会从 GitHub Release 下载最新 CLI 包。

set -euo pipefail

INSTALL_DIR=""
NO_PATH=0
DRY_RUN=0
TIMEOUT_SECONDS=8

while [[ $# -gt 0 ]]; do
  case "$1" in
    --install-dir)
      INSTALL_DIR="${2:-}"
      shift 2
      ;;
    --no-path)
      NO_PATH=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --timeout)
      TIMEOUT_SECONDS="${2:-8}"
      shift 2
      ;;
    *)
      echo "未知参数: $1"
      exit 1
      ;;
  esac
done

RELEASE_API_URL="https://api.github.com/repos/yuaotian/sanshu/releases/latest"
USER_AGENT="sanshu-online-installer/1.0"
PROXY_PREFIXES=(
  "https://wget.la/"
  "https://rapidgit.jjda.de5.net/"
  "https://fastgit.cc/"
  "https://gitproxy.mrhjx.cn/"
  "https://github.boki.moe/"
  "https://github.ednovas.xyz/"
)
LOCAL_PROXY_PORTS=(7890 7891 7892 10808 8080)

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "缺少依赖命令: $1"
    exit 1
  fi
}

join_proxy_url() {
  local prefix="${1%/}"
  local url="$2"
  printf '%s/%s' "$prefix" "$url"
}

detect_country() {
  local result
  if result="$(curl -fsSL --connect-timeout 3 --max-time 3 -H "User-Agent: $USER_AGENT" "https://ipinfo.io/json" 2>/dev/null)"; then
    python3 - "$result" <<'PY'
import json, sys
try:
    print(json.loads(sys.argv[1]).get("country") or "UNKNOWN")
except Exception:
    print("UNKNOWN")
PY
    return
  fi
  echo "UNKNOWN"
}

tcp_port_open() {
  local port="$1"
  python3 - "$port" <<'PY'
import socket, sys
port = int(sys.argv[1])
sock = socket.socket()
sock.settimeout(0.3)
try:
    sock.connect(("127.0.0.1", port))
    print("1")
except Exception:
    print("0")
finally:
    sock.close()
PY
}

curl_json() {
  local url="$1"
  local proxy="${2:-}"
  local insecure="${3:-0}"
  local timeout="${4:-$TIMEOUT_SECONDS}"
  local args=(-fsSL --connect-timeout "$timeout" --max-time "$timeout" -H "User-Agent: $USER_AGENT" -H "Accept: application/vnd.github+json")
  if [[ "$insecure" == "1" ]]; then
    args+=(-k)
  fi
  if [[ -n "$proxy" ]]; then
    args+=(--proxy "$proxy")
  fi
  curl "${args[@]}" "$url"
}

download_file() {
  local url="$1"
  local out_file="$2"
  local proxy="${3:-}"
  local insecure="${4:-0}"
  local args=(-fL --connect-timeout "$TIMEOUT_SECONDS" --max-time 60 -H "User-Agent: $USER_AGENT")
  if [[ "$insecure" == "1" ]]; then
    args+=(-k)
  fi
  if [[ -n "$proxy" ]]; then
    args+=(--proxy "$proxy")
  fi
  curl "${args[@]}" -o "$out_file" "$url"
}

strategy_candidates() {
  local url="$1"
  local country="$2"
  local direct_timeout="$TIMEOUT_SECONDS"
  if [[ "$country" == "CN" || "$country" == "UNKNOWN" ]]; then
    direct_timeout=3
  fi

  printf 'github-direct-%s\t%s\t\t0\t%s\n' "$country" "$url" "$direct_timeout"
  for prefix in "${PROXY_PREFIXES[@]}"; do
    printf 'github-proxy:%s\t%s\t\t1\t4\n' "${prefix%/}" "$(join_proxy_url "$prefix" "$url")"
  done
  for port in "${LOCAL_PROXY_PORTS[@]}"; do
    if [[ "$(tcp_port_open "$port")" == "1" ]]; then
      printf 'local-proxy:127.0.0.1:%s\t%s\thttp://127.0.0.1:%s\t0\t%s\n' "$port" "$url" "$port" "$TIMEOUT_SECONDS"
    fi
  done
}

json_with_strategy() {
  local url="$1"
  local country="$2"
  local line label candidate_url proxy insecure timeout
  while IFS=$'\t' read -r label candidate_url proxy insecure timeout; do
    echo "尝试获取 JSON: $label" >&2
    if result="$(curl_json "$candidate_url" "$proxy" "$insecure" "$timeout" 2>/dev/null)"; then
      printf '%s' "$result"
      return 0
    fi
  done < <(strategy_candidates "$url" "$country")
  return 1
}

download_with_strategy() {
  local url="$1"
  local out_file="$2"
  local country="$3"
  local line label candidate_url proxy insecure timeout
  while IFS=$'\t' read -r label candidate_url proxy insecure timeout; do
    echo "尝试下载: $label" >&2
    if download_file "$candidate_url" "$out_file" "$proxy" "$insecure" 2>/dev/null; then
      echo "$label"
      return 0
    fi
    rm -f "$out_file"
  done < <(strategy_candidates "$url" "$country")
  return 1
}

detect_platform_asset_keyword() {
  local os arch
  os="$(uname -s)"
  arch="$(uname -m)"
  case "$os:$arch" in
    Darwin:arm64) echo "macos-aarch64" ;;
    Darwin:x86_64) echo "macos-x86_64" ;;
    Linux:x86_64|Linux:amd64) echo "linux-x86_64" ;;
    *) echo "不支持的平台: $os $arch" >&2; exit 1 ;;
  esac
}

resolve_install_dir() {
  if [[ -n "$INSTALL_DIR" ]]; then
    printf '%s\n' "$INSTALL_DIR"
  else
    printf '%s\n' "$HOME/mcp_server/sanshu"
  fi
}

add_shell_path() {
  local dir="$1"
  local shell_file="$HOME/.profile"
  if [[ -n "${ZSH_VERSION:-}" || "${SHELL:-}" == *"zsh"* ]]; then
    shell_file="$HOME/.zshrc"
  elif [[ -n "${BASH_VERSION:-}" || "${SHELL:-}" == *"bash"* ]]; then
    shell_file="$HOME/.bashrc"
  fi

  if grep -F "$dir" "$shell_file" >/dev/null 2>&1; then
    return 0
  fi
  printf '\n# 三术 MCP 工具\nexport PATH="%s:$PATH"\n' "$dir" >> "$shell_file"
}

open_install_dir() {
  local dir="$1"
  if command -v open >/dev/null 2>&1; then
    open "$dir" >/dev/null 2>&1 || true
  elif command -v xdg-open >/dev/null 2>&1; then
    xdg-open "$dir" >/dev/null 2>&1 || true
  fi
}

require_command curl
require_command python3
require_command tar

echo "三术在线安装开始"
country="$(detect_country)"
echo "检测到区域: $country"

release_json="$(json_with_strategy "$RELEASE_API_URL" "$country")"
platform_keyword="$(detect_platform_asset_keyword)"
asset_info="$(python3 - "$release_json" "$platform_keyword" <<'PY'
import json, sys
release = json.loads(sys.argv[1])
keyword = sys.argv[2]
for asset in release.get("assets", []):
    name = asset.get("name", "")
    if keyword in name and name.endswith(".tar.gz"):
        print(name)
        print(asset.get("browser_download_url", ""))
        print(release.get("tag_name", ""))
        break
else:
    raise SystemExit(f"未找到平台资产: {keyword}")
PY
)"
asset_name="$(printf '%s\n' "$asset_info" | sed -n '1p')"
asset_url="$(printf '%s\n' "$asset_info" | sed -n '2p')"
tag_name="$(printf '%s\n' "$asset_info" | sed -n '3p')"
install_dir="$(resolve_install_dir)"
temp_root="${TMPDIR:-/tmp}/sanshu-online-install"
archive_path="$temp_root/$asset_name"
extract_dir="$temp_root/extract"

echo "最新版本: $tag_name"
echo "Release 资产: $asset_name"
echo "安装目录: $install_dir"
echo "DryRun: $DRY_RUN"

if [[ "$DRY_RUN" == "1" ]]; then
  echo "DryRun 完成：未下载、未解压、未修改 PATH。"
  exit 0
fi

rm -rf "$extract_dir"
mkdir -p "$temp_root" "$extract_dir" "$install_dir"

route="$(download_with_strategy "$asset_url" "$archive_path" "$country")"
echo "下载完成: $archive_path via $route"

tar -xzf "$archive_path" -C "$extract_dir"
for name in "等一下" "三术" "sanshu" "README.md" "sanshu_prompt_word.md" "sanshu_prompt_word_cli.md"; do
  source_path="$(find "$extract_dir" -type f -name "$name" | head -n 1)"
  if [[ -z "$source_path" ]]; then
    echo "Release 包中缺少文件: $name" >&2
    exit 1
  fi
  cp "$source_path" "$install_dir/$name"
done
chmod +x "$install_dir/等一下" "$install_dir/三术" "$install_dir/sanshu"

path_added=0
if [[ "$NO_PATH" != "1" ]]; then
  read -r -p "是否自动添加安装目录到 PATH？默认 Y，输入 n 跳过: " answer
  answer_lc="$(printf '%s' "$answer" | tr '[:upper:]' '[:lower:]')"
  if [[ -z "$answer" || "$answer_lc" != "n" ]]; then
    add_shell_path "$install_dir"
    path_added=1
    echo "已写入 shell 配置，请重启终端或运行 source 对应配置文件。"
  fi
fi

if [[ "$path_added" == "1" ]]; then
  command_value="sanshu"
else
  command_value="$install_dir/sanshu"
fi

cat <<EOF

MCP 配置（新建配置可直接使用）：
{
  "mcpServers": {
    "sanshu": {
      "command": "$command_value"
    }
  }
}

如果已有 MCP 配置，请只把下面这一段插入既有 mcpServers 对象中，不要新建第二个 mcpServers：
"sanshu": {
  "command": "$command_value"
}
EOF

open_install_dir "$install_dir"
echo "三术在线安装完成: $install_dir"
