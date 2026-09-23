# WestData 账号订阅（需要图片识别的面板）

WestData 面板（`wd-gold.com` / `wd-gold.net`）默认**关闭订阅更新**，只在点击
「打开订阅更新开关」后的 **十分钟内**提供订阅内容，其余时间订阅链接返回 404。
因此普通的定时刷新会一直失败。选择本供应商后，云端每次刷新都会：

1. 登录面板（账号 + 密码 + 图片字符识别）；
2. 读取当前订阅地址，转换为面板同款 Clash 地址；
3. **把它写回订阅源的 `url` 并入库**；
4. 打开订阅更新开关（十分钟有效）；
5. 立即用平台统一出口拉取该地址，失败仍保留上次成功配置。

面板没有开放 API，也没有浏览器可用，所以整条链路是纯 HTTP：Cookie 会话、
表单提交与图片识别，云端不启动浏览器、不保存页面内容。

## 使用

在「订阅源」新建或编辑时，把**订阅来源**选为「WestData 账号（自动获取并激活订阅）」，
填写登录账号与密码，然后点**「扫描账号下的产品」**：云端登录一次面板，列出该账号下的
全部服务（名称、状态、下次到期日、服务 ID），选择其中一个即可，无需手工查地址栏。
同一账号常有多个同名产品，只有状态与到期日能区分，所以列表必须带这两列。
只有一项时会自动选中。

界面以扫描为主路径：默认不显示产品 ID 输入框，只有点「手动填写产品 ID」才展开，
旁边可「返回扫描」；扫描失败只提示错误并保留重试/手动两个入口，不会误报成"账号下没有产品"。

也可以把产品 ID 留空：刷新时会自动列出产品，若账号下**只有一个**产品就直接使用；
有多个则报错提示先在订阅源里选择，绝不猜测。

切回「直接订阅 URL」即可恢复普通的固定订阅地址，账号信息同时被清除。
自动刷新、手动刷新、首次拉取共用同一条队列与同一个平台出口，行为与普通订阅一致。

扫描与刷新默认一样经平台订阅出口（每次提取一个短效出口 IP），按用户限速 10 次/分钟，
返回内容只有 ID、名称、状态与到期日，不回显密码、Cookie 或页面正文。
出口 IP 被 Cloudflare 挑战时可另配一条**面板专用出口**（见下节「面板出口代理」），
扫描与刷新都会使用它。

## 面板出口代理（`panel_proxy`）

Cloudflare 是按出口 IP 的信誉下发托管挑战的：同一个客户端、同一份代码，换个干净出口就是
200。因此订阅源支持给**面板会话单独指定一条出口**，与订阅地址本身的抓取解耦：

- 字段：WestData 账号区里的「面板出口代理（可选）」，形如
  `http://user:password@host:8080`，支持 `http` / `https` / `socks5`（`security::egress_client`
  同一套校验与握手实现，含代理认证）。
- 生效范围：**只作用于面板会话**（登录、读产品页、开订阅开关、扫描产品）。
  订阅地址 `wd-turbo.com` / `api.wd-turbo.com` 的抓取始终走平台出口，不受影响。
- 留空 = 沿用已保存的地址（与密码一致，接口从不回显它）；
  填写 `none` = 清除，面板会话回到平台出口。
- 它存在 `data.westdata.panel_proxy`，与账号密码一起 AES-256-GCM 加密入库；
  列表/保存响应、日志、刷新历史里都不会出现它。

配置位置：编辑订阅源 → 订阅来源选「WestData 账号」→ 「面板出口代理（可选）」。
命令行联调可传 `CAMOFY_WESTDATA_PANEL_PROXY=…`。

## 凭据存放与回显

- 账号、密码、服务 ID、面板出口代理存在该订阅源自己的 `data.westdata` 中，随整条
  `resources.data` 一起用 AES-256-GCM 加密入库，与代理凭据同一套机制。
- 接口从不回显密码与面板出口代理：列表和保存响应都会删除这两个字段；编辑时留空即保留原值。
- 日志只记录步骤名、HTTP 状态码与响应体长度，不记录 URL、Cookie、表单值或页面正文。
  刷新历史里的失败信息是固定文案（含具体步骤），不含任何密钥。
- 服务 ID 不写死在代码里：它属于单个账号，由使用者在界面上填写。

## 图片识别

