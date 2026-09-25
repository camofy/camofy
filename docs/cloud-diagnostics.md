# 云端关键链路诊断

云端默认 `RUST_LOG=info`。本项目审查了云端 API、后台刷新、DNS/代理/面板/订阅下载、
验证码识别、设备回报、配置构建、认证、商店发布和公开软件下载；桌面端及 Agent 的
核心启停/调度已有各自的本地运行日志，不随 `camofy-cloud` 容器发布。

## 一次订阅刷新如何追踪

1. 在刷新历史取任务时间、结果和诊断码；服务日志中找相应的
   `subscription refresh claimed` 和 `claim` UUID。随后过滤同一个 `claim`。
2. `subscription_refresh_attempt` span 给出尝试次数。代理提取用
   `provider DNS ...` / `provider API ...` 识别解析器、候选地址、请求阶段和 errno。
3. `subscription egress prepared` 与 `WestData panel ...` 表示已获取代理及面板进度；
   Zyte 的 `zyte_conversation` 额外用独立 UUID 关联页面、登录、产品和激活步骤。
4. `subscription fetch started` 后，依次查看 `subscription DNS ...`、
   `subscription proxy destination pinned`、`subscription_proxy_bridge` 下的
   `proxy_connection`、`subscription response headers received` 和
   `subscription fetch completed`。代理桥接错误包含 `phase`、`proxy_addr`、
   `target_addr`、底层 I/O 类别和 OS 错误码；HTTP 请求错误有连接/超时/正文标志。
5. `subscription refresh result published` 只在数据库提交完成后记录。失败不会
   覆盖上次发布的订阅内容；单次超时、请求超时与 DNS 超时有不同的阶段日志。

所有关键外部请求只记录域名、数值状态、地址、时长、字节数和错误类别。
不要把含 token 的订阅 URL、携趣 uid/vkey、Zyte 会话或 API key、账号、
验证码、Cookie、请求/响应正文写入日志。受控解析可能返回多个地址；候选地址
不是每一个都曾真正连接，代理桥接的固定 `proxy_addr` 才是该连接使用的地址。
原始 `reqwest::Error` 的 Display 可能包含 URL，统一使用
`security::log_network_failure` 提取结构化字段。日志不是抓包：若发生静默丢包，
应按日志确定故障跳点，再用短时 TCP 抓包与代理商的连接记录核对。

## 其他关键事件

- 平台出口配置、用户资源编辑、手动刷新排队、配置 revision 与设备控制操作
  在提交后记 ID/版本/结果，不记表单或完整报错正文。
- Zyte、视觉服务、WestData 非 Zyte 页面请求、公开版本下载的失败记请求阶段与
  结构化网络诊断；面板浏览器回退、会话重启和验证码预处理回退均有事件。
- 后台清理与通知监听的数据库故障记数据库错误码；设备报告失败和忽略过期
  控制命令均有 ID 可追踪。
- 生产 Compose 的 Docker `json-file` 日志滚动限制仍为 20 MiB × 3；
  超出保留期的旧事件不会因为本次增加日志而恢复。常规健康探测不逐条记录。

本地验证使用 `cargo fmt --check` 和 `cargo test --locked --lib --bin camofy-cloud`；
需要真实数据库的端到端测试、真实 WestData/携趣调用保持显式 opt-in。
