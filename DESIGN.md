# Camofy（路由器版 Mihomo Web 管理面板）设计文档

## 1. 项目概述

- **定位**：  
  类似 clash-verge-rev 的 Mihomo 管理工具，但运行环境从 PC 客户端变为华硕路由器上的 Web 管理面板，提供订阅管理、内核下载、内核运行控制和本地自定义配置合并等能力。
- **目标用户**：  
  使用华硕路由器（支持 `/jffs` 持久化存储）的家庭和个人用户，希望在路由器侧部署 Mihomo，实现全网透明代理。
- **核心功能**：  
  - 配置和管理订阅链接  
  - 下载并缓存订阅内容  
  - 下载并管理 Mihomo 内核（不同架构版本）  
  - 控制 Mihomo 内核的启动、停止、重启  
  - 自定义本地配置，与订阅配置进行合并应用  
  - 在路由器重启后自动恢复运行状态（可选）

---

## 2. 运行环境与约束

- **硬件/系统**：
  - 主要运行环境为华硕路由器（假设为原厂或 Merlin 固件，具备 `/jffs` 分区），同时兼容经典 Linux（如 x86_64 / ARM 服务器或本地开发机）
  - CPU 架构可能为 ARMv7/ARMv8/MIPS/x86_64 等（Mihomo 内核下载时自动检测对应架构）
  - 内存、CPU 有限，需要后端和前端实现尽量轻量
- **文件系统与路径约定**：
  - 数据根目录按以下策略自动选择（不通过环境变量覆写）：
    - 若存在 `/jffs` 分区：默认使用 `/jffs/camofy` 作为数据根目录（典型路由器场景）
    - 若不存在 `/jffs`，但存在用户家目录（`$HOME`）：默认使用 `$HOME/.local/share/camofy` 作为数据根目录（经典 Linux 场景）
  - 在选定的数据根目录下，保持统一的子目录结构，示例（以 `/jffs/camofy` 为例）：
    - `<DATA_ROOT>/config/`：应用配置相关
      - `app.json`：应用自身设置（多订阅列表、当前活跃订阅、多用户 profile 列表、当前活跃用户 profile 及后续其他应用级设置）
      - `subscriptions/`：远程订阅 profile 目录，每个订阅一个子目录：
        - `<DATA_ROOT>/config/subscriptions/<id>/subscription.yaml`：从订阅源拉取的远程 profile（YAML 配置，后续如需规范化/转换也在此文件上直接覆盖更新）
      - `user-profiles/`：用户自定义 profile 目录，每个用户 profile 一个文件：
        - `<DATA_ROOT>/config/user-profiles/<id>.yaml`：用户 profile（YAML 配置，结构与订阅 profile 大致相同，支持 `prepend-rules` / `append-rules` / `prepend-proxies` / `append-proxies` 等增强字段）
      - `merged.yaml`：实际提供给 Mihomo 的合并后配置
    - `<DATA_ROOT>/core/`：Mihomo 内核
      - `mihomo` 或 `mihomo-<arch>`：内核二进制
      - `core.meta.json`：内核版本和架构信息
    - `<DATA_ROOT>/log/`：
      - `mihomo.log`：Mihomo 日志（可选）
      - `app.log`：本应用日志
    - `<DATA_ROOT>/tmp/`：下载临时文件
- **进程模型**：
  - `camofy`：Web 后端服务 + 控制逻辑，常驻进程/守护进程
  - `mihomo`：由 `camofy` 启动和管理的独立进程
  - 路由器开机启动：通过 `/jffs/scripts/services-start` 或类似启动脚本将 `camofy` 启动（后续可扩展）
  - 在经典 Linux 上可以通过 systemd、supervisord 等方式将 `camofy` 配置为守护进程，数据目录仍遵循上述策略

---

## 3. 总体架构设计

- **架构概览**：
  - 前端：基于 Vite + React 的单页 Web UI（HTML/JS/CSS），构建产物为静态资源文件，由后端二进制内嵌并按路径提供
  - 后端：运行在路由器上的轻量 HTTP 服务（Rust 实现），提供 RESTful API
  - 内核：Mihomo 进程，使用合并后的 YAML 配置文件运行

- **模块划分**：
  1. Web API 服务模块（HTTP Server）
  2. 订阅管理模块
  3. Mihomo 内核管理模块（下载、版本管理、启动/停止）
  4. 配置与合并模块（订阅配置 + 用户配置）
  5. 持久化存储模块（基于数据根目录 `<DATA_ROOT>`，在路由器上通常为 `/jffs/camofy`，在经典 Linux 上通常为 `$HOME/.local/share/camofy`）
  6. 状态与监控模块（内核运行状态、日志查看）

