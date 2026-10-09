# 完整配置兼容契约

配置能力保存在 `src/compatibility/config-registry.json`，由 `src/config_compatibility.rs` 查询。它与节点协议矩阵独立维护：节点能连接，不代表规则、DNS 或远程资源能被同一个客户端正确解析。

此表是有证据范围的契约，不是“所有客户端所有版本均已实测”的承诺。当前记录 21 个可识别家族、65 个能力键、22 个配置 profile；10 个家族可使用已实现的 Clash YAML 完整输出，其余 11 个家族明确报告尚未实现完整 renderer。识别家族和具备完整输出能力是两个不同结果。

## 三态和输出决策

| 状态 | 含义 | 调用方行为 |
| --- | --- | --- |
| `supported` | 所列来源支持此字段及证据范围 | 可按该契约处理；仍须校验输入值和资源内容 |
| `unsupported` | 有明确版本边界、平台限制或解析器拒绝分支 | 不向该版本声称支持；无法无损转换时说明具体问题 |
| `unknown` | 未知版本、范围外版本、未研究字段或不完整公开资料 | 保留原字段并显示未验证诊断；不得自动改写为“不支持” |

`full_configuration()` 只选择已实现的完整 renderer，不判断整份配置的每个字段，也没有仅节点回退。未识别家族、缺少 renderer、没有已知输入语法、或输入语法明确不支持时，返回 `blocked: true` 及原因。某个字段为未知不会由本模块改写、删除或阻止整份配置。

客户端版本无法从 User-Agent 确定时，不采用“最新版本 profile”。例如 `Shadowrocket/3131` 的单整数可能是构建号，不当作 `3.1.3.1` 或某个营销版本。只有明确的家族输入语法基线可以继续选择完整输出，同时产生版本未知提示；高级功能保持未知。未知家族也不会借用 Mihomo 的完整能力。

已知版本优先查有界版本范围，再查明确的家族基线。未来版本不会因为版本号较大而继承最后一个功能 profile。多个版本条目可以补充不同能力，但测试禁止在重叠范围中重复声明同一能力，避免顺序影响结论。

## 能力维度

| 键前缀 | 说明 |
| --- | --- |
| `syntax.*` | 完整配置输入语法，例如 Clash YAML、sing-box JSON |
| `native.rule.*` | 客户端原生规则语法 |
| `clash.rule.*` | 客户端接受 Clash YAML 时的规则能力 |
| `clash.dns.*` | Clash DNS 字段；与流量路由独立 |
| `clash.provider.*` | provider 类型及 behavior/format 组合 |
| `clash.proxy_groups` | 策略组输入结构；不保证所有组参数均支持 |
| `clash.group.*` | select、url-test、fallback、load-balance、relay、direct 各类型独立能力 |

`native.rule.and` 和 `clash.rule.and` 是不同能力。某客户端原生配置支持 AND，并不能证明它的 Clash 导入器能正确转换同名规则。`domain_mrs`、`ipcidr_mrs`、`classical_mrs` 同样分别记录；“支持 MRS”不能扩张成支持全部 behavior。

