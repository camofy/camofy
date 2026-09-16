// End-to-end smoke test against an explicitly supplied TEST ACCOUNT. Creates temporary
// Profiles/identities, downloads their subscriptions, then removes only those resources.
// Never binds devices, refreshes airports or changes existing user configurations.
// Required env: CAMOFY_TEST_URL, CAMOFY_TEST_EMAIL, CAMOFY_TEST_PASSWORD,
// MIHOMO_TEST_BINARY, CAMOFY_TEST_OUTPUT. Optional CAMOFY_TEST_ORIGIN for an SSH tunnel.
import assert from 'node:assert/strict';
import { mkdirSync, writeFileSync } from 'node:fs';
import { join, resolve } from 'node:path';
import { spawnSync } from 'node:child_process';
import { randomUUID } from 'node:crypto';
const env=process.env;
for (const k of ['CAMOFY_TEST_URL','CAMOFY_TEST_EMAIL','CAMOFY_TEST_PASSWORD','MIHOMO_TEST_BINARY','CAMOFY_TEST_OUTPUT']) assert(env[k],`${k} required`);
const base=env.CAMOFY_TEST_URL.replace(/\/$/,''), origin=env.CAMOFY_TEST_ORIGIN??base;
const output=resolve(env.CAMOFY_TEST_OUTPUT);mkdirSync(output,{recursive:true});
let cookie='';const resources=[];
async function request(path, method='GET', body) {
  const r=await fetch(`${base}/api${path}`,{method,headers:{cookie,origin,'content-type':'application/json'},body:body===undefined?undefined:JSON.stringify(body)});
  if (!r.ok) throw new Error(`${method} ${path}: ${r.status} ${await r.text()}`);
  if (path==='/auth/login') cookie=r.headers.get('set-cookie').split(';')[0];
  return r.status===204?null:r.json();
}
async function create(kind,data) {const r=await request('/resources','POST',{kind,data});resources.push(r.id);return r;}
const expected={'douyin-direct':['douyin.com','douyinec.com','douyinpay.com'],'bilibili-direct':['bilibili.com','acgvideo.com'],'steam-cn-download':['dl.steam.clngaa.com'],'telegram-routing':['telegram.org','t.me'],'openai-routing':['openai.com','chatgpt.com']};
try {
  const me=await request('/auth/login','POST',{email:env.CAMOFY_TEST_EMAIL,password:env.CAMOFY_TEST_PASSWORD});assert.equal(me.email,env.CAMOFY_TEST_EMAIL);
  const baseProfile=await create('profile',{name:'Catalog smoke base',type:'overlay',content:"tun: {enable: false}\nmixed-port: 0\nproxies: [{name: smoke, type: ss, server: example.com, port: 443, cipher: aes-256-gcm, password: fixture-only}]\nproxy-groups: [{name: SmokeRoute, type: select, proxies: [smoke, DIRECT]}]\nrules: ['MATCH,SmokeRoute']\n"});
  const report=[];
  for(const [slug,domains] of Object.entries(expected)) {
    const detail=await request(`/store/packages/${slug}`),v=detail.versions[0];
    assert.equal(detail.publisher,'Camofy');assert(!JSON.stringify(detail).includes('@camofy.app'),'login email leaked in catalog');
    const id=randomUUID();await request('/store/install','POST',{version_id:v.id,profile_id:id});resources.push(id);
    const policy=v.manifest.default_policy??'SmokeRoute';
    const bundle=await create('bundle',{name:`Catalog smoke ${slug}`,profiles:[{profile_id:baseProfile.id,enabled:true},{profile_id:id,enabled:true,parameters:{policy}}],selections:{}});
    assert(!bundle.data.error,bundle.data.error);
    const url=new URL(bundle.data.subscription_url);
    const r=await fetch(base+url.pathname);assert.equal(r.status,200);
    const yaml=await r.text();
    for (const d of domains) assert(yaml.includes(`${d},${policy}`),`Missing ${d} -> ${policy}`);
    assert(yaml.includes('Copyright (c) 2018-2019 V2Ray'),'license missing from delivered configuration');
    assert(yaml.includes(`DOMAIN,${new URL(origin).hostname},DIRECT`),'cloud safety missing');
    assert(yaml.includes('enable: false'),'TUN disabled fixture missing');
    if(slug==='steam-cn-download') assert(!yaml.includes('DOMAIN-SUFFIX,steamcommunity.com,DIRECT'));
    if(slug==='bilibili-direct') assert(!yaml.includes('DOMAIN-SUFFIX,bilibili.tv,DIRECT'));
    const config=join(output,`${slug}.yaml`);writeFileSync(config,yaml);
    const core=spawnSync(env.MIHOMO_TEST_BINARY,['-t','-d',output,'-f',config],{encoding:'utf8',timeout:30000,windowsHide:true});
    writeFileSync(join(output,`${slug}.validation.log`),`${core.stdout??''}\n${core.stderr??''}`);
    assert.equal(core.status,0,`Mihomo rejected ${slug}: ${core.stdout} ${core.stderr}`);
    const sr=await fetch(base+url.pathname+'/shadowrocket');assert.equal(sr.status,200);const full=await sr.text();assert(full.includes('rules:'));assert(full.includes('proxy-groups:'));assert(full.includes(domains[0]));
    const nodes=await fetch(base+url.pathname+'/shadowrocket-nodes');assert.equal(nodes.status,200);assert(Buffer.from(await nodes.text(),'base64').toString().includes('ss://')); 
    const conditional=await fetch(base+url.pathname,{headers:{'if-none-match':r.headers.get('etag')}});assert.equal(conditional.status,304);
    report.push({slug,version:v.version,rules:v.rules.length,mihomo:'passed',shadowrocket:'export-checked',subscription:'200 / 304'});console.log(`${slug}: subscription, rule coverage, Mihomo -t, Shadowrocket exports passed`);
  }
  writeFileSync(join(output,'report.json'),JSON.stringify(report,null,2));
} finally {
  for (const id of resources.reverse()) await request(`/resources/${id}`,'DELETE');
  if(cookie) await request('/auth/logout','POST',{});
}