---

## 4. 模块设计

### 4.1 Web API 服务模块

- **职责**：
  - 提供统一的 HTTP 接口给前端调用
  - 提供静态文件（前端页面）服务
  - 承上启下：将用户操作转换为对订阅、内核、配置等模块的调用
- **技术选择（建议）**：
  - 语言：Rust（与当前仓库一致）
  - Web 框架：轻量框架，如 `axum` 或 `warp`（可根据后续需求选定）
  - 运行方式：独立监听一个端口（如 `0.0.0.0:3000`），通过路由器防火墙限制访问范围
- **主要接口（示例）**：
  - 订阅相关：
    - `GET /api/subscriptions` 获取订阅列表及每个订阅的基本信息、最后更新时间和拉取状态
    - `POST /api/subscriptions` 新增订阅（名称、URL 等基本信息）
    - `PUT /api/subscriptions/:id` 更新指定订阅的基本信息（名称、URL）
    - `DELETE /api/subscriptions/:id` 删除订阅（并清理该订阅对应的本地订阅配置文件）
    - `POST /api/subscriptions/:id/activate` 将指定订阅设置为“当前活跃订阅”（影响后续配置合并与 Mihomo 使用的订阅来源，但本里程碑不实现与内核联动）
    - `POST /api/subscriptions/:id/fetch` 手动拉取指定订阅的远程配置并更新本地订阅 profile（`subscription.yaml`）
  - 用户 profile 相关：
    - `GET /api/user-profiles` 获取用户 profile 列表及每个 profile 的基本信息（名称、最后修改时间、是否为当前活跃 user profile 等）
    - `POST /api/user-profiles` 新增用户 profile（名称、可选初始内容），在 `<DATA_ROOT>/config/user-profiles/` 下创建对应 YAML 文件
    - `GET /api/user-profiles/:id` 获取指定用户 profile 的 YAML 内容
    - `PUT /api/user-profiles/:id` 更新指定用户 profile 的 YAML 内容（完整覆盖写入）
    - `DELETE /api/user-profiles/:id` 删除用户 profile（并删除对应的 YAML 文件）
    - `POST /api/user-profiles/:id/activate` 将指定用户 profile 设置为当前活跃 user profile（影响后续配置合并，但不会直接重启 Mihomo）
  - 内核相关：
    - `GET /api/core` 获取当前内核版本、架构信息、下载状态
    - `POST /api/core/download` 从 GitHub 官方发布地址自动下载对应架构的最新版本 Mihomo 内核（自动检测架构）
    - `POST /api/core/start` 启动内核
    - `POST /api/core/stop` 停止内核
    - `GET /api/core/status` 查询内核运行状态（PID、端口、是否连通）
  - 配置相关：
    - `GET /api/config/merged` 查看当前生效的合并后配置（只读）
  - 应用设置：
    - `GET /api/settings`
    - `PUT /api/settings`
  - 日志与诊断：
    - `GET /api/logs/mihomo`（支持分页/尾部）
    - `GET /api/logs/app`

### 4.2 前端 Web UI 模块

- **实现方式**：
  - 使用 Vite + React + TypeScript 搭建单页应用（SPA），样式层统一使用 Tailwind CSS
  - 前端代码统一放在仓库根目录下的 `/web` 目录
  - 开发阶段通过 Vite dev server 进行调试（Bun 作为包管理与脚本运行工具，例如 `bun dev`）
  - 构建阶段使用 Bun 执行构建命令（如 `bun run build`）产出静态资源（HTML/JS/CSS），再通过 Rust 构建脚本或资源打包库（选型为 `rust-embed`）将构建产物打包进 `camofy` 二进制中，由后端在运行时直接从内嵌资源中提供静态文件服务（不依赖外部静态文件目录）
- **主要页面/功能**：
  - 仪表盘（Dashboard）：
    - 显示 Mihomo 运行状态（运行/停止）、当前延迟测试（可选）、订阅最后更新时间
  - 订阅管理页面：
    - 管理多个订阅的基本信息（名称、URL），支持新增、编辑、删除
    - 为每个订阅提供“拉取”操作，从远程订阅源获取配置并保存到本地 `<DATA_ROOT>/config/subscriptions/<id>/` 下
    - 选择“当前订阅”（活跃订阅），影响后续配置合并使用的订阅来源（本里程碑不直接联动 Mihomo 内核）
    - 显示每个订阅的最后拉取时间、拉取状态等基础信息
    - 订阅内容（YAML）在页面中只读，不提供直接编辑入口
  - 内核管理页面：
    - 显示自动检测到的架构
    - 一键下载/更新内核按钮（自动从 GitHub 官方发布地址下载对应架构的最新版本），显示进度与结果
    - "启动 / 停止 / 重启"按钮
  - 配置管理页面：
    - 左侧：订阅配置结构摘要（只读）
    - 右侧：用户自定义配置编辑器（YAML 文本编辑框）
    - “保存并合并”按钮，显示合并结果/错误信息
  - 设置页面：
    - 自动订阅更新周期
    - 路由器开机自动启动 Mihomo 与否
    - 面板访问密码（简易认证）