验证码图片先在进程内解码、二值化为黑白字模并用最近邻放大六倍（这一步是识别的关键，
原始 100×24 图片直接交给模型并不可靠），再以 data URL 发给配置的视觉接口。
提示词只描述「把图片里的文字转成文本」——按登录验证码提问会触发模型的敏感检查与长篇推理，
反而拿不到字符。请求同时关闭思考模式（`{"thinking":{"type":"disabled"}}`），
否则推理会吃掉全部输出预算；若网关不认识该字段，会自动去掉重试一次。

识别结果按「大写字母数字、4–12 位、优先同时含字母与数字」的规则提取，最多重试三次，
每次重新取图并重新识别；三次都失败才判定本次刷新失败，不影响已有配置。

## 运维配置

| 变量 | 默认值 | 说明 |
| --- | --- | --- |
| `CAMOFY_VISION_API_KEY` | 空 | 未设置时**不启用**本功能，保存 WestData 订阅源会被拒绝 |
| `CAMOFY_VISION_API_URL` | `https://api.deepseek.com/chat/completions` | OpenAI 兼容的对话补全地址 |
| `CAMOFY_VISION_MODEL` | `deepseek-flash` | 支持图片输入的模型名 |

该接口由云端**直连**（不走平台订阅出口、不跟随重定向、校验公网 IPv4），
只发送处理后的验证码图片，不包含账号、密码或订阅地址。
`CAMOFY_ALLOW_PRIVATE_EGRESS=true` 时，视觉接口与服务面板都允许解析到私网地址，
仅用于本机/内网部署；验证码图片在无密钥、识别失败或响应异常时都不会退化为明文直连。

`CAMOFY_WESTDATA_SITE` 与 `CAMOFY_WESTDATA_CONVERT` 可改写面板与转换站点，
只为本地联调与自建镜像保留，默认为官方域名。

面板登录、读取订阅、打开开关**共用平台订阅出口**，出口不可用即失败，不直连降级。
出口 IP 被 Cloudflare 拦截时，历史里会给出 `westdata_egress` 步骤提示更换出口。

### 出口必须是一个不被 Cloudflare 挑战的地址

面板域名（`wd-gold.net` / `wd-gold.com`）对部分来源下发 **Cloudflare 托管挑战**
（`cf-mitigated: challenge`、`Just a moment...`、`_cf_chl_opt`）：挑战页要求执行
JavaScript，纯 HTTP 客户端直接以 403 结束；而在被标记的出口上，**即使执行 JS 也过不去**。

2026-09-23 在同一台生产机上实测（每次只换一个变量）：

| 客户端 | 出口 | 结果 |
| --- | --- | --- |
| plain curl | 服务器自身 IP（154.36.168.183，US/NetLab） | 403 挑战 |
| `curl_cffi` 模拟 Chrome / Safari / Firefox | 服务器自身 IP | 403 挑战 |
| plain curl | 携趣短效地址（辽阳联通 / 宿迁电信 / 南通电信 / 淮南电信 等 4 个地址） | 403 挑战或连接重置 |
| `curl_cffi`，11 个浏览器指纹（chrome / chrome131 / 124 / 120 / 116 / 110、edge101、safari17_2_ios、safari17_0、firefox133 / 135） | 携趣（`49.70.23.20`，`cdn-cgi/trace` 确认出口一致） | **全部 403，同一个挑战页** |
| 无头 Chromium（Playwright） | 携趣 | 403 → 升级为「请稍候…」，7 秒后仍卡住 |
| **有头真 Chrome**（channel=chrome，非自动化参数） | 携趣 | 403 → 「正在进行安全验证…正在验证…」**转圈 30 秒不放行，始终没有 `cf_clearance`** |
| **有头真 Chrome（同一客户端）** | **WestData 节点** | **200，登录表单直接出现** |
| Chromium net DLL（Chromium 141 的 `net` 栈，Cookie 上下文共享，首页→clientarea→重试） | 携趣（`221.227.255.222`，江苏南通电信） | **403，`server: cloudflare`、`cf-mitigated: challenge`，全程 0 Cookie** |
| 同一个 Chromium net DLL | WestData 节点 | 200，登录表单出现，拿到 `WHMCS` 会话 Cookie |
| reqwest（云端实际使用的客户端） | WestData 节点 | 200（联调测试全绿） |

