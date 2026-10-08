// Rebuild public, reviewable evidence. No credentials, private hosts or local paths
// are written to the registry. Review the generated diff before publishing it.
import { execFileSync } from 'node:child_process';
import { mkdirSync, existsSync, writeFileSync, readFileSync } from 'node:fs';
import { createHash } from 'node:crypto';
import { inflateRawSync } from 'node:zlib';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const cache = path.join(tmpdir(), 'camofy-public-client-research');
mkdirSync(cache, { recursive: true });
const git = (...args) => execFileSync('git', args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'] }).trim();
const version = s => s.replace(/^v/, '').split('.').map(Number);
const compare = (a, b) => { const x=version(a), y=version(b); for(let i=0;i<3;i++) if(x[i]!==y[i]) return x[i]-y[i]; return 0; };
const texts = new Map();
async function read(url, optional = false) {
  if(texts.has(url)) return texts.get(url);
  const saved=path.join(cache,createHash('sha256').update(url).digest('hex')+'.txt');
  // Only immutable source refs are cached across runs, never release lists/docs.
  const immutable=/raw\.githubusercontent\.com\/[^/]+\/[^/]+\/(?:v\d+\.\d+\.\d+|[a-f0-9]{40})\//.test(url);
  if(immutable && existsSync(saved)) return readFileSync(saved,'utf8');
  const pending = (async () => {
    for(let i=0;i<3;i++) {
      try {
        const r = await fetch(url, { signal: AbortSignal.timeout(25000) });
        if(r.status === 404 && optional) return '';
        if(!r.ok) throw new Error(`Public source HTTP ${r.status}`);
        const value=await r.text();
        if(immutable) writeFileSync(saved,value);
        return value;
      } catch(e) { if(i===2) throw e; }
    }
  })();
  texts.set(url, pending); return pending;
}
// Read one bounded Go module source member in memory; never extract paths from
// an archive onto disk. Go's module mirror preserves the original release source.
function zipMember(zip, suffix) {
  let end=zip.length-22;
  while(end>=Math.max(0,zip.length-65557) && zip.readUInt32LE(end)!==0x06054b50) end--;
  if(end<0) throw new Error('Missing ZIP directory');
  let at=zip.readUInt32LE(end+16);
  const count=zip.readUInt16LE(end+10);
  for(let i=0;i<count;i++) {
    if(zip.readUInt32LE(at)!==0x02014b50) throw new Error('Invalid ZIP directory');
    const method=zip.readUInt16LE(at+10), size=zip.readUInt32LE(at+20), expanded=zip.readUInt32LE(at+24);
    const nameLength=zip.readUInt16LE(at+28), extra=zip.readUInt16LE(at+30), comment=zip.readUInt16LE(at+32), offset=zip.readUInt32LE(at+42);
    const name=zip.subarray(at+46,at+46+nameLength).toString('utf8');
    at+=46+nameLength+extra+comment;
    if(!name.endsWith('/'+suffix)) continue;
    if(expanded>512*1024 || ![0,8].includes(method) || zip.readUInt32LE(offset)!==0x04034b50) throw new Error('Unexpected module member');
    const start=offset+30+zip.readUInt16LE(offset+26)+zip.readUInt16LE(offset+28);
    const content=zip.subarray(start,start+size);
    const decoded=method===8?inflateRawSync(content,{maxOutputLength:512*1024}):content;
    if(decoded.length!==expanded) throw new Error('Module source length mismatch');
    return decoded.toString('utf8');
  }
  return '';
}
async function concurrent(items, action, limit=6) {
  let index=0; const results=new Array(items.length);
  await Promise.all(Array.from({length:Math.min(limit,items.length)},async()=>{
    while(index<items.length) { const i=index++; results[i]=await action(items[i]); }
  })); return results;
}
function checkout(repo) {
  const dir=path.join(cache,repo.split('/')[1]);
  if(!existsSync(dir)) git('-c','credential.helper=','clone','--bare','--filter=blob:none','--depth=1','--no-single-branch',`https://github.com/${repo}.git`,dir);
  return dir;
}
const registry={schema_version:1,version:'2026-10-09.1',checked_at:'2026-10-09',profiles:{},clients:[],protocols:[]};
const pendingProfiles=new Map();
function coreProfile(repo, ref) {
  const id=`${repo.replaceAll('/','-').toLowerCase()}-${ref}`;
  if(pendingProfiles.has(id)) return pendingProfiles.get(id);
  const pending=(async()=>{
    const base=`https://raw.githubusercontent.com/${repo}/${ref}`;
    const parser=await read(`${base}/adapter/parser.go`, true);
    if(!parser) return null;
    const section=parser.slice(parser.indexOf('switch proxyType'));
    const protocols=[...new Set([...section.matchAll(/case\s+"([^\"]+)"\s*:/g)].map(m=>m[1]))].sort();
    if(!protocols.includes('ss') || !protocols.includes('vmess')) throw new Error('Unrecognized public parser structure');
    const features={};
    const evidence=[`https://github.com/${repo}/blob/${ref}/adapter/parser.go`];
    for(const protocol of ['mieru','vless','vmess','trojan']) {
      if(!protocols.includes(protocol)) continue;
      const src=await read(`${base}/adapter/outbound/${protocol}.go`);
      evidence.push(`https://github.com/${repo}/blob/${ref}/adapter/outbound/${protocol}.go`);
      if(protocol==='mieru') {
        for(const [key,pattern] of Object.entries({
          'mieru.udp-transport':'TransportProtocol_UDP','mieru.udp-relay':'option.UDP',
          'mieru.multiplexing':'Multiplexing','mieru.handshake-mode':'HandshakeMode',
          'mieru.traffic-pattern':'TrafficPattern',
        })) features[key]=src.includes(pattern);
      } else {
        // Network dispatch is explicit in these adapters; absent cases cannot
        // silently be upgraded to a supported transport.
        for(const transport of ['ws','grpc','h2','http','xhttp','httpupgrade']) {
          if(src.includes(`case "${transport}"`)) features[`${protocol}.${transport}`]=true;
          else if(/switch\s+\w+\.Network/.test(src)) features[`${protocol}.${transport}`]=false;
        }
        if(src.includes('Reality')) features[`${protocol}.reality`]=true;
        if(src.includes('xtls-rprx-vision')) features[`${protocol}.xtls-rprx-vision`]=true;
      }
    }
    registry.profiles[id]={name:`${repo.split('/')[1]} ${ref.slice(0,12)}`,protocols,complete_protocols:true,features,evidence};
    return id;
  })();
  pendingProfiles.set(id,pending); return pending;
}

for(const spec of [
  {id:'cmfa',name:'Clash Meta for Android',repo:'MetaCubeX/ClashMetaForAndroid',tokens:['clashmetaforandroid'],corePath:'core/src/foss/golang/clash',coreRepo:'MetaCubeX/mihomo',min:'2.8.0',notes:'官方发行版内置内核；.Meta 后缀按同一版本处理。自行编译或替换内核时可关闭自动模式并按类型排除。'},
  {id:'flclash',name:'FlClash',repo:'chen08209/FlClash',tokens:['flclash'],corePath:'core/Clash.Meta',coreRepo:'chen08209/Clash.Meta',min:'0.8.0',notes:'按官方发行标签固定的内核提交识别。自定义 User-Agent 或自定义内核无法可靠推断。'},
]) {
  const dir=checkout(spec.repo);
  const tags=git(`--git-dir=${dir}`,'tag','--list').split('\n').filter(t=>/^v\d+\.\d+\.\d+$/.test(t)&&compare(t,spec.min)>=0).sort(compare);
  const releases=await concurrent(tags,async tag=>{
    const entry=git(`--git-dir=${dir}`,'ls-tree',tag,spec.corePath);
    const sha=entry.match(/^160000 commit ([0-9a-f]{40})\s/);
    if(!sha) return null;
    const profile=await coreProfile(spec.coreRepo,sha[1]);
    return {version:tag.slice(1),profile,evidence:`https://github.com/${spec.repo}/tree/${tag}/${spec.corePath}`,confidence:'bundled'};
  });
  registry.clients.push({id:spec.id,name:spec.name,tokens:spec.tokens,format:'clash',notes:spec.notes,releases:releases.filter(Boolean),ranges:[]});
}
const tags=git('ls-remote','--tags','https://github.com/MetaCubeX/mihomo.git').split('\n')
  .map(l=>l.match(/refs\/tags\/(v\d+\.\d+\.\d+)$/)?.[1]).filter(Boolean)
  .filter(t=>compare(t,'1.15.0')>=0).sort(compare);
registry.clients.push({id:'mihomo',name:'Mihomo / Clash.Meta 内核',tokens:['mihomo','clash.meta'],format:'clash',
  notes:'只按明确的内核版本匹配。只有 clash.meta 而没有版本时保留节点并提示未识别。',ranges:[],
  releases:await concurrent(tags,async tag=>({version:tag.slice(1),profile:await coreProfile('MetaCubeX/mihomo',tag),evidence:`https://github.com/MetaCubeX/mihomo/releases/tag/${tag}`,confidence:'verified'}))});
const vergeReleases=JSON.parse(await read('https://api.github.com/repos/clash-verge-rev/clash-verge-rev/releases?per_page=100'));
const verge=[];
for(const r of vergeReleases.filter(r=>/^v\d+\.\d+\.\d+$/.test(r.tag_name))) {
  const match=r.body.match(/(?:Mihomo|mihomo|Meta|meta)[^\n]{0,45}?(1\.\d+\.\d+)/);
  if(match) verge.push({version:r.tag_name.slice(1),profile:await coreProfile('MetaCubeX/mihomo',`v${match[1]}`),evidence:r.html_url,confidence:'bundled'});
}
registry.clients.push({id:'verge',name:'Clash Verge Rev',tokens:['clash-verge','clashverge'],format:'clash',releases:verge.sort((a,b)=>compare(a.version,b.version)),ranges:[],
  notes:'UA 可自定义，内核可单独更新。仅明确公布内置版本的发行版参与自动判断；其他版本不会套用相邻版本。'});

const goProxy='https://proxy.golang.org/github.com/!dreamacro/clash/@v/';
const classicTags=(await read(goProxy+'list')).trim().split('\n').filter(t=>/^v1\.\d+\.\d+$/.test(t)).sort(compare);
const classicReleases=await concurrent(classicTags,async tag=>{
  const archive=path.join(cache,`clash-${tag}.zip`);
  let zip;
  if(existsSync(archive)) zip=readFileSync(archive);
  else { const response=await fetch(goProxy+tag+'.zip',{signal:AbortSignal.timeout(30000)}); if(!response.ok) throw new Error('Module mirror unavailable'); zip=Buffer.from(await response.arrayBuffer()); if(zip.length>8*1024*1024) throw new Error('Oversized module'); writeFileSync(archive,zip); }
  const parser=zipMember(zip,'adapter/parser.go') || zipMember(zip,'adapters/outbound/parser.go');
  const protocols=[...new Set([...parser.matchAll(/case\s+"([^\"]+)"\s*:/g)].map(m=>m[1]))].sort();
  if(!protocols.includes('ss') || !protocols.includes('vmess')) return null;
  const evidence=`https://pkg.go.dev/github.com/Dreamacro/clash@${tag}/adapter`;
  const features={};
  for(const protocol of ['vmess','trojan']) {
    const src=zipMember(zip,`adapter/outbound/${protocol}.go`) || zipMember(zip,`adapters/outbound/${protocol}.go`);
    if(!src) continue;
    for(const transport of ['ws','grpc','h2','http','xhttp','httpupgrade']) {
      if(src.includes(`case "${transport}"`)) features[`${protocol}.${transport}`]=true;
      else if(/switch\s+\w+\.Network/.test(src)) features[`${protocol}.${transport}`]=false;
    }
  }
  const id='clash-'+tag;
  registry.profiles[id]={name:`Clash ${tag}`,protocols,complete_protocols:true,features,evidence:[evidence,goProxy+tag+'.zip']};
  return {version:tag.slice(1),profile:id,evidence,confidence:'verified'};
});
registry.clients.push({id:'clash-core',name:'Clash 开源内核',tokens:['clash'],format:'clash',notes:'原始 Go 模块发行源码；仅匹配明确的开源内核版本，Premium 日期版本和第三方 fork 保持未知。',releases:classicReleases.filter(Boolean),ranges:[]});

const xrayProfiles=new Map();
async function xrayProfile(tag) {
  if(xrayProfiles.has(tag)) return xrayProfiles.get(tag);
  const promise=(async()=>{
    const base=`https://raw.githubusercontent.com/XTLS/Xray-core/${tag}/infra/conf/`;
    const source=await read(base+'xray.go',true);
    if(!source) return null;
    const section=source.match(/outboundConfigLoader\s*=\s*NewJSONConfigLoader\(ConfigCreatorCache\{([\s\S]*?)\n\s*\},/)?.[1] ?? '';
    if(!section.includes('VLessOutboundConfig')) throw new Error('Unrecognized Xray outbound registry');
    const aliases={shadowsocks:'ss',socks:'socks5',freedom:'direct',blackhole:'reject'};
    const native=[...section.matchAll(/"([a-z0-9-]+)":\s*func/g)].map(m=>m[1]);
    const protocols=native.filter(p=>p!=='hysteria').map(p=>aliases[p]??p);
    const evidence=[`https://github.com/XTLS/Xray-core/blob/${tag}/infra/conf/xray.go`];
    let complete=true;
    if(native.includes('hysteria')) {
      const hysteria=await read(base+'hysteria.go');
      if(hysteria.includes('c.Version != 2')) protocols.push('hysteria2');
      else complete=false; // Earlier experimental syntax does not prove the wire version.
      evidence.push(`https://github.com/XTLS/Xray-core/blob/${tag}/infra/conf/hysteria.go`);
    }
    const id='xray-'+tag;
    registry.profiles[id]={name:`Xray ${tag}`,protocols:[...new Set(protocols)].sort(),complete_protocols:complete,unsupported_protocols:['mieru','anytls','ssr','snell','tuic'].filter(p=>!protocols.includes(p)),features:{},evidence};
    return id;
  })();
  xrayProfiles.set(tag,promise); return promise;
}
const ngReleases=JSON.parse(await read('https://api.github.com/repos/2dust/v2rayNG/releases?per_page=100'));
const ng=await concurrent(ngReleases.filter(r=>/^v?\d+\.\d+\.\d+$/.test(r.tag_name)),async r=>{
  const tag=r.body.match(/XTLS\/Xray-core\/releases\/tag\/(v\d+\.\d+\.\d+)/)?.[1];
  if(!tag) return {version:r.tag_name.replace(/^v/,''),profile:null,evidence:r.html_url,confidence:'unknown'};
  return {version:r.tag_name.replace(/^v/,''),profile:await xrayProfile(tag),evidence:r.html_url,confidence:'bundled',prerelease:r.prerelease};
});
registry.clients.push({id:'v2rayng',name:'v2rayNG',tokens:['v2rayng'],format:'uri',notes:'按已发布的完整版本号与发行说明中明确的 Xray 内核匹配；预发行版本单独标注。自定义内核可能不同，Clash YAML 不是原生完整配置。',releases:ng.sort((a,b)=>compare(a.version,b.version)),ranges:[]});
registry.clients.push({id:'xray',name:'Xray 内核',tokens:['xray','xray-core'],format:'xray',notes:'根据官方源码的出站配置注册表核实协议；原生配置使用 Xray JSON，传输细节未确认时保留。',releases:await Promise.all([...xrayProfiles.keys()].sort(compare).map(async tag=>({version:tag.slice(1),profile:await xrayProfile(tag),evidence:`https://github.com/XTLS/Xray-core/releases/tag/${tag}`,confidence:'verified'}))),ranges:[]});

// Closed-source and multi-core clients: preserve uncertainty explicitly. A
// missing feature is not negative evidence unless the profile is exhaustive.
function profile(id,name,protocols,features,evidence,unsupported_protocols=[]) {
  registry.profiles[id]={name,protocols,features,complete_protocols:false,unsupported_protocols,evidence}; return id;
}
function client(id,name,tokens,format,notes,ranges=[]) {registry.clients.push({id,name,tokens,format,notes,releases:[],ranges});}
const stashDoc='https://stash.wiki/proxy-protocols/proxy-types';
const stashBase=['ss','ssr','socks5','http','vmess','snell','trojan','hysteria','hysteria2','vless','wireguard','tuic'];
profile('stash-3.3','Stash iOS 3.3.0–3.3.2', [...stashBase,'anytls'], {'vless.reality':false,'vless.xhttp':false,'vmess.reality':false,'trojan.reality':false,'snell.v4':false,'snell.v5':false},[stashDoc,'https://stash.wiki/release-notes/ios'],['mieru','masque','tailscale']);
profile('stash-3.3.3','Stash iOS 3.3.3', [...stashBase,'anytls'], {'vless.reality':true,'vless.xhttp':false,'vmess.reality':false,'trojan.reality':false,'snell.v4':false,'snell.v5':false},[stashDoc,'https://stash.wiki/release-notes/ios'],['mieru','masque','tailscale']);
profile('stash-3.4','Stash iOS 3.4–3.5',[...stashBase,'anytls','tailscale'],{'vless.reality':true,'vless.xhttp':false,'vmess.reality':false,'trojan.reality':false,'snell.v4':false,'snell.v5':false},[stashDoc],['mieru','masque']);
profile('stash-3.6','Stash iOS 3.6 / macOS 4.3',[...stashBase,'anytls','tailscale','mieru','masque','trusttunnel'],{'vless.xhttp':true,'vless.reality':true,'vmess.reality':true,'trojan.reality':true,'mieru.udp-transport':true},[stashDoc]);
client('stash','Stash（iOS / tvOS）',['stash'],'clash','版本规则来自官方协议文档。未识别的平台或构建号不映射为 iOS 版本。',[
  {min:'3.3.0',max:'3.3.2',profile:'stash-3.3'},{min:'3.3.3',max:'3.3.99',profile:'stash-3.3.3'},{min:'3.4.0',max:'3.5.99',profile:'stash-3.4'},{min:'3.6.0',max:'3.6.99',profile:'stash-3.6'}]);
client('stash-mac','Stash Mac',['stashmac','stash mac'],'clash','macOS 与 iOS 版本分别匹配。', [{min:'4.3.0',max:'4.3.99',profile:'stash-3.6'}]);
const sr='https://apps.apple.com/us/app/shadowrocket/id932747118';
profile('shadowrocket-2.2.86','Shadowrocket 2.2.86–2.2.88',['ss','ssr','vmess','vless','trojan','socks5','http','hysteria','hysteria2','tuic','wireguard','ssh','anytls','mieru'],{'mieru.udp-transport':true},[sr]);
profile('shadowrocket-2.2.89','Shadowrocket 2.2.89+',['ss','ssr','vmess','vless','trojan','socks5','http','hysteria','hysteria2','tuic','wireguard','ssh','anytls','mieru'],{'mieru.udp-transport':true,'mieru.traffic-pattern':true},[sr]);
client('shadowrocket','Shadowrocket',['shadowrocket'],'clash','协议能力与 Camofy URI 编码器的能力分别检查。官方历史记录对 Mieru 首次支持版本存在歧义，较早版本不猜测。',[
  {min:'2.2.86',max:'2.2.88',profile:'shadowrocket-2.2.86'},{min:'2.2.89',max:'2.2.90',profile:'shadowrocket-2.2.89'}]);
const sb='https://sing-box.sagernet.org/configuration/outbound/';
profile('singbox-1.11','sing-box 1.11',['ss','socks5','http','vmess','vless','trojan','wireguard','hysteria','hysteria2','tuic','ssh'],{},[sb],['mieru','anytls','ssr']);
profile('singbox-1.12','sing-box 1.12–1.13',['ss','socks5','http','vmess','vless','trojan','wireguard','hysteria','hysteria2','tuic','ssh','anytls'],{},[sb,'https://sing-box.sagernet.org/configuration/outbound/anytls/'],['mieru','ssr']);
client('sing-box','sing-box',['sing-box','sfi','sfa'],'sing-box','原生配置使用 JSON；协议支持不代表可以直接加载 Clash YAML。',[
  {min:'1.11.0',max:'1.11.99',profile:'singbox-1.11'},{min:'1.12.0',max:'1.13.16',profile:'singbox-1.12'}]);
const surge='https://manual.nssurge.com/policies/overview.html';
profile('surge-current','Surge（官方协议目录）',['http','socks5','ss','snell','vmess','trojan','tuic','hysteria2','anytls','ssh','wireguard','tailscale'],{},[surge],['mieru','vless','ssr']);
client('surge','Surge',['surge'],'surge','原生配置采用 Surge 格式。iOS 与 Mac 版本和构建号不能互换；须指定经核实的完整版本再启用自动过滤。');
for(const [id,name,tokens,bands] of [
  ['surge-ios','Surge iOS',['surge ios','surge-ios'],[['5.0.0','5.7.99',[]],['5.8.0','5.16.99',['hysteria2']],['5.17.0','5.19.99',['hysteria2','anytls']],['5.20.0','5.21.99',['hysteria2','anytls','tailscale']],['5.22.0','5.23.99',['hysteria2','anytls','tailscale','masque']]]],
  ['surge-mac','Surge Mac',['surge mac','surgemac'],[['5.0.0','5.3.99',[]],['5.4.0','6.4.2',['hysteria2']],['6.4.3','6.6.99',['hysteria2','anytls']],['6.7.0','6.8.99',['hysteria2','anytls','tailscale']],['6.9.0','6.10.99',['hysteria2','anytls','tailscale','masque']]]],
]) {
  const ranges=bands.map(([min,max,added])=>{
    const pid=`${id}-${min}`;
    profile(pid,`${name} ${min}–${max}`,['http','socks5','ss','snell','vmess','trojan',...added],{},[surge],['mieru','vless','ssr',...['hysteria2','anytls','tailscale','masque'].filter(p=>!added.includes(p))]);
    return {min,max,profile:pid};
  });
  client(id,name,tokens,'surge','平台和版本必须同时确认；协议最低版本来自官方目录，未列出的功能保持未知。',ranges);
}
client('loon','Loon',['loon'],'loon','官方示例使用 Loon 配置语法；历史版本协议边界尚未核实，自动模式保留未知能力。');
client('quantumult-x','Quantumult X',['quantumult x','quantumult%20x','quantumult-x'],'quantumult-x','原生配置格式独立；不能把自定义资源解析器的能力当作内核能力。');
client('v2rayn','v2rayN',['v2rayn'],'uri','可切换 Xray、sing-box 等内核，单独的应用 UA 不足以决定协议能力。');
client('hiddify','Hiddify',['hiddify'],'uri','分支内核与上游 sing-box 能力可能不同；未提供内核信息时不套用上游能力。');
client('nekobox','NekoBox / NekoRay',['nekobox','nekoray'],'uri','应用、内核和构建选项共同决定能力，缺少已核实映射时标记未知。');
client('clashx-meta','ClashX Meta',['clashx meta','clashx.meta'],'clash','只有内核名称不足以确定版本，优先采用 UA 中明确的 Mihomo 版本。');
client('clash-legacy','Clash for Windows / Clash for Android / ClashX',['clashforandroid','clashforwindows','clashx'],'clash','不能把所有 Clash 都当作 Mihomo。应用版本不等于内核版本，缺少对应关系时不推测协议边界。');
registry.references=[
  'https://github.com/MetaCubeX/ClashMetaForAndroid/blob/v2.10.2/core/src/main/golang/native/config/fetch.go',
  'https://github.com/chen08209/FlClash/blob/main/lib/common/package.dart',
  'https://www.clashverge.dev/privacy.html',
  'https://github.com/2dust/v2rayN/wiki/List-of-supported-cores',
  'https://github.com/XTLS/Xray-core/blob/main/infra/conf/xray.go',
  'https://github.com/crossutility/Quantumult-X/blob/master/sample.conf',
  'https://github.com/Loon0x00/LoonExampleConfig/blob/master/example.conf',
];
registry.profiles=Object.fromEntries(Object.entries(registry.profiles).sort(([a],[b])=>a.localeCompare(b)));
registry.protocols=[...new Set(Object.values(registry.profiles).flatMap(p=>[...p.protocols,...p.unsupported_protocols??[]]))].sort();
mkdirSync(path.join(root,'src','compatibility'),{recursive:true});
writeFileSync(path.join(root,'src','compatibility','registry.json'),JSON.stringify(registry,null,2)+'\n');
console.log(JSON.stringify({clients:registry.clients.length,releases:registry.clients.reduce((n,c)=>n+c.releases.length,0),profiles:Object.keys(registry.profiles).length,protocols:registry.protocols.length}));