### 4.3 Profile 与订阅管理模块

- **数据结构**（示例）：
  - 系统中所有用于生成 Mihomo 配置的单元统称为 **profile**，分为两类：
    - `remote`：由订阅 URL 自动拉取的远程 profile；
    - `user`：用户自定义的本地 profile，用于在订阅基础上进行增强（如增加规则、代理、代理组等）。
  - `ProfileType`：
    - 字符串枚举，取值为 `"remote"` 或 `"user"`。
  - `ProfileMeta`（单个 profile 的元数据，存储在 `app.json` 中）：
    - `id: String`：profile 唯一标识（例如 UUID）
    - `name: String`：profile 名称（便于用户区分）
    - `profile_type: "remote" | "user"`：profile 类型
    - `path: String`：profile 文件相对路径（例如 `subscriptions/<id>/subscription.yaml` 或 `user-profiles/<id>.yaml`）
    - `url: Option<String>`：仅对 `remote` profile 生效，订阅链接
    - `last_fetch_time: Option<DateTime>`：仅对 `remote` profile 生效，最后一次成功拉取时间
    - `last_fetch_status: Option<String>`：仅对 `remote` profile 生效，最后一次拉取状态（例如 `"ok"`、`"request_failed"`、`"write_failed"` 等）
    - `last_modified_time: Option<DateTime>`：仅对 `user` profile 生效，最后一次保存时间
  - `AppConfig`（应用级配置）：
    - `profiles: Vec<ProfileMeta>`：profile 列表（包含所有 `remote` / `user` profile）
    - `active_subscription_id: Option<String>`：当前“活跃订阅”的 profile `id`（要求 `profile_type = "remote"`）
    - `active_user_profile_id: Option<String>`：当前“活跃用户 profile”的 `id`（要求 `profile_type = "user"`）
    - 后续可在此扩展其他应用设置（自动更新策略、面板密码等）
  - profile 对应的 YAML 配置文件示例路径：
    - 远程订阅 profile：`<DATA_ROOT>/config/subscriptions/<id>/subscription.yaml`
    - 用户 profile：`<DATA_ROOT>/config/user-profiles/<id>.yaml`

- **远程订阅 profile 流程**：
  1. 用户在前端订阅管理页面中新增订阅：输入订阅名称与 URL，提交后在 `app.json` 的 `profiles` 列表中新增一个 `profile_type = "remote"` 的 `ProfileMeta`，同时在 `<DATA_ROOT>/config/subscriptions/<id>/` 下预留对应目录；如当前无订阅，则自动将该 profile 设为活跃订阅（`active_subscription_id`）
  2. 用户可对任意远程 profile 执行“拉取订阅”操作，后端发起 HTTP 请求获取订阅内容（YAML/JSON/Vmess 列表等，后续可扩展转换）
  3. 拉取成功后，将内容保存为 `<DATA_ROOT>/config/subscriptions/<id>/subscription.yaml`，并更新对应 `ProfileMeta` 中的 `last_fetch_time` / `last_fetch_status`
  4. 用户可选择某个远程 profile 作为“当前活跃订阅”，后端更新 `active_subscription_id`；后续配置合并模块在生成 `merged.yaml` 时将以该远程 profile 的 `subscription.yaml` 作为订阅侧基础配置
  5. 用户可以编辑订阅的基本信息（名称、URL），也可以删除订阅；删除时会移除 `app.json` 中对应的 `ProfileMeta`，并清理由该订阅产生的本地订阅配置目录
  6. 远程订阅 profile 的 YAML 内容在 UI 中只读，不提供直接编辑入口；用户仅能通过用户 profile 和合并策略对生效配置进行个性化定制