结论：**这是按来源 IP 段下发的托管挑战，不是 TLS/UA 指纹问题，也不是"缺少 JS 执行"能解释的**——
同一个浏览器、同一台机器只换出口，携趣永远停在验证页，机场自己的节点直接 200。
连 Chromium 自己的 `net` 栈（`chromium_net.dll`，非浏览器但网络实现与 Chrome 同源）在携趣出口上
同样只拿到 `cf-mitigated: challenge`，换出口即 200。因此：

- 伪造 JA3/JA4 指纹（`curl_cffi`、`rquest` 一类）无效，不要往这个方向修；
- 携趣整条产品线都是公开代理 IP，属于风控标记段，换多少个 IP、换什么客户端都一样。

可行做法只有两条：

1. 给面板登录换一个不被挑战的出口。实测机场节点与海外干净出口可行，且订阅拉取在这些出口上
   同样可用，因此可以整条出口替换，或另配一条仅供面板登录使用的出口策略
   —— 后者已实现为上面「面板出口代理」字段；
2. 让供应商提供不走 Cloudflare 的面板入口。

2026-09-24 又把这三种"绕过客户端"的方向全部实测排除，结论没有变化：

| 客户端 | 出口 | 结果 |
| --- | --- | --- |
| 有头真 Chrome（Playwright，含 `--disable-blink-features=AutomationControlled`） | 2captcha 住宅代理（JP/FR/RU/IT/CA/ES/BR/MY 轮换） | 403 挑战，`cf-mitigated: challenge`；页面停在「请稍候…」，`cf_chl_rc_ni` 限速循环 |
| **完全不带 CDP 的普通 Chrome**（`--remote-debugging-port`，加载期间无任何自动化连接） | 同一住宅出口 | 同样停在挑战页 120 秒以上，始终没有登录表单 |
| 真 Chrome + 干净的 CN 住宅出口（2captcha 云浏览器，吉林联通） | CN 住宅 | 403 挑战 |
| **2captcha 云浏览器（Browser API）+ 其内置 `Captcha.setAutoSolve`** | 其免费 IPv6 出口 | 403 挑战；`Captcha.solve` 返回 `solveFailed`，内置解题器识别不到托管挑战 |
| 纯 HTTP（`curl` / 应用内的 reqwest） | 本机直连出口（TW，Kirino LLC） | **200，7842B 登录页，`signinform` 齐全，零挑战** |

即：**同一时刻、同一套客户端，唯一变量是出口 IP**。挑战页要求执行 JS，而被标记的出口上
即使真实浏览器也只会被无限限速循环；换到干净出口后，连 `curl` 都直接拿到登录页。
`cf_clearance` 不能跨客户端复用（它绑定出口 IP 与浏览器指纹，实测同 IP 同 UA 换 curl 仍 403），
所以"在别处过一次挑战再把 Cookie 搬回来"也不可行。

**不要把希望放在客户端伪装上**：已验证连有头真实 Chrome 都无法通过携趣出口的验证页。
云端也不引入浏览器：解 JS 挑战需要真实浏览器运行时，而在这个出口上它同样过不去，
与"镜像内无浏览器、最小体积"的既有约束冲突。定位这类问题时，服务日志只应记录
"收到挑战/拦截页"这一事实与状态码，不记录页面正文，也不记录 Cookie。

### 为什么不能"不加载浏览器直接解算"

2026-09-23 穷尽搜索了这条路（只审计源码，不运行第三方预编译产物）：

| 类别 | 项目 | 结论 |
| --- | --- | --- |
| 纯 JS 引擎 + 模拟 DOM（**真正的无浏览器**） | `Advik-B/cloudscraper`（Go/otto）、`sriharsha-y/go-cfscraper`（Go/goja + JA3）、`sayem314/hooman`（Node/jsdom）、`Anorov/cloudflare-scrape` 与 `VeNoMouS/cloudscraper`（Python） | **只覆盖 v1（`jschl_vc` 数学题）与老式 v2/v3（`window._cf_chl_opt`）**。对现代托管挑战实测失败（见下） |
| 号称"无需浏览器"的封装 | `CircuitSavage/turnstile-curl`、`Maas6696/cloudflare-challenge-clearance`、`biusberline/cloudflare-turnstile-solver`（均转 Peak 付费 API）、npm `cfsolver`/`cloudbypass-skill`（转穿云/CloudFlyer），Go 库的 Turnstile 分支（源码里 `if s.CaptchaSolver == nil { return ErrNoCaptchaSolver }` → 2captcha） | "无浏览器"只是**浏览器不在你这里**：还要把目标 URL（以及部分实现的代理凭据）交给第三方 |
| 真浏览器方案 | FlareSolverr、`Xewdy444/CF-Clearance-Scraper`、camoufox/patchright/nodriver 系 | 需要浏览器，且实测携趣出口撑不住挑战自身的子请求 |

