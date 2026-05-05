#!/bin/bash

# 三术 MCP 工具 - 最简化安装脚本
# 只需构建 CLI 工具即可运行 MCP

set -e

echo "🚀 安装 三术 MCP 工具..."

# 检查必要工具
for cmd in cargo pnpm; do
    if ! command -v "$cmd" &> /dev/null; then
        echo "❌ 请先安装 $cmd"
        exit 1
    fi
done

# 构建
echo "📦 构建前端资源..."
pnpm build

echo "🔨 构建 CLI 工具..."
cargo build --release

# 检查构建结果
if [[ ! -f "target/release/等一下" ]] || [[ ! -f "target/release/三术" ]]; then
    echo "❌ 构建失败"
    exit 1
fi

# 安装到用户目录
BIN_DIR="$HOME/.local/bin"
mkdir -p "$BIN_DIR"

cp "target/release/等一下" "$BIN_DIR/"
cp "target/release/三术" "$BIN_DIR/"
# 中文说明：sanshu 是三术的 ASCII 兼容副本，供不稳定支持中文命令的 MCP 客户端使用。
cp "target/release/三术" "$BIN_DIR/sanshu"
chmod +x "$BIN_DIR/等一下" "$BIN_DIR/三术" "$BIN_DIR/sanshu"

echo "✅ 安装完成！CLI 工具已安装到 $BIN_DIR"

# 检查PATH
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    echo ""
    echo "💡 请将以下内容添加到 ~/.bashrc 或 ~/.zshrc:"
    echo "export PATH=\"\$PATH:$BIN_DIR\""
    echo "然后运行: source ~/.bashrc"
fi

echo ""
echo "📋 使用方法："
echo "  sanshu      - 启动 MCP 服务器（推荐，ASCII 兼容入口）"
echo "  三术        - 启动 MCP 服务器（中文兼容入口）"
echo "  等一下      - 启动弹窗界面"
echo ""
echo "📝 MCP 客户端配置："
echo '{"mcpServers": {"sanshu": {"command": "sanshu"}}}'