- **用户 profile 流程**：
  1. 用户在“配置管理”或“用户 profile 管理”页面中创建新的用户 profile：输入名称、可选的初始内容，后端在 `app.json` 的 `profiles` 列表中新增一个 `profile_type = "user"` 的 `ProfileMeta`，并在 `<DATA_ROOT>/config/user-profiles/<id>.yaml` 中写入 YAML 内容
  2. 用户可以对任意用户 profile 执行“编辑/保存”操作，后端完整覆盖写入对应的 `<id>.yaml` 文件，并更新 `last_modified_time`
  3. 用户可以将某个用户 profile 设为“当前活跃用户 profile”，后端更新 `active_user_profile_id`；后续配置合并模块在生成 `merged.yaml` 时将以该用户 profile 的 YAML 作为用户侧基础配置
  4. 用户可以删除用户 profile；删除时会移除 `app.json` 中对应的 `ProfileMeta`，并删除对应的 `<id>.yaml` 文件

- **错误处理**：
  - 远程 profile：网络错误、解析错误写入 `app.log`，并在 `ProfileMeta.last_fetch_status` 中记录
  - 用户 profile：当解析用户 YAML 失败时，API 返回错误，不覆盖原文件，并在 `app.log` 中记录详细原因
  - 前端展示最近一次错误信息（例如拉取失败原因、保存失败原因）

### 4.4 Mihomo 内核管理模块

- **职责**：
  - 根据路由器架构下载对应 Mihomo 内核
  - 管理内核二进制文件及元数据
  - 启动、停止、重启内核进程
- **IPC 通信与控制**：
  - `camofy` 在生成 `merged.yaml` 时，同时为 Mihomo 配置一个仅本机可访问的控制端点，用于进程间通信（IPC），具体形式可以是：
    - 本地回环地址上的 HTTP 控制端口（例如 `127.0.0.1:<CONTROL_PORT>`），使用 Mihomo 的 external-controller HTTP API；
    - 或 Unix Domain Socket（例如 `<DATA_ROOT>/tmp/mihomo.sock`），只允许 `camofy` 进程访问。
  - 所有对 Mihomo 的配置热重载、状态查询、优雅停止等操作，均通过该 IPC 端点完成，对外仅暴露统一的 Web API（`/api/core/...`），前端不直接访问 Mihomo。
- **下载流程**：
  1. 自动检测当前路由器架构（`uname -m`, `/proc/cpuinfo` 等），将架构映射到 Mihomo GitHub 发布页面的对应架构名称（如 `linux-arm64`、`linux-armv7`、`linux-amd64`、`linux-mips` 等）
  2. 前端显示自动检测到的架构
  3. 用户在 UI 中点击"下载/更新内核"按钮，后端自动从 GitHub 官方发布地址（`https://github.com/MetaCubeX/mihomo/releases`）获取最新稳定版本的下载链接
  4. 根据自动检测到的架构，拼接对应的 GitHub Release 下载 URL（例如 `https://github.com/MetaCubeX/mihomo/releases/download/v{version}/mihomo-{arch}-{version}.gz` 或类似格式），将文件下载到 `<DATA_ROOT>/tmp/mihomo-xxx.tmp`
  5. 校验（文件大小/可选哈希）
  6. 解压（如为压缩格式）并将文件移动到 `<DATA_ROOT>/core/mihomo` 并设置执行权限
  7. 更新 `core.meta.json`（包括版本、下载时间、架构）
- **运行管理**：
  - 启动：
    - 生成/确认 `merged.yaml` 存在且合法
    - 为 Mihomo 生成包含 external-controller / IPC 设置的运行参数（例如 `external-controller: 127.0.0.1:<CONTROL_PORT>` 或指定 Unix socket 路径）
    - 调用 `mihomo -d /jffs/camofy/config -f merged.yaml` 或类似参数，将 Mihomo 作为 `camofy` 的子进程启动，并记录 PID 到 `/jffs/camofy/core/mihomo.pid`
  - 停止：
    - 首先通过 IPC 调用 Mihomo 的优雅停止接口（例如调用 external-controller 暴露的自定义 `shutdown`/`stop` 控制 API），等待最多 N 秒（例如 10 秒）
    - 在等待期间轮询检查子进程是否已退出；若在超时时间内退出，则清理 PID 文件并更新状态
    - 若超时仍未退出，则根据 PID 向子进程发送 SIGTERM，再等待一小段时间；如仍未退出则发送 SIGKILL 强制终止
    - 全程保持 Mihomo 作为 `camofy` 的子进程运行，不依赖系统级服务管理（不将 Mihomo 注册为 systemd/service 等独立服务）
  - 状态查询：
    - 检查 PID 文件是否存在
    - 验证 `/proc` 中是否存在相应进程
    - 通过 IPC 对 Mihomo 管理端点发送一次轻量请求（例如获取版本或当前连接数），用于检测控制通道是否可用

### 4.5 配置与合并模块