纯 Go 解算器在本面板上的实测（`cfscraper`，经携趣出口）：

```text
[cf] Modern (v2/v3) JavaScript challenge detected. Solving with 'goja'...
[cf] goja: warning, a script block failed to run: ReferenceError: location is not defined
Get failed: v2 challenge solver failed: goja: answer value is empty or undefined
```

同一程序换到不被挑战的出口：`HTTP 200, 8038 bytes, loginform=true`。

**把这条路修到底的结果**（不是只读代码下的结论）：

1. 自己重写了它的 960 字节 DOM shim（原版 `createElement` 里引用了未定义的 `domain`，且 `getElementById`
   每次返回新对象，脚本写入的值读不回来）——补上 per-id 元素缓存、`location`/`navigator`/`document`、
   canvas/WebGL/`OfflineAudioContext` 桩、同步定时器后，`location is not defined` 消失；
2. 仍然失败，且原因变了：挑战页里只有 1 个内联 `window._cf_chl_opt` 脚本，它做的事是**注入一个外部
   `<script>`**，真正的挑战逻辑在 `…/orchestrate/chl_page/v1`（实测 233–238 KB）；
3. 再往下一层：自己抓下那个 bundle 塞进 goja 运行 —— **66 ms 跑完、零报错、零网络请求**，
   只记录了 `createdTags: ["script"]`、`timers: 1`：它把工作继续交给下一层（Turnstile loader），
   而那一层需要完整浏览器环境。

也就是说，**当前这一代挑战的逻辑根本不在 HTML 里**，纯 JS 引擎要复刻的是整个浏览器环境
（canvas/WebGL/audio 真机指纹、`isTrusted` 交互、`Function.prototype.toString` 原生代码校验、
JA3/JA4 与 UA 一致、PoW），且 token 一次性、服务端 `siteverify` 校验。
连最新的 Rust 实现也这么写：`cloudscraper-rs` 的 `browser` feature 是
"Headless-browser fallback for interactive challenges (`orchestrate/chl_page`)，需要 Chrome/Chromium 二进制"。
所以**不存在可用的开源"无浏览器解算"实现**：要么换出口（推荐，该出口根本不再被挑战），
要么接受付费第三方（并把面板地址/代理凭据交给它，不建议）。

**付费解算服务（2captcha）也去掉了浏览器依赖吗**：没有。按其官方文档，托管挑战属于
"Cloudflare Challenge page"，要用 Turnstile 任务（`TurnstileTask` + 你的代理）并额外传
`data`(cData)、`pagedata`(chlPageData)、`action` —— 这三个值要**在页面里拦截 `turnstile.render`**
才能拿到，拿到 token 后还要**执行页面的 callback** 才算把 token 用上去。
也就是说它只替你"解"，不替你"有页面运行时"；而我们的云端是纯 HTTP worker，没有回调可执行。
另外其代理文档明确写着：*如果我们无法通过你的代理打开目标站，我们就不会使用你的代理* ——
携趣这种不稳的出口一旦连不上，解算会退回他们自己的 IP，而 `cf_clearance` 要求 IP 匹配，
结果对我们就无效。实测中该 key 的 API 可用（余额、任务创建、轮询均正常，
`AntiCloudflareTask` 已不是有效类型，会返回 `ERROR_TASK_ABSENT`），但每个携趣出口都在
流程中途中失效，无法端到端完成一次挑战页解算。

**免费与付费的其它路都试过了，结论如下**：自建 CF Worker 让请求从 Cloudflare 自己的网络发出，
并不能免挑战（实测 `www.cloudflare.com` 200，而 `linux.do`、`wd-gold.net`、`wd-gold.com`
一律 403 `cf-mitigated: challenge`）；2captcha 的 Scraper API 能取到面板登录页
（`x-debug.price=0.0005`，$0.5/1000 次，说明干净出口确实能过），但它的 `scrape` 方法只有
`url`/`data_format`/`format`/`waitFor`/`fullPage`/`cdpurl`，**没有 `method`/`post_data`/`cookies`，
只能 GET**，做不了登录 POST 与会话；2captcha 的 Browser API（云浏览器 + CDP）三种配置实测均失败：
`proxyMode:none` 时云浏览器同样被挑战、`Captcha.setAutoSolve` 无事件、显式 `Captcha.solve`
（`*` 与 `turnstile` 两种）都返回 `solveFailed`；传我们自己的代理时 CDP 连接直接被拒
（`500 proxy_error`）；`our_proxy` 模式要求先购买他们的住宅代理（`ERROR_PROXY_ACCOUNT_ID`）。

