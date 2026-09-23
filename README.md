# Camofy

开源、自托管的多租户网络配置平台。云端管理订阅与可组合的 YAML profiles，
向路由器 Agent、Clash Verge Rev 和 Shadowrocket 分发生成的配置。

- 多个 Clash YAML 订阅与独立 Profile，按身份内的顺序和启用状态组合。
- 管理员维护平台代理池，全局选择 HTTP / HTTPS / SOCKS5 订阅出口。
- 首次、手动和定时刷新共用全局代理；失败保留最后有效配置，禁止回退直连。
- 多设备绑定、版本历史、发布回滚、独立订阅令牌与撤销。
- WebSocket 通知 + HTTPS 拉取；Agent 五分钟轮询兜底、离线恢复。
- 云端不运行 Mihomo，不接收客户端日志；测速由受管设备执行。

## 自托管

需要 Docker Compose。复制 `.env.example` 为 `.env`，填写数据库密码与加密密钥：

```sh
# 数据库密码建议使用 hex，避免 URL 特殊字符
openssl rand -hex 24
# 数据加密密钥：保存并备份，不要在重启时重新生成
openssl rand -base64 32
docker compose up -d --build
```

默认访问 http://localhost:3000 并注册独立账号。公网部署请配置 HTTPS 反向代理，
将 `CAMOFY_PUBLIC_URL` 设置为浏览器使用的完整 origin，例如
`https://config.example.com`。反向代理需支持 WebSocket。
自托管实例不会连接其他 Camofy 云平台。

注册账号始终为普通用户。自托管管理员需由数据库运维显式授予：
`UPDATE users SET role='admin' WHERE id='<已核对的账号 UUID>';`。
在管理员“系统管理 → 订阅出口”创建并选定代理后，订阅才能刷新；
未配置出口时暂停拉取，不使用服务器直连。角色与迁移说明见
[账号角色与全局出口](docs/admin-egress-proposal.md)。

## 使用流程

1. 管理员在“系统管理 → 订阅出口”创建代理并设置全局生效出口。
2. 用户在“订阅源”添加 Clash YAML 订阅及刷新间隔，无需选择代理。
   订阅来源可选“WestData 账号”：填入账号、密码与服务 ID 后，云端每次刷新会
   自动登录面板、写回当前订阅地址、打开订阅更新开关并立即拉取。详见
   [WestData 账号订阅](docs/westdata-source.md)。
3. 创建功能 profiles，例如 [工作节点示例](examples/work-profile.yaml)。
4. 新建“身份”，关联任意订阅/独立 Profile，在关联上启用、禁用和排序。
5. 两条下发渠道：第三方客户端使用身份订阅 URL；路由器打开本地绑定页，通过云端登录授权绑定设备，云端分配身份并自动下发，用户无需配置订阅 URL。
6. 每个身份带有固定最后应用的系统 Profile，保护云端域名直连。设备详情页和路由器本地页面均可控制 Mihomo 启动、停止、重启。

Shadowrocket 提供节点 URI 订阅及 Clash 兼容 YAML 完整配置输出。
支持的协议/字段有明确范围，不能转换的目标返回诊断，不静默丢弃配置。
完整矩阵、同步约束及公开服务运维边界见 [云端设计与操作文档](docs/cloud.md)。

## 本地开发

需要 Rust 1.95+、Bun 和 PostgreSQL 16+。设置 `DATABASE_URL`、
`CAMOFY_ENCRYPTION_KEY` 和 `CAMOFY_PUBLIC_URL`。

```sh
cd web
bun install --frozen-lockfile
bun run build
cd ..
cargo run --bin camofy-cloud
```

云端默认监听 3000 并服务 `web/dist`。前端热更新使用 `bun run dev`；
此时将 cloud 的 `CAMOFY_PUBLIC_URL` 配成实际开发 origin（通常
`http://localhost:5173`），以通过 cookie 写操作的 Origin 校验。

```sh
cargo test --lib --bin camofy-cloud
# 必须使用专用测试数据库
cargo build --no-default-features --features agent --bin camofy-agent --example mock-core
TEST_DATABASE_URL=postgres://... CAMOFY_TEST_AGENT="$PWD/target/debug/camofy-agent" CAMOFY_TEST_CORE="$PWD/target/debug/examples/mock-core" cargo test --bin camofy-cloud cloud_end_to_end -- --ignored
CAMOFY_TEST_CORE="$PWD/target/debug/examples/mock-core" cargo test --no-default-features --features agent --bin camofy-agent -- --ignored
cd web && bun run build && bun run lint
```

## 轻量 Agent 与本地应急控制

全新安装（Linux amd64 / ARMv7）：

```sh
curl -fsSL https://camofy.app/install.sh | sh
# 只查看安装计划，不修改设备
curl -fsSL https://camofy.app/install.sh | sh -s -- --plan
```

脚本、版本查询、Agent 和 Mihomo 下载均经同一云端域名完成；设备不需要访问 GitHub。
安装器校验 GitHub 提供的 SHA256，仅接受新版 Agent 发布包，不会覆盖既有安装。
初始 Mihomo 为停止状态；打开安装完成时显示的本地地址，授权绑定后再明确启动。
若最新 Release 尚无新版 Agent，安装器会停止并保留设备原状，不能使用旧版单体替代。
自托管、发布要求及镜像 API 见 [安装与下载分发](docs/downloads.md)。

从源码构建：

```sh
cargo build --release --no-default-features --features agent --bin camofy-agent
# 安装和运行必须由设备所有者主动执行；不会自动部署到路由器
camofy-agent /etc/camofy/agent.json
```

配置参考 [agent.json](examples/agent.json) 和 [本地覆盖](examples/local.yaml)。
绑定和设备控制见 [设备授权](docs/device-authorization.md)。本地界面不提供订阅编辑；初次访问跳转绑定页，绑定后显示运行状态和内核控制。
手动部署需预先安装 Mihomo；一键安装器会下载它，Agent 运行时不执行内核下载/升级。设备配置和令牌文件应只允许
服务账号读取。TUN/防火墙适配必须在目标硬件验证后再启用。

旧的路由器 Web 单体不再是构建目标。工作树中原有的本地未提交路由器源码修改
被保留，但不进入云端或 Agent 二进制。

十万用户是架构目标，尚未进行十万在线设备的容量认证。实际容量取决于活跃连接、
订阅大小与刷新频率，需要按部署环境压测。