- **目标**：  
  提供一种可控的合并策略，将“远程订阅 profile 配置”（`profile_type = "remote"`）和“用户 profile 配置”（`profile_type = "user"`）合成为 Mihomo 实际使用的配置。
- **合并原则（建议）**：
  - 基于 YAML 的“深度合并”：
    - 标量（string/number/bool）：用户 profile 中的值覆盖远程 profile 中的值
    - 对象（maps）：按 key 深度合并，对应字段如果用户 profile 提供则覆盖；未提供则继承远程 profile 的值
    - 数组（lists）：
      - 默认策略：如果用户 profile 中显式提供了某个列表字段（如 `dns.servers`），则直接替换远程 profile 中同名数组；
      - 针对规则与代理相关列表，支持 **增强字段**：
        - `prepend-rules` / `append-rules`：仅在用户 profile 中使用，类型为规则列表：
          - 读取合并前的基础 `rules` 列表（若用户 profile 中显式提供 `rules`，则以用户提供的为基础，否则以远程 profile 中的 `rules` 为基础，如果两者均缺失则视为空列表）；
          - 将 `prepend-rules` 中的规则插入到基础 `rules` 之前，将 `append-rules` 中的规则追加到基础 `rules` 之后；
          - 最终输出配置中仅保留合成后的 `rules` 字段，不保留 `prepend-rules` / `append-rules` 字段。
        - `prepend-proxies` / `append-proxies`：仅在用户 profile 中使用，类型为代理列表：
          - 读取基础 `proxies` 列表（优先使用用户 profile 中的 `proxies`，否则使用远程 profile 中的 `proxies`，缺失则视为空列表）；
          - 将 `prepend-proxies` 中的代理插入到基础 `proxies` 之前，将 `append-proxies` 中的代理追加到基础 `proxies` 之后；
          - 最终输出配置中仅保留合成后的 `proxies` 字段，不保留 `prepend-proxies` / `append-proxies` 字段。
        - 类似地，可扩展 `prepend-proxy-groups` / `append-proxy-groups` 等字段，用于在不完全重写的情况下为 `proxy-groups` 追加或前置条目（设计上预留该能力，具体规则可在实现阶段细化）。
      - 用户 profile 中的特殊 `prepend-*` / `append-*` 字段只作为合并指令使用，不会出现在最终交给 Mihomo 的 `merged.yaml` 中。
    - 未识别/未知字段：保持“原样透传”（远程 profile 与用户 profile 的所有字段都保留，除非被用户 profile 在同路径上显式覆盖，或属于上述 `prepend-*` / `append-*` 辅助字段）
  - 禁止用户配置无效 YAML；解析失败时返回错误，并不更新 `merged.yaml`
- **实现方式**：
  - 使用 YAML 解析库，将远程 profile 与用户 profile 的 YAML 转换为中间结构（如 `serde_yaml::Value` / `Mapping`）
  - 对中间结构实现自定义合并逻辑，先做通用深度合并，再根据 `prepend-*` / `append-*` 等增强字段对 `rules` / `proxies` / `proxy-groups` 等关键列表进行二次处理
  - 合并时：
    - 订阅侧基础配置来自当前“活跃订阅”对应的远程 profile：`<DATA_ROOT>/config/subscriptions/<active_subscription_id>/subscription.yaml`；
    - 用户侧基础配置来自当前“活跃用户 profile”：`<DATA_ROOT>/config/user-profiles/<active_user_profile_id>.yaml`（如果未设置活跃用户 profile，可视为一个空配置）；
    - 合并完成后再次序列化为 YAML 写入 `<DATA_ROOT>/config/merged.yaml`
- **典型场景**：
  - 用户希望在订阅基础上增加少量自定义规则
  - 用户希望替换 DNS 配置、监听端口、外部控制端口等

### 4.6 持久化存储模块

- **职责**：
  - 提供对数据根目录 `<DATA_ROOT>` 下数据读写的统一封装（在路由器上通常为 `/jffs/camofy`，在经典 Linux 上通常为 `$HOME/.local/share/camofy`）
  - 确保存取路径、权限、安全性
- **功能**：
  - 初始化目录结构（第一次运行时创建 `<DATA_ROOT>/config`、`<DATA_ROOT>/core`、`<DATA_ROOT>/log`、`<DATA_ROOT>/tmp` 等子目录）
  - 读写配置文件（YAML/JSON），带简单备份机制（如 `.bak`）
  - 提供原子写入（写临时文件再 `rename` 覆盖）

### 4.7 状态与监控模块