## 刷新与重试
面板会话处在 30 秒的单次尝试预算内：登录（含识别）通常 3–5 秒。
一次刷新最多三次尝试，第一次解析出的订阅地址在 8 分钟内（`retry::PANEL_REUSE`，
小于开关的十分钟有效期）会被后续尝试复用，避免重复登录与重复识别；
若拉取返回 401/403/404（开关过期或链接被面板更换），缓存立即失效并在下次尝试重新登录激活。

## 验证记录（2026-09-23）

- 单元测试：图片解码（五种过滤器、调色板/灰度位深、二值化阈值、放大）、
  模型输出提取、登录表单与订阅地址解析、Clash 地址拼接、表单编码、凭据校验与脱敏。
  `cargo test --all-features --bin camofy-cloud` 全部通过。
- 真实面板联调（`live_panel_reads_a_working_clash_subscription`，默认 `--ignored`）：

  ```sh
  CAMOFY_VISION_API_KEY=… CAMOFY_WESTDATA_USER=… CAMOFY_WESTDATA_PASS=… \
  CAMOFY_WESTDATA_PRODUCT=… cargo test --all-features --bin camofy-cloud -- \
      --ignored --nocapture live_panel
  ```

  实测流程：登录页 8038B（含 token）→ 取图 → 识别通过 → `dologin.php` 302 →
  客户中心 36471B（已登录）→ 产品页 75547B → `ActivateSublink` 返回 `success` →
  Clash 转换 177B、21 个节点、用量状态 `ok`；
  随后扫描产品列出 4 项服务（1 项「有效的」、3 项「已终止」，含到期日），
  配置的产品 ID 在其中。首次识别失败会重取图片重试，测试中出现过一次误识并自动恢复。
  单次识别成功时全流程约 3.4 秒。
- 本机位于透明代理（fake-IP DNS）后，联调时用 `CAMOFY_WESTDATA_PRIVATE=1`
  放宽地址校验；生产环境解析公网地址，不需要该开关。
- 期间修正的两个真实缺陷：`__CF$cv$params` 出现在该域名**每一个**正常页面上，
  不能作为 Cloudflare 拦截判据；登录完成判据必须是「存在 `logout.php` 且没有登录表单」，
  否则挑战页会被误判为已登录。
- 未验证：并发多实例下同一账号的登录频率、面板改版导致的选择器变化、
  以及视觉模型在更复杂验证码上的长期成功率。
- 生产环境首次使用即命中托管挑战（见上节）；当时携趣出口与服务器自身出口都被挑战，
  说明"能跑通"依赖一个不被挑战的出口，而不是客户端实现细节。

## 验证记录（2026-09-24）

在干净出口上用**纯 HTTP、无浏览器**复跑了一遍完整面板流程（与云端 reqwest 客户端同形态）：

```
GET  /clientarea.php                                 200   7842B  signinform ✓  无挑战
GET  /includes/verifyimage.php                       200   1824B  验证码图片
POST /dologin.php                                    302 → /clientarea.php
GET  /clientarea.php                                 200  35634B  已登录（logout.php ✓）
GET  /clientarea.php?action=productdetails&id=…      200  73673B  2 条订阅地址
GET  …fuqingsocksAction=ActivateSublink&Serviceid=…  200  "success"   订阅更新开关已打开
```

期间还确认了三件事：云浏览器自带的 `Captcha.setAutoSolve` 只处理独立验证码组件，
识别不到 Cloudflare 托管挑战页；2captcha 的 Turnstile 挑战模式（拦截 `turnstile.render`
取 `action`/`cData`/`chlPageData` + 执行回调）**确实能拿到 `cf_clearance`**，但 token 回填走的是
加密的 CF 编排请求（`/cdn-cgi/challenge-platform/h/b/fo/…`），纯 Rust 客户端无法复刻，
且该路径只在 Cloudflare 偶尔升级为交互式组件时才可用 —— 因此不作为产品方案。
最终实现的是「面板出口代理」：不改客户端、不引入浏览器、不引入解题服务。
