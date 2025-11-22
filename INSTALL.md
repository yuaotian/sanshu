# 三术 MCP 工具安装指南

## 快速安装

### 方式一：使用安装脚本（推荐）

```bash
# 克隆仓库
git clone https://github.com/yuaotian/sansu.git
cd sansu

# 运行安装脚本
chmod +x install.sh
./install.sh
```

### 方式二：下载预编译版本

从 [Releases](https://github.com/yuaotian/sansu/releases) 页面下载对应平台的预编译版本：

- **Linux**: `sansu-linux-x86_64.tar.gz`
- **macOS**: `sansu-macos-universal.tar.gz`
- **Windows**: `sansu-windows-x86_64.zip`

#### 安装步骤：

1. 下载对应平台的压缩包
2. 解压到任意目录
3. 将解压目录添加到 PATH 环境变量

```bash
# Linux/macOS 示例
tar -xzf sansu-linux-x86_64.tar.gz
sudo cp 等一下 三术 /usr/local/bin/
```

```powershell
# Windows 示例
# 解压 zip 文件到 C:\sansu
# 将 C:\sansu 添加到系统 PATH
```

## 验证安装

```bash
# 检查工具是否正确安装
三术 --help
等一下 --help
```

## MCP 客户端配置

将以下配置添加到您的 MCP 客户端配置文件中：

```json
{
  "mcpServers": {
    "三术": {
      "command": "三术"
    }
  }
}
```

## 使用方法

### MCP 服务器模式
```bash
三术  # 启动 MCP 服务器
```

### 弹窗界面模式
```bash
等一下                          # 启动设置界面
等一下 --mcp-request file       # MCP 弹窗模式
```

## 工具说明

- **三术**: MCP 服务器，提供记忆管理和智能交互功能
- **等一下**: 弹窗界面，用于用户交互和设置

## 系统要求

- **Linux**: x86_64 架构
- **macOS**: 10.15+ (支持 Intel 和 Apple Silicon)
- **Windows**: Windows 10+ x86_64

## 故障排除

### 权限问题
```bash
# Linux/macOS
chmod +x 等一下 三术
```

### PATH 问题
确保安装目录已添加到 PATH 环境变量中。

### 依赖问题
两个 CLI 工具必须在同一目录下才能正常工作。

## 开发者安装

如果您想从源码构建：

```bash
# 安装依赖
cargo --version  # 需要 Rust 1.70+
pnpm --version   # 需要 pnpm

# 构建
pnpm install
pnpm build
cargo build --release

# 安装
cp target/release/等一下 target/release/三术 ~/.local/bin/
```

## 更新

### 使用预编译版本
重新下载最新版本并替换旧文件。

### 使用源码
```bash
git pull
./install.sh
```