- **内容**：
  - 运行状态：Mihomo 进程状态（PID、CPU/内存占用可选）
  - 配置状态：订阅更新时间、合并时间、最后合并结果（成功/失败）
  - 日志：最近 N 行日志，支持手动刷新

---

## 5. 核心业务流程

### 5.1 初始化流程

1. 在路由器上：开机后通过脚本启动 `camofy` 服务；在经典 Linux 上：由 systemd 等进程管理工具启动 `camofy`  
2. 应用根据运行环境自动选择数据根目录 `<DATA_ROOT>`（若存在 `/jffs` 则使用 `/jffs/camofy`，否则使用 `$HOME/.local/share/camofy`），并检查该目录结构，若不存在则创建  
3. 读取 `<DATA_ROOT>/config/app.json` 配置，若不存在则初始化默认配置  
4. 检查 `<DATA_ROOT>/core/mihomo` 内核是否存在，并记录状态  
5. 提供 Web UI 访问入口（端口、路径）

### 5.2 订阅管理与拉取流程

1. 用户访问 Web UI → 订阅管理页面  
2. 在“新增订阅”表单中输入订阅名称和订阅 URL，点击“新增订阅”  
3. 后端在 `<DATA_ROOT>/config/app.json` 的 `profiles` 列表中新增一个 `profile_type = "remote"` 的 `ProfileMeta`，如当前无远程订阅则将其设为活跃订阅（更新 `active_subscription_id`）  
4. 用户在订阅列表中选择某个订阅，点击“拉取”按钮  
5. 后端根据该订阅的 URL 发起 HTTP 请求，拉取成功后将内容写入 `<DATA_ROOT>/config/subscriptions/<id>/subscription.yaml`，并更新对应 `ProfileMeta` 的拉取时间与状态  
6. 用户可以在订阅列表中将某个订阅设为“当前订阅”，后端更新 `active_subscription_id`，后续配置合并模块在生成 `merged.yaml` 时将使用该订阅对应的远程 profile 作为基础  
7. 订阅内容（YAML）在 UI 中只读；用户在“用户 profile 管理/配置管理”页面中维护多个 `profile_type = "user"` 的用户 profile，并选择一个作为当前活跃用户 profile，由合并模块将该用户 profile 与活跃订阅 profile 合并生成 `merged.yaml`

### 5.3 下载 Mihomo 内核流程

1. 用户访问内核管理页面  
2. 系统自动检测并展示当前路由器架构及当前内核版本信息（如已安装）  
3. 用户点击"下载/更新内核"按钮  
4. 后端自动从 GitHub 官方发布地址（`https://github.com/MetaCubeX/mihomo/releases`）获取最新稳定版本，根据自动检测到的架构拼接下载 URL，进行下载、校验、解压、安装  
5. 更新 `core.meta.json`，前端显示下载完成状态

### 5.4 启动/停止内核流程

- 启动：
  1. 用户点击“启动内核”  
  2. 后端检查内核二进制是否存在、`merged.yaml` 是否存在且合规  
  3. 以子进程方式启动 `mihomo`，为其配置 external-controller / IPC 端点，并记录 PID  
  4. UI 显示运行状态为“已启动”
- 停止：
  1. 用户点击“停止内核”  
  2. 后端首先通过 IPC 向 Mihomo 发送优雅停止指令，并在限定时间内轮询进程退出状态  
  3. 若超时仍未退出，则根据 PID 向子进程发送 SIGTERM（必要时再发送 SIGKILL），确保内核进程被终止，并清理 PID 文件与状态  
  4. UI 显示运行状态为“已停止”

### 5.5 用户配置合并流程

1. 用户在配置管理页面选择或创建一个用户 profile（`profile_type = "user"`），并在编辑器中修改其 YAML 内容  
2. 用户点击“保存并合并”  
3. 后端解析用户 profile 的 YAML，如失败则返回错误并不覆盖旧文件  
4. 成功解析后覆盖写入对应的 `<DATA_ROOT>/config/user-profiles/<id>.yaml`，并更新 `last_modified_time`；若该 profile 被设为当前活跃用户 profile，则更新 `active_user_profile_id`  
5. 调用合并模块，将当前活跃订阅 profile（远程）与当前活跃用户 profile 进行深度合并，并应用 `prepend-rules` / `append-rules` / `prepend-proxies` / `append-proxies` 等增强字段，生成 `merged.yaml`  
6. 将合并结果状态告知前端，如成功可提示“可以重启内核以应用新配置”

---

## 6. 安全与访问控制

