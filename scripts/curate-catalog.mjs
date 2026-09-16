// Reproducible curation; no credentials or publishing side effects.
// node scripts/curate-catalog.mjs <output-directory>
// Requires gh authentication for GitHub reads. Publication is a separate reviewed operation.
import { execFileSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { mkdirSync, writeFileSync } from 'node:fs';
import { resolve, join } from 'node:path';
const output = process.argv[2];
if (!output) throw new Error('An explicit output directory is required');
const revision = '6fe5416797ec88d29a3a1c69ce1431d73b955e88';
const repo = 'v2fly/domain-list-community';
const cache = new Map();
function file(path) {
  if (!cache.has(path)) {
    const json = JSON.parse(execFileSync('gh', ['api', `repos/${repo}/contents/${path}?ref=${revision}`], {encoding:'utf8',timeout:60000}));
    cache.set(path, Buffer.from(json.content, 'base64'));
  }
  return cache.get(path);
}
const license = file('LICENSE').toString('utf8');
if (!license.startsWith('MIT License') || !license.includes('Copyright')) throw new Error('Unexpected license');
const steamDownloads = new Set(['client-update.queniuqe.com','dl.steam.clngaa.com','st.dl.bscstorage.net','st.dl.eccdnx.com','st.dl.pinyuncloud.com','steampowered.com.8686c.com','steamstatic.com.8686c.com','full:alibaba.cdn.steampipe.steamcontent.com','full:lv.queniujq.cn','full:xz.pphimalayanrt.com','gstore.val.manlaxy.com']);
const specs = [
  {slug:'douyin-direct',file:'douyin',name:'抖音与商城直连',category:'国内服务',policy:'DIRECT',summary:'为抖音视频、图片、商城与支付域名提供直连规则。',notes:'按上游抖音分类收录，包含火山、汽水音乐等关联服务。只调整域名访问路径，不改 DNS、TUN 或端口；不能保证覆盖共享 CDN、App 内硬编码 IP 或解决运营商拥塞。'},
  {slug:'bilibili-direct',file:'bilibili',name:'B 站国内服务直连',category:'国内服务',policy:'DIRECT',summary:'B 站国内站点及其专用 CDN 走直连，保留国际版的原策略。',notes:'展开 bilibili-cdn，排除 bilibili-game 引用和标记 @!cn 的国际域名。含上游分类中的漫画、猫耳等关联站点。不是地区解锁组件。'},
  {slug:'steam-cn-download',file:'steam',name:'Steam 国内下载直连',category:'下载加速',policy:'DIRECT',summary:'只匹配精选国内下载与更新 CDN，不把整个 Steam 社区强制直连。',notes:'从上游 @cn 条目中精选 11 条下载/更新域名；不收录 steamcontent.com 整体、Steam 商店、社区或游戏联机地址。客户端实际选中这些 CDN 时才生效。'},
  {slug:'telegram-routing',file:'telegram',name:'Telegram 域名分流',category:'通讯',policy:null,summary:'把 Telegram 相关域名交给你在身份内选择的策略组。',notes:'仅覆盖上游 Telegram 域名分类及其关联服务；不包含 Telegram 原生客户端的 IP 网段和通话直连 IP，不保证覆盖全部 App 流量。必须指定已有策略组或节点，未指定则拒绝发布。'},
  {slug:'openai-routing',file:'openai',name:'OpenAI 常用域名分流',category:'AI 服务',policy:null,summary:'为 ChatGPT、OpenAI API、Sora 及固定 CDN 域名设置独立出口。',notes:'收录固定域名及语音服务域名；明确排除上游 @ads 遥测条目和动态 Azure WebPubSub 正则域名（首版不支持正则规则），因此不是全流量保证。必须指定出口；规则不保证地区支持或账号可用性。'},
];
mkdirSync(resolve(output), {recursive:true});
for (const s of specs) {
  const sources = new Map(), rules = new Map(), visiting = new Set();
  function collect(name) {
    if (visiting.has(name)) throw new Error('Include cycle');
    visiting.add(name);
    const path=`data/${name}`, bytes=file(path);
    sources.set(path,{url:`https://github.com/${repo}/blob/${revision}/${path}`,revision,sha256:createHash('sha256').update(bytes).digest('hex'),license:'MIT',license_text:license,attribution:'V2Ray / v2fly/domain-list-community contributors; curated and converted by Camofy'});
    for (const raw of bytes.toString('utf8').split(/\r?\n/)) {
      const line=raw.split('#')[0].trim();if (!line) continue;
      const [token,...attrs]=line.split(/\s+/);
      if (s.slug==='steam-cn-download' && !steamDownloads.has(token)) continue;
      if (s.slug==='bilibili-direct' && (attrs.includes('@!cn')||token==='include:bilibili-game')) continue;
      if (s.slug==='openai-routing' && (attrs.includes('@ads')||token==='regexp:^chatgpt-async-webps-prod-\\S+-\\d+\\.webpubsub\\.azure\\.com$')) continue;
      if (token.startsWith('include:')) {collect(token.slice(8));continue;}
      const kind=token.startsWith('full:')?'DOMAIN':'DOMAIN-SUFFIX', value=token.replace(/^(full|domain):/,'');
      if (!/^[a-z0-9.-]+\.[a-z0-9-]+$/.test(value)) throw new Error(`Unsupported rule (review explicitly): ${line}`);
      rules.set(`${kind},${value}`,{kind,value,no_resolve:false});
    }
    visiting.delete(name);
  }
  collect(s.file);
  if (!rules.size || (s.slug==='steam-cn-download'&&rules.size!==11)) throw new Error('Unexpected curation result');
  const pkg={slug:s.slug,version:'1.0.0',manifest:{name:s.name,summary:s.summary,category:s.category,notes:s.notes,default_policy:s.policy,sources:[...sources.values()]},rules:[...rules.values()]};
  writeFileSync(join(resolve(output),`${s.slug}.json`),JSON.stringify(pkg,null,2)+'\n');
  console.log(`${s.slug}: ${rules.size} rules, ${sources.size} immutable source files`);
}
