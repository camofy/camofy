# 本次验证记录

以下首版记录在本机及隔离 Docker 环境执行，当时未访问路由器。
2026-09-16 经用户明确授权完成了真机无 TUN 部署、配置迁移与端到端验证，
最新结果见 [路由器部署记录](router-deployment.md)。没有接管用户网络流量。

## 已验证

- `cargo fmt --all -- --check`：通过。
- `cargo clippy --all-targets --all-features -- -D warnings`：通过。
- 配置引擎：有序叠加、共享选择、重名/缺失引用/循环拒绝、输出格式差异、非法 YAML、路由器默认值不覆盖显式云配置。
- 安全与网络：加密认证、私网地址拒绝、SOCKS5 数字目标握手、保留 Host、禁止重定向。
- 独立 PostgreSQL + 实际 HTTP API：注册与租户隔离、跨租户引用拒绝、更新版本冲突、加密持久化、CSRF、令牌撤销、ETag、版本回滚。
- 实际模拟 HTTP 代理：认证、手动刷新和计划任务均经指定代理，目标端口没有服务器，排除了意外直连成功；并发 worker 只领取一次任务；损坏响应不替换已发布内容。
- 真实 Agent 进程连接测试云端：初次应用、WebSocket 驱动的新版本收敛、测速命令及回报。内核是本机测试替身，不是实际 Mihomo。
- Agent 应用：本地覆盖优先、控制接口绑定 loopback、哈希错误拒绝、内核校验失败拒绝、热重载失败恢复旧配置、无云连接时恢复本地有效版本。
- 前端 `bun run build`、`bun run lint`：通过。产物仅包含新云端 UI，旧页面不进入构建。
- Playwright 浏览器：注册、会话恢复、创建拉取代理、订阅选择代理与五分钟刷新、添加功能 Profile、组合发布、预览合并结果和生成订阅凭据。
- 桌面 1280×720 与手机 390×844 视觉检查；修复移动端横向溢出，最终 `scrollWidth === innerWidth === 390`。手机端可退出登录。
- `docker compose config --quiet`：通过。
- Linux `docker build --target verify`：5 个引擎测试、4 个安全/转换测试和 Agent 回滚/恢复测试通过。
- Linux 生产镜像构建与本机启动：通过；数据库迁移成功，健康检查与前端页面返回 200，容器以非 root 的 UID 10001 运行。
- Linux 容器内完整 PostgreSQL + 真实 Agent 跨进程测试：通过（另建隔离测试库）。
- 浏览器设备创建、配置绑定与设备凭据生成：通过；只有设备凭据显示 Agent 启动示例，普通订阅令牌不会误导用户用于 Agent。

Playwright 截图保存在工作区 `output/playwright/`（不提交）。浏览器注册前的
`/api/auth/me` 401 是预期未登录状态；登录后的最终页面没有控制台错误。

## 复现

根 README 包含 Windows/Linux 通用的 Cargo 命令；Windows 需使用 `.exe` 后缀与
PowerShell 环境变量语法。数据库测试必须显式设置专用 `TEST_DATABASE_URL`。
给集成测试设置 `CAMOFY_TEST_AGENT` 和 `CAMOFY_TEST_CORE` 才会包含跨进程 Agent 验证。

```sh
docker build --target verify -t camofy-verify .
docker build -t camofy-cloud .
```

`verify` 构建目标执行 Linux 单元/网络测试与本机假内核 Agent 测试。
PostgreSQL 的完整测试另由 CI 提供临时数据库执行，不访问生产环境。

## 尚不能宣称已验证

- 十万账号/在线设备的压力、稳定性和成本测试。当前是可横向扩展的队列、数据库与通知设计，不是容量认证。
- 真机路由器的 TUN、DNS、iptables、断电恢复、常驻资源占用及各 CPU 架构运行情况。须经设备所有者授权后测试。
- 实际 Clash Verge Rev、Shadowrocket 导入/路由行为；Shadowrocket 完整输出目前是有兼容性限制的 Clash YAML，不是原生 `.conf`，不支持的格式明确返回错误。
- 任意第三方客户端实时强制切换节点。第三方客户端由自身刷新和本地选择策略决定，实时控制仅适用于 Camofy Agent。
- 公网 HTTPS 代理的实际运营商链路、全部认证方式与证书环境。目前 TLS 保持证书验证，不以跳过验证换取兼容。
- 公开运营所需的邮件验证、密码找回、防滥用、监控告警和备份恢复演练。这些不是本次功能首版的完整交付范围。

前端工具链仅有 Browserslist 数据陈旧提示，不影响构建；没有为消除此提示进行额外依赖升级。

## 清理

验证结束后已停止本机测试服务与浏览器，移除本次创建的两个临时容器及其
临时数据库卷（仅含模拟账号/订阅，不保留恢复副本）。本地构建镜像与截图保留，
没有停止或修改工作区外其他容器，也没有改动真实设备。