策略组结构支持不等于某个组类型仍存在。例如 Mihomo 1.19.17 的[解析器](https://github.com/MetaCubeX/mihomo/blob/a001b1b11008ba469442e5e59b058dd032263470/adapter/outboundgroup/parser.go)已明确拒绝 `relay`，而 Clash 开源 1.18.0 支持它；两者都不把 `direct` 当作策略组类型。内置 `DIRECT` 策略作为成员是另一回事，不能被这项负面结论误伤。Shadowrocket 的逐类型 Clash 导入契约仍保持未知。

`capability(detection, key)` 返回 `support`、`basis`、`evidence` 和 `notes`。`basis` 为 `version_range`、`family_baseline` 或 `unknown`。缺失键一律返回未知。`registry()` 可直接序列化给能力矩阵接口；字段支持和产品 renderer 状态分别展示。

## 当前家族覆盖

| 家族 | 完整输出入口 | 当前证据范围 |
| --- | --- | --- |
| Mihomo / Clash.Meta 内核 | Clash YAML | 基础规则及 DNS 检查 1.15.0–1.19.17；扩展字段另有精确快照 |
| Clash 开源内核 | Clash YAML | 配置字段检查 1.18.0 精确版本；不冒充 Premium |
| Clash Meta for Android | Clash YAML | 家族输入语法；应用版本不当作内核版本 |
| FlClash | Clash YAML | 家族输入语法；独立字段未知 |
| Clash Verge Rev | Clash YAML | 家族输入语法；独立字段未知 |
| ClashX Meta | Clash YAML | 家族输入语法；独立字段未知 |
| Clash for Windows / Clash for Android / ClashX | Clash YAML | 家族输入语法；开源/Premium 等内核差异不推定 |
| Stash iOS / tvOS | Clash YAML | MRS 自 3.1.0；当前规则及 inline provider 检查 3.6.0 |
| Stash Mac | Clash YAML | 当前规则及 provider 检查 4.3.0；独立于 iOS 版 |
| Shadowrocket | Clash YAML 导入 | 导入语法入口 2.1.60–2.2.92；逐字段导入能力另列未知 |
| sing-box | 未实现 | 已记录原生 JSON 输入；不生成伪完整配置 |
| Xray | 未实现 | 已记录原生 JSON 输入 |
| Surge / Surge iOS / Surge Mac | 未实现 | 3 个识别家族，已记录原生语法 |
| Loon | 未实现 | 已记录原生语法 |
| Quantumult X | 未实现 | 已记录开发者原生配置示例 |
| v2rayNG / v2rayN / Hiddify / NekoBox | 未实现 | 4 个识别家族，尚无完整转换契约 |

这些条目不复制节点矩阵中的数百个发布 profile。包装应用需要明确的内核关联及配置证据后，才能增加字段能力；不能仅从应用名或应用版本推导。

## Shadowrocket 的边界

[官方发布频道 2.1.60（1050）](https://t.me/ShadowrocketNews/318) 已在 2020-08-23 公布 Clash YAML 导入；[2.1.95](https://t.me/ShadowrocketNews/362) 于 2021-12-19 再次公布同项能力，并包含 DST-PORT 修复。因此不能把 2.1.95 当作最早引入边界，也不能将 2.1.60–2.1.94 全部判为不支持。

矩阵只为 2.1.60–2.2.92 确认 YAML 导入语法入口，规则、DNS、策略组与 provider 的逐字段导入能力仍独立评估。2.1.59 及更早版本缺少已核实的输入语法契约，标记 `unknown`，完整输出入口按现有语法门槛阻止选择；这不是客户端“不支持”的结论。未知版本保留家族入口及明确提示。后续 [App Store 发布记录](https://apps.apple.com/us/app/shadowrocket/id932747118) 仍有 Clash 解析修复，但没有公开整个 YAML 字段的映射契约。

原生规则的 DOMAIN、DOMAIN-SUFFIX、DOMAIN-KEYWORD、IP-CIDR、GEOIP、DST-PORT、RULE-SET、FINAL 及逻辑规则，可由[规则项目原作者配置说明](https://github.com/GMOogway/shadowrocket-rules/blob/68f92ea9aed0f119649813d9d60d69f5139ffe7b/docs/01.shadowrocket_configure.md)与[维护手册快照](https://github.com/LOWERTOP/Shadowrocket/blob/3537f928451038ba74258eccbb6d9bd7865628dd/README.md)核对。它们是原作者/维护者资料，不标成 Shadowrocket 开发者正式规范。

- `PROCESS-NAME`：目前缺少 Shadowrocket 平台及版本范围的一手契约，记录未知。不能因为另一个 iOS 客户端不支持就自动删除。
- `DST-PORT`：原生支持有发布记录；Clash 导入的表达式边界、端口范围语法仍不假定全部支持。
- `AND` / `OR` / `NOT`：官方发布记录证明逻辑规则存在，原作者配置有语法示例；不能用错误的社区表断言“不支持逻辑规则”。Clash 导入契约仍独立。
- `DOMAIN-REGEX`：未找到明确的原生/Clash 导入支持契约。保持未知，绝不换成 `URL-REGEX`，两者匹配的输入对象不同。
- DNS：`nameserver-policy`、`fake-ip-filter`、`fallback-filter` 等 Clash 字段的完整解析范围未公开。不能把 DNS 策略移入 `[Rule]`，也不能以“配置导入成功”证明 DNS 策略实际等价。

因此 Shadowrocket 完整输出包含未知诊断时，不应宣传“全部规则和 DNS 行为已验证等价”。实际验证应分别检查规则命中、DNS 查询、策略组引用及远程资源更新。

## 规则资源转换必须保留的语义

以下是转换器的约束，并非所有约束均已实现；字段为未知不表示可以安全省略。

1. 按原规则位置展开资源，保留动作和策略组引用。相同 domain 出现在不同位置或不同策略下不能全局去重。逻辑表达式需要 AST；不能按逗号简单切分或把 AND/NOT 展开成多个顶层规则。
2. `GEOSITE` 使用配置指定的实际数据源。MetaCubeX 的 `cn`、`onedrive` 等集合经过定制，不能换成名称相同的其他作者规则集。记录资源 URL、内容哈希及版本。
3. GeoSite `Full`、`Domain`、`Plain` 分别表达完整域名、域名后缀及关键词。`Regex` 按域名匹配，不能当 URL 正则。`@a@b` 过滤要求同时具备属性；只在开头的 `!` 表示反转，`geolocation-!cn` 是合法类别名。
4. `GEOSITE,!set` 在源内核中仍要求非空域名。直接转为 `NOT(set)` 会错误匹配没有域名的纯 IP 请求。[Mihomo GEOSITE 源码](https://github.com/MetaCubeX/mihomo/blob/a001b1b11008ba469442e5e59b058dd032263470/rules/common/geosite.go)、[类别及属性解析](https://github.com/MetaCubeX/mihomo/blob/a001b1b11008ba469442e5e59b058dd032263470/component/geodata/utils.go)给出这些边界。
5. `GEOIP` 同名国家在两个不同数据库中并不一定是同一 CIDR 集合。无损转换需要使用源数据库、保持 IPv4/IPv6 和反转语义；保留目标客户端自带 `GEOIP` 只能称为原生执行，不能声称数据库完全相同。
6. `no-resolve` 在源 Mihomo 中是禁止此规则主动发起 DNS 查询，不会清除之前已得到的目标 IP。[IP-CIDR 匹配器](https://github.com/MetaCubeX/mihomo/blob/a001b1b11008ba469442e5e59b058dd032263470/rules/common/ipcidr.go)和 [RULE-SET](https://github.com/MetaCubeX/mihomo/blob/a001b1b11008ba469442e5e59b058dd032263470/rules/provider/rule_set.go)对此有明确实现。Shadowrocket 中“前面已触发 DNS”这类边界需实机验证。
7. provider 的 `domain` 内容不是全部后缀匹配：`example.com`、`+.example.com`、`.example.com`、`*.example.com` 分别有不同的完整域名/层级语义。[源内核测试](https://github.com/MetaCubeX/mihomo/blob/a001b1b11008ba469442e5e59b058dd032263470/component/trie/domain_test.go)覆盖这些差异。

特别注意：MetaCubeX 的 `geo/geosite/<name>.list`、同目录 YAML 和 MRS **只含完整域名及后缀**；`geo/geosite/classical/<name>.list` 才包含 Keyword/Regex。官方[转换器源码快照](https://github.com/MetaCubeX/meta-rules-converter/blob/7dea27841a3579a633189830c98c08a0434e8b79/input/geosite.go)分别构造 domain 与 classical 两份列表，不能将普通列表当作完整 DAT 的等价替代。例如已查数据快照 `ad2798bba7340c09298f364bebedebbfa4398f5b` 中，`private` 有 1 条域名正则，`google` 有 2 条，`geolocation-!cn` 有 151 条。

| 源资源 | 可采用的转换入口 | 不可省略的检查 |
| --- | --- | --- |
| `classical` + YAML | 读取 `payload` 并逐条解析 | 规则类型、参数、逻辑 AST、策略绑定 |
| `classical` + text | 解析每个有效行 | 不能把不认识的行当注释略过 |
| `domain` + YAML/text | 解析域名模式 | 完整域名、后缀、仅子域、单层通配符不能混同 |
| `ipcidr` + YAML/text | 校验 IPv4/IPv6 前缀 | 保留 no-resolve/来源方向及边界 |
| `domain` / `ipcidr` + MRS | 官方 MRSv1 解码或有界转换器 | behavior/header、解压大小、条目数、前缀/域名结构 |
| `classical` + MRS | 当前格式没有此组合 | 不能伪装成 domain MRS |
| 本地 `file` provider | 显式导入的源资源 | 不能读取云服务器上同名任意路径 |

输出远程 RULE-SET URL 前必须确认内容已是目标可用语法，而非仅改 URL 后缀。使用自己生成的规则资源时需保留来源、版本和鉴权边界。内联过大或资源不可读取时报告具体阻碍，不能截断、跳过或改成仅节点输出。

## 默认 CN 规则的专用镜像

按用户明确指定的输出策略，Shadowrocket 完整 YAML 可将默认 GeoSite 数据源中顶层、正向、无属性的原子 `GEOSITE,cn,<策略>` 引用改为 HTTP `classical` / `text` provider。只有实际来源与固定内容哈希匹配默认快照时才应用此特例；保留引用位置、原策略及 `no-resolve`。复杂逻辑、DNS 中的 GeoSite 引用、属性过滤、反向匹配和自定义来源继续使用现有内联转换路径，不扩大镜像条件。

公开资源路径为 `/api/rules/geosite/<固定来源版本>/cn.list`，由 cloud 程序内嵌的固定 gzip 资源提供 MetaCubeX 官方 classical 原始文本；运行时不依赖上游网络。该独立入口不是任意 URL 代理，不需要身份 token，也不需要数据库迁移；资源仅包含公开规则，不携带身份、节点或订阅凭据。更新 CN 数据时生成新的版本路径，保留已发布配置引用的历史路径。不能换成第三方同名 China 规则，也不能用 GEOIP 替代域名集合。

这项转换策略不构成新的客户端能力证据：`clash.provider.http`、`clash.provider.classical_text` 和 `clash.rule.rule_set` 对 Shadowrocket 仍保持 `unknown` 并显示诊断。[2.2.34（1977）](https://t.me/ShadowrocketNews/427) 的 Clash text ruleset 解析修复，只能证明存在相应解析路径，不能证明所有 YAML provider 参数或导出后的外链保留行为。[2.2.81（3232）](https://t.me/ShadowrocketNews/1468) 修复了资源下载的 HTTP 304 处理，[2.2.91（3381）](https://t.me/ShadowrocketNews/1578) 修复了远程规则刷新可靠性；这些记录不保证首次下载失败会阻止启用，也不保证刷新失败必然保留旧缓存。

服务端测试可验证镜像字节、哈希、引用位置和转换边界；客户端验收仍须检查首次下载失败、已有缓存后刷新失败、304、规则命中，以及导入、编译、重启、再次导出后的外链是否保留。外置资源减小主配置文本，不等于客户端不再编译这些规则或运行内存同比下降。

## 维护与验证

新增条目必须提供 HTTPS 证据、明确说明和能力维度。负面结论只来自明确拒绝、明确平台限制或明确引入版本；缺少搜索结果不构成不支持。GitHub 源码优先固定提交或版本，动态文档记录检查日期和已核实版本上限。

当前合成测试覆盖：21 家族与节点识别表一致；Shadowrocket 2.1.59 保持未知且不节点降级，2.1.60/2.1.94/2.1.95/2.2.92 仅确认语法入口且全部 provider 能力保持未知；未知构建号、预发布及未来版本不继承最新 profile；原生/Clash 导入隔离；Stash MRS 引入边界及 iOS/macOS 进程差异；缺 renderer 不节点降级；矩阵键、引用、证据和重叠版本范围完整性。以上是服务端契约测试，不能替代客户端实机验证。
