# Tunely

本地 SSH 端口转发管理器。使用 Tauri 2 + Vue 3 + JavaScript + Vite 构建，前端依赖由 Bun 管理，SSH 连接由 Rust 的 russh 库处理。

## 功能

- 管理多台 SSH 服务器，支持密码、私钥文件和加密私钥口令。
- 保存前测试连接，验证网络、服务器指纹与 SSH 身份认证。
- 每个连接可配置多条 TCP 本地转发规则，分别指定本地端口、监听地址、目标主机和远程端口。
- 本地监听可选择 `localhost`（IPv4 `127.0.0.1`）或 `0.0.0.0`（所有 IPv4 网卡）。
- 首页单独启停规则，一键启停同一连接下的规则，或停止所有转发。
- 列表“操作”列可直接用系统默认浏览器打开 `http://127.0.0.1:本地端口`，不检测服务类型或转发运行状态。
- 实时显示转发状态、活动数据连接数和端口占用、认证失败、网络断开等错误。
- 支持搜索连接及端口、筛选已开启或未开启的规则。

例如，将本地 `8000` 转发到 SSH 服务器的 `8080`：

1. 新建 SSH 连接，填写服务器、SSH 端口、用户名及认证信息。
2. 添加规则：本地监听 `localhost`、本地端口 `8000`、目标主机 `127.0.0.1`、远程端口 `8080`。
3. 点击“测试连接”，首次连接时核对并确认服务器 SHA256 指纹，然后保存。
4. 在首页开启规则，访问 `http://localhost:8000`。

目标主机从 SSH 服务器所在网络访问；`127.0.0.1` 表示 SSH 服务器自身。连接测试只验证 SSH 登录，目标服务的可达性在有数据连接时验证。监听 `0.0.0.0` 时，其他设备也能通过本机 IP 访问该端口。

## 本地存储

连接、端口规则和受信任服务器指纹保存到 Tauri 应用数据目录中的 `connections.json`。macOS 默认路径：

```text
~/Library/Application Support/com.tunely.desktop/connections.json
```

配置使用临时文件原子替换写入；Unix 系统目录权限为 `0700`，文件权限为 `0600`。密码和私钥口令保存于系统凭据库（macOS Keychain、Windows Credential Manager、Linux Secret Service），不会写入 JSON，也不会返回前端。私钥仅保存路径，文件保留在原位置；支持 `~/` 路径。

首次连接需要确认服务器指纹，后续连接会校验已保存的指纹；指纹变化会阻止连接并要求重新核实。编辑转发配置前需停止该连接的转发。删除连接会先停止转发，再删除配置及凭据。再次启动恢复配置，规则默认处于停止状态。

应用启动后显示菜单栏图标。关闭主窗口只隐藏窗口，macOS 同时隐藏 Dock 图标，所有通道继续运行。左键点击菜单栏图标或右键选择“显示主窗口”可恢复窗口和 Dock 图标。右键选择“退出 Tunely”（或使用系统退出操作）时，先停止全部通道、关闭数据连接并释放监听端口，再退出进程；退出过程中禁止开启新通道。

## 环境准备

- Bun 1.3.13 或兼容版本。
- Rust 1.89+ 稳定版工具链（包含 `cargo` 和 `rustc`）。
- 当前操作系统所需的 [Tauri 系统依赖](https://v2.tauri.app/start/prerequisites/)。macOS 需要 Xcode Command Line Tools，可通过 `xcode-select --install` 安装。
- Linux 需要可用的 Secret Service（例如 GNOME Keyring），用于持久保存凭据。

## 开发与构建

```bash
./dev.sh       # 启动 Vite 和 Tauri 桌面应用，支持热更新
./build.sh     # 构建前端、编译 Rust、生成当前系统的应用与安装包
```

脚本会切换到项目根目录、检查工具、按 `bun.lock` 安装前端依赖。两个脚本均可从任意工作目录调用，参数透传给 Tauri CLI：

```bash
./build.sh --bundles app
./dev.sh --help
```

默认构建产物位于 `src-tauri/target/release/bundle/`，macOS 下包含 `.app` 和 `.dmg`。`bun run build` 仅将前端构建到 `dist/`；`bun run desktop:build` 构建完整桌面应用。

只预览前端：

```bash
bun install --frozen-lockfile
bun run dev
```

访问 `http://localhost:1420`。浏览器预览将非敏感配置保存在独立的 localStorage 中，不保存密码或口令，不能执行真实 SSH 测试和转发。桌面应用使用本地文件与系统凭据库，两者配置互相独立。

## 验证

```bash
bun run build
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
```

Rust 测试会启动临时本机 SSH 服务，验证密码和加密私钥认证、主机指纹、并发数据传输、两种监听地址、启停隔离、断线处理和端口释放，不访问真实远程服务器。系统凭据库测试需在具备本地凭据库的环境中单独运行，测试后删除临时条目：

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib system_credentials_round_trip -- --ignored
```

前端页面验收使用 ego-browser。

## 项目结构

```text
src/App.vue                    首页、转发启停、主机指纹确认
src/components/                连接编辑表单、弹窗、图标
src/services/api.js             Tauri 调用及浏览器预览适配
src/style.css                  页面样式
src-tauri/src/commands.rs       Tauri 命令与操作协调
src-tauri/src/model.rs          配置模型与校验
src-tauri/src/store.rs          本地原子存储、系统凭据库
src-tauri/src/ssh.rs            SSH 认证与主机指纹校验
src-tauri/src/tunnel.rs         TCP 转发、运行状态、任务清理
src-tauri/src/tests.rs          本地 SSH 集成测试
src-tauri/tauri.conf.json       应用、开发服务、打包配置
```

前端依赖使用 `bun.lock`，Rust 依赖使用 `src-tauri/Cargo.lock` 固定解析结果。应用标识为 `com.tunely.desktop`。
