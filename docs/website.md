# 官网与云工作区的域名分离

`site/` 是独立官网：原生 HTML/CSS/JavaScript，无前端依赖、第三方字体、跟踪脚本或账户数据。产品示意不读取真实账号或设备。可在不重建云业务镜像的情况下独立部署。

## 域名契约

| 地址 | 处理 |
| --- | --- |
| `https://camofy.app/` | 官网 |
| `https://cloud.camofy.app/` | 云工作区，原有账户和数据 |
| 主域名 `/sub`、`/api`、`/install.sh`、`/github`、`/downloads` 及其子路径 | 原云服务处理，不跳转、不截断路径 |
| 主域名 `/assets/*` | 保留旧版工作区静态资源兼容 |
| 主域名 `/site-assets/*`、`/robots.txt`、`/sitemap.xml` | 官网静态资源 |
| 其他主域名路径，如 `/identities/:id`、`/authorize?...` | 308 到工作区，保留完整路径、查询参数 |

参见 `site/edge.Caddyfile`。`handle` 是互斥路由，不能使用会剥离 `/sub` 或 `/api` 的 `handle_path`。云域名的 `/robots.txt` 禁止抓取，并返回 `X-Robots-Tag: noindex, nofollow`。

`CAMOFY_PUBLIC_URL` **仍为 `https://camofy.app`**，不改动现有订阅 token、Agent 地址或公开安装/下载地址。将 `https://cloud.camofy.app` 加入 `CAMOFY_LEGACY_ORIGINS` 信任列表（该配置本质上是允许的浏览器 Origin 列表）。保留已有兼容 Origin。不要用任意反射 Origin 或通配符绕过 CSRF 校验。

控制台使用同域 `/api` 和 WebSocket，不需要跨域 CORS。会话 Cookie 保持 host-only：迁移后用户需要在新域名登录一次，不扩大 Cookie Domain 到整个父域。旧域名上的设备授权入口跳转新控制台，原查询参数保留，已有 Agent 仍从主域名 API 轮询授权结果。

## 本地验证和镜像

```sh
node --check site/site-assets/site.js
node --test site/tests/site.test.cjs
docker build -t camofy-site:test site
docker run --rm --read-only --cap-drop ALL --security-opt no-new-privileges \
  --tmpfs /tmp:rw,nosuid,nodev,size=16m --memory 64m \
  -p 127.0.0.1:8080:8080 camofy-site:test
```

官网与路由器 Agent 发行流程分离；此镜像不包含 Rust 编译、Agent 构建、业务数据库或任何私有凭据。生产在本地构建、推送注册表，并由 Compose 按 digest 拉取，禁止服务器构建。基础 Caddy 镜像固定 digest。

验证应覆盖 1440px/390px/320px、无脚本阅读、键盘切换设备选项、FAQ 展开、复制成功/失败反馈、链接跳转和横向溢出。部署时验证公开路径没有返回官网 HTML、旧路径查询保留、云域名登录及授权不被 CSRF 拒绝。

## 发布安全

Cloudflare 添加 `cloud.camofy.app` 指向同一入口的代理记录，`proxied=true`（橙云），先保证入口 TLS 就绪再切换首页。不能改动其他 DNS 记录。保存生产 Compose、Caddy、环境文件和受保护容器快照；只新建官网服务，必要时以相同镜像重建云容器以应用 Origin 配置。禁止全栈 up/down，Caddy 只做验证后的热加载。检查无关服务容器 ID/启动时间及连续健康探针。

回滚时恢复 Caddy 路由及环境配置即可恢复旧首页；保留官网容器与 DNS 不影响旧服务。不要为回滚删除数据库、卷、镜像或任何用户数据。