- 初期设计简单认证机制：
  - 在 `app.json` 中存储一个面板访问密码的安全哈希（推荐使用 Argon2id 等现代密码哈希算法，包含随机盐和参数配置）
  - 前端在首次访问时要求输入密码，后端验证成功后签发一个包含过期时间戳的签名 Token（例如基于 HMAC 的自定义 Token 结构），返回给前端
  - Token 由前端存储在浏览器（如 `localStorage`），并在后续请求中通过自定义 HTTP Header（如 `X-Auth-Token`）携带，减少 CSRF 风险
  - Token 设计包含明确的过期时间（例如数小时），后端在每次请求时进行签名校验与过期校验，过期后需要重新登录
- 访问限制：
  - 默认仅允许内网访问（监听 `0.0.0.0` 但由路由器防火墙控制）
- 日志与隐私：
  - 控制日志级别和日志保留长度，避免敏感信息泄露和占满持久化存储空间（无论是路由器的 `/jffs` 还是经典 Linux 上的 `<DATA_ROOT>`）

---

## 7. 日志与错误处理

- **应用日志（app.log）**：
  - 记录关键操作：订阅拉取、内核下载、启动/停止内核、配置合并结果等
  - 支持日志轮转/截断，防止持久化存储被写满：单个日志文件大小上限约为 1MB，最多保留 5 个轮转文件（超过后删除最旧的）
- **Mihomo 日志（mihomo.log）**：
  - 将 Mihomo 输出重定向到日志文件
  - 提供 Web UI 查看最近 N 行日志的接口，轮转策略与 `app.log` 保持一致
- **错误返回规范**：
  - API 返回统一的错误结构 `{ code: string, message, detail? }`，其中 `code` 为字符串形式的机器可读错误码（如 `"ok"`、`"subscription_fetch_failed"` 等）
  - HTTP 状态码可以统一使用 200（除非出现严重服务器错误），由前端主要依据 JSON 中的 `code` 字段判断请求是否成功
  - 前端对常见错误进行友好提示（网络错误、解析错误、下载失败等）

---

## 8. 扩展规划（后续可选）

- 支持多订阅源及订阅优先级策略
- 支持更多路由器平台（OpenWrt、其他品牌）
- 支持节点延迟测试、节点筛选与切换
- 提供简单的导入/导出配置功能
- 提供多语言 UI（简体中文/英文）

---

## 9. 里程碑规划

### 里程碑 1：项目骨架与基础运行

- 建立 Rust 后端基础工程结构（引入 Web 框架，如 `axum`）
- 实现 HTTP Server，提供健康检查接口（如 `GET /api/health`）
- 使用 `rust-embed` 在二进制中内嵌静态文件，实现仅基于内嵌资源的静态文件服务（预留前端入口）
- 在 `/web` 目录下使用 Vite + React + TypeScript + Tailwind 初始化前端项目，完成基础页面和路由结构，并打通前端构建产物打包到 Rust 二进制的基础构建流程（开发阶段可以暂时使用占位页面）
- 初始化数据根目录 `<DATA_ROOT>` 的目录结构（`config/`、`core/`、`log/`、`tmp/`），其中在路由器上通常为 `/jffs/camofy`，在经典 Linux 上通常为 `$HOME/.local/share/camofy`
- 在文档中记录基于路由器与经典 Linux 双场景的基本部署与启动方式

**验收标准**：  
在路由器或经典 Linux 上可启动 `camofy` 后端进程，浏览器可以访问基本页面并得到健康检查成功返回。

### 里程碑 2：订阅管理基础能力

- 设计并实现 profile 配置结构（`AppConfig` + `ProfileMeta`），支持多远程订阅 profile 及活跃订阅的管理，并为后续用户 profile 扩展预留结构（`profile_type = "user"`）
- 提供订阅相关 API：`GET /api/subscriptions`、`POST /api/subscriptions`、`PUT /api/subscriptions/:id`、`DELETE /api/subscriptions/:id`、`POST /api/subscriptions/:id/activate`、`POST /api/subscriptions/:id/fetch`
- 实现从订阅源拉取内容并持久化到 `<DATA_ROOT>/config/subscriptions/<id>/subscription.yaml`，并在 `app.json` 中记录每个远程订阅 profile 的最后拉取时间与状态
- 实现基础错误处理与日志记录（拉取失败原因写入 `app.log`，API 返回统一错误码）
- 前端订阅管理页面：支持多订阅的增删改查、手动“拉取订阅”、选择当前活跃订阅，并展示基本状态（本里程碑不实现与 Mihomo 内核的自动重启联动）

**验收标准**：  
用户通过 Web UI 管理多个订阅（新增、编辑、删除），可以选择当前活跃订阅，并成功为某个订阅执行拉取操作；订阅文件在 `<DATA_ROOT>/config/subscriptions/` 中可见，前端能够展示每个订阅的基本状态和活跃标记。

### 里程碑 3：Mihomo 内核下载与运行控制

- 实现路由器架构自动检测逻辑（`uname -m`、`/proc/cpuinfo` 等），将系统架构映射到 Mihomo GitHub 发布页面的架构名称，并在 API 中暴露
- 设计并实现内核元数据结构（版本、架构、下载时间）
- 提供内核相关 API：`GET /api/core`、`POST /api/core/download`、`POST /api/core/start`、`POST /api/core/stop`、`GET /api/core/status`
- 实现从 GitHub 官方发布地址（`https://github.com/MetaCubeX/mihomo/releases`）自动获取最新版本并下载对应架构内核的逻辑，完成下载、校验、解压、安装到 `<DATA_ROOT>/core/mihomo`
- 实现 Mihomo 进程启动/停止管理，记录 PID 并可查询状态
- 前端内核管理页面：展示自动检测的架构、版本信息、一键下载/更新按钮、启动/停止按钮与状态显示

**验收标准**：  
在配置合适的（临时）配置文件情况下，用户可以通过 Web UI 下载内核并启动/停止 Mihomo，状态展示准确。

### 里程碑 4：配置合并与用户配置管理

- 定义 YAML 合并策略并实现通用合并函数（基于 `serde_yaml::Value`）
- 实现用户 profile 的增删改查 API：`GET/POST/PUT/DELETE /api/user-profiles`、`POST /api/user-profiles/:id/activate`，对应 `<DATA_ROOT>/config/user-profiles/<id>.yaml`
- 实现合并 API：在拉取订阅或更新用户 profile 后自动生成 `merged.yaml`，提供 `GET /api/config/merged` 只读查看
- 增强错误处理：当用户配置无效 YAML 或合并失败时提供详细错误信息，同时保持旧版本 `merged.yaml` 不变
- 前端配置管理页面：用户 profile 列表与当前活跃用户 profile 选择器、用户 profile YAML 编辑器、保存并合并按钮、合并结果提示

**验收标准**：  
用户可以在订阅配置基础上编写自己的 YAML 配置，系统正确合并生成 `merged.yaml`，并在错误情况下清晰提示。

### 里程碑 5：开机自启动、安全与监控完善

- 在华硕路由器上配置 `/jffs/scripts/services-start` 或等价机制，实现 `camofy` 自启动
- 增加面板访问密码功能（`app.json` 中存储哈希 + 简单认证流程）
- 实现日志查看 API：`GET /api/logs/mihomo`、`GET /api/logs/app`，支持查看最近若干行
- 前端增加日志查看页面和简单监控信息（内核运行状态、最近错误）
- 优化日志轮转/截断策略，防止 `/jffs` 空间被占满

**验收标准**：  
路由器重启后 `camofy` 能自动启动，用户访问 Web 面板需要密码，日志与状态在前端可视化展示，系统能长期稳定运行。

---

## 10. 基本部署与启动方式（里程碑 1）

> 注意：以下命令以本地开发环境为例，实际路由器环境请根据架构选择合适的交叉编译和拷贝方式。

- 本地构建（建议使用 release 模式）：

  ```bash
  # 构建前端（在前端目录下）
  bun install
  bun run build

  # 构建后端二进制（会在构建过程中将前端产物通过 rust-embed 等方式打包进二进制）
  cargo build --release
  ```

- 将二进制拷贝到路由器（示例，无需单独拷贝静态资源）：

  ```bash
  # 假设路由器 SSH 地址为 192.168.50.1，用户名为 admin
  scp target/release/camofy admin@192.168.50.1:/jffs/camofy/camofy
  ```

- 在路由器上手动启动（数据目录示例为 `/jffs/camofy`，可根据需要调整）：

  ```bash
  # SSH 登录到路由器后
  chmod +x /jffs/camofy/camofy
  CAMOFY_HOST=0.0.0.0 \
  CAMOFY_PORT=3000 \
  /jffs/camofy/camofy &
  ```

- 在经典 Linux 上手动启动（假设使用默认数据目录 `$HOME/.local/share/camofy`）：

  ```bash
  chmod +x ./target/release/camofy
  CAMOFY_HOST=127.0.0.1 \
  CAMOFY_PORT=3000 \
  ./target/release/camofy &
  ```

- 访问方式：
  - 在局域网浏览器中访问 `http://路由器IP:3000/` 可以看到静态占位页面
  - 访问 `http://路由器IP:3000/api/health` 可以获得健康检查返回（如 `{"status":"ok"}`）
