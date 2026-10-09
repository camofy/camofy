//! Tenant-scoped, encrypted and immutable rule dependency snapshots.
//! Network work occurs outside identity publication transactions. Database leases
//! coalesce concurrent cold requests; no caller holds a transaction during fetch.
use crate::App;
use anyhow::{Result, ensure};
use camofy::config_compat::{
    self, DependencyPlan, ResolvedSource, ResolvedSources, SourceContent, SourceFormat, SourceKind,
    SourceRequest, Target,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{sync::OnceLock, time::Duration};
use tokio::sync::Semaphore;
use uuid::Uuid;

pub const COMPILER_VERSION: &str = "rules-v1";
/// Full classical exports retain exact, suffix, keyword and regex predicates.
/// Never substitute the domain-only files from the parent directory.
pub const GEOSITE_REVISION: &str = "ad2798bba7340c09298f364bebedebbfa4398f5b";
const MAX_SOURCES: usize = 32;
const MAX_SOURCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOTAL_BYTES: usize = 24 * 1024 * 1024;
const TOTAL_TIMEOUT: Duration = Duration::from_secs(50);

#[derive(Clone, Copy)]
pub struct Scope {
    pub user: Uuid,
    pub bundle: Uuid,
    /// None represents an authenticated draft preview, keyed by its full input.
    pub revision: Option<Uuid>,
}

#[derive(Serialize, Deserialize)]
pub struct ResourceSnapshot {
    pub sources: ResolvedSources,
    pub fingerprint: String,
    pub compiler_version: String,
    pub geosite_revision: String,
    pub source_count: usize,
    pub total_bytes: usize,
}

fn target_id(target: Target) -> &'static str {
    match target {
        Target::Shadowrocket => "shadowrocket",
    }
}

fn geosite_url(tag: &str) -> Result<url::Url> {
    ensure!(
        !tag.is_empty()
            && tag.len() <= 180
            && !tag.starts_with('!')
            && tag.bytes().all(|b| b.is_ascii_lowercase()
                || b.is_ascii_digit()
                || matches!(b, b'-' | b'_' | b'.' | b'!' | b'@'))
            && !tag.contains(".."),
        "GeoSite 分类或属性标识无法解析；不会替换为其他数据源"
    );
    let mut url = url::Url::parse(&format!(
        "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/{GEOSITE_REVISION}/geo/geosite/classical/"
    ))?;
    url.path_segments_mut()
        .unwrap()
        .pop_if_empty()
        .push(&format!("{tag}.list"));
    Ok(url)
}

fn source_url(source: &SourceRequest) -> Result<url::Url> {
    if matches!(source.kind, SourceKind::Geosite) && source.url.is_none() {
        return geosite_url(source.geosite_tag.as_deref().unwrap_or(""));
    }
    ensure!(
        !matches!(source.format, SourceFormat::Dat),
        "自定义 GeoSite DAT 暂未提供经过验证的解码器，完整配置转换已阻止；不会使用默认库替代"
    );
    let url = source
        .url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("规则资源缺少远程地址"))?;
    let url = url::Url::parse(url).map_err(|_| anyhow::anyhow!("规则资源地址无效"))?;
    ensure!(
        ["http", "https"].contains(&url.scheme())
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.fragment().is_none(),
        "规则资源只接受不含用户名密码或片段的 HTTP(S) 地址"
    );
    Ok(url)
}

fn validate_plan(config: &serde_yaml::Value, plan: &DependencyPlan) -> Result<()> {
    ensure!(
        plan.sources.len() <= MAX_SOURCES,
        "完整配置最多解析 32 项规则资源"
    );
    let mut keys = std::collections::BTreeSet::new();
    for source in &plan.sources {
        ensure!(keys.insert(&source.key), "规则依赖标识重复");
        source_url(source)?;
        if matches!(source.kind, SourceKind::RuleProvider) {
            let name = source
                .key
                .strip_prefix("provider:")
                .ok_or_else(|| anyhow::anyhow!("规则集合引用无效"))?;
            let provider = config
                .get("rule-providers")
                .and_then(|providers| providers.get(name))
                .ok_or_else(|| anyhow::anyhow!("规则集合定义不存在"))?;
            ensure!(
                provider.get("type").and_then(serde_yaml::Value::as_str) == Some("http"),
                "完整配置无法读取客户端本地规则文件，请先改为可访问的远程或内联规则集合"
            );
            ensure!(
                provider
                    .get("proxy")
                    .is_none_or(|value| value.is_null() || value.as_str() == Some("")),
                "规则集合指定了客户端下载代理，云端无法等价执行；请提供明确的可访问规则源"
            );
            for field in ["header", "headers"] {
                ensure!(
                    provider.get(field).is_none_or(|value| value.is_null()
                        || value
                            .as_mapping()
                            .is_some_and(serde_yaml::Mapping::is_empty)),
                    "规则集合包含自定义请求头，当前完整配置解析未启用该请求方式"
                );
            }
        }
    }
    Ok(())
}

fn cache_key(
    scope: Scope,
    target: Target,
    config: &serde_yaml::Value,
    plan: &DependencyPlan,
) -> Result<String> {
    Ok(camofy::digest(serde_json::to_vec(&json!({
        "user":scope.user,"bundle":scope.bundle,"revision":scope.revision,
        "target":target_id(target),"compiler":COMPILER_VERSION,"geosite":GEOSITE_REVISION,
        "plan":plan,"preview_input":if scope.revision.is_none() {Some(config)} else {None}
    }))?))
}

pub(crate) async fn validate_scope(app: &App, scope: Scope) -> Result<()> {
    let exists: bool = if let Some(revision) = scope.revision {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3)",
        )
        .bind(revision)
        .bind(scope.user)
        .bind(scope.bundle)
        .fetch_one(&app.db)
        .await?
    } else {
        sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM resources WHERE id=$1 AND user_id=$2 AND kind='bundle')",
        )
        .bind(scope.bundle)
        .bind(scope.user)
        .fetch_one(&app.db)
        .await?
    };
    ensure!(exists, "规则快照不属于该身份或版本已过期");
    Ok(())
}

async fn cached(app: &App, scope: Scope, key: &str) -> Result<Option<ResourceSnapshot>> {
    let sealed: Option<Value> = sqlx::query_scalar(
        "SELECT sealed FROM config_resource_snapshots WHERE cache_key=$1 AND user_id=$2 AND bundle_id=$3 AND sealed IS NOT NULL"
    ).bind(key).bind(scope.user).bind(scope.bundle).fetch_optional(&app.db).await?;
    sealed
        .map(|value| {
            let snapshot: ResourceSnapshot = serde_json::from_value(app.vault.open(value)?)?;
            ensure!(
                snapshot.fingerprint == key
                    && snapshot.compiler_version == COMPILER_VERSION
                    && snapshot.geosite_revision == GEOSITE_REVISION
                    && snapshot.source_count <= MAX_SOURCES
                    && snapshot.total_bytes <= MAX_TOTAL_BYTES,
                "已锁定规则快照的元数据无效"
            );
            Ok(snapshot)
        })
        .transpose()
}

async fn fetch_source(target: &url::Url, private: bool) -> Result<Vec<u8>> {
    let egress = crate::security::egress_client(target, None, private, Duration::from_secs(15))
        .await
        .map_err(|_| anyhow::anyhow!("规则资源地址不可访问或不符合网络安全策略"))?;
    let mut response = egress
        .client()
        .get(target.clone())
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("规则资源下载失败"))?;
    ensure!(
        response.status().is_success(),
        "规则资源未返回成功响应；不跟随重定向或替换来源"
    );
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size <= MAX_SOURCE_BYTES as u64),
        "单项规则资源超过 8 MiB 上限"
    );
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| anyhow::anyhow!("规则资源下载中断"))?
    {
        ensure!(
            bytes.len() + chunk.len() <= MAX_SOURCE_BYTES,
            "单项规则资源超过 8 MiB 上限"
        );
        bytes.extend_from_slice(&chunk);
    }
    ensure!(!bytes.is_empty(), "规则资源为空，完整配置转换已阻止");
    Ok(bytes)
}

async fn load_sources(
    config: &serde_yaml::Value,
    target: Target,
    plan: &DependencyPlan,
    key: &str,
) -> Result<ResourceSnapshot> {
    load_sources_with(config, target, plan, key, |url| async move {
        fetch_source(&url, false).await
    })
    .await
}

async fn load_sources_with<F, Fut>(
    config: &serde_yaml::Value,
    target: Target,
    plan: &DependencyPlan,
    key: &str,
    fetch: F,
) -> Result<ResourceSnapshot>
where
    F: Fn(url::Url) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<u8>>>,
{
    // A single resolver limits memory amplification from decoding + JSON + AES.
    static SLOTS: OnceLock<Semaphore> = OnceLock::new();
    let _slot = tokio::time::timeout(
        Duration::from_secs(5),
        SLOTS.get_or_init(|| Semaphore::new(1)).acquire(),
    )
    .await
    .map_err(|_| anyhow::anyhow!("规则资源解析繁忙，请稍后重试"))??;
    let mut sources = ResolvedSources::new();
    let mut total_bytes = 0;
    let mut current_plan = plan.clone();
    for _ in 0..=MAX_SOURCES {
        validate_plan(config, &current_plan)?;
        let mut fetched = false;
        for source in &current_plan.sources {
            if sources.contains_key(&source.key) {
                continue;
            }
            ensure!(
                sources.len() < MAX_SOURCES,
                "完整配置最多解析 32 项规则资源"
            );
            let url = source_url(source)?;
            // Untrusted provider destinations never gain private-network access
            // from a client-side proxy, header, file path or redirect.
            let bytes = fetch(url).await?;
            total_bytes += bytes.len();
            ensure!(
                total_bytes <= MAX_TOTAL_BYTES,
                "完整配置规则资源总量超过 24 MiB 上限"
            );
            let hash = camofy::digest(&bytes);
            let text = String::from_utf8(bytes)
                .map_err(|_| anyhow::anyhow!("规则资源不是有效 UTF-8 文本，当前格式无法解析"))?;
            sources.insert(
                source.key.clone(),
                ResolvedSource {
                    content: SourceContent::Text(text),
                    hash,
                    kind: source.kind,
                },
            );
            fetched = true;
        }
        if !fetched {
            break;
        }
        current_plan = config_compat::plan_with_sources(config, target, &sources)?;
    }
    ensure!(
        current_plan
            .sources
            .iter()
            .all(|source| sources.contains_key(&source.key)),
        "规则资源依赖未收敛，完整配置转换已阻止"
    );
    // Planning intentionally defers checks that need complete source data,
    // notably DNS selector expressibility. Do not permanently pin a failed
    // candidate: the lease owner's error path releases its claim for retry.
    config_compat::compile(config, target, &sources)?;
    Ok(ResourceSnapshot {
        source_count: sources.len(),
        sources,
        fingerprint: key.into(),
        compiler_version: COMPILER_VERSION.into(),
        geosite_revision: GEOSITE_REVISION.into(),
        total_bytes,
    })
}

/// Called only after releasing publication/read transactions. A non-empty plan
/// is pinned once per revision and target; refreshes never fetch a moving latest.
pub async fn resolve(
    app: &App,
    scope: Scope,
    target: Target,
    config: &serde_yaml::Value,
) -> Result<ResourceSnapshot> {
    tokio::time::timeout(TOTAL_TIMEOUT, resolve_inner(app, scope, target, config))
        .await
        .map_err(|_| anyhow::anyhow!("规则资源解析超时，请稍后重试；未交付不完整配置"))?
}

async fn claim_snapshot(app: &App, scope: Scope, key: &str, claim: Uuid) -> Result<bool> {
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO config_resource_snapshots(cache_key,user_id,bundle_id,revision_id,claim,lease_until) VALUES($1,$2,$3,$4,$5,now()+interval '55 seconds') ON CONFLICT(cache_key) DO UPDATE SET claim=EXCLUDED.claim,lease_until=EXCLUDED.lease_until,updated_at=now() WHERE config_resource_snapshots.sealed IS NULL AND config_resource_snapshots.lease_until<now() RETURNING claim"
    ).bind(key).bind(scope.user).bind(scope.bundle).bind(scope.revision).bind(claim)
        .fetch_optional(&app.db).await?;
    Ok(claimed == Some(claim))
}

async fn resolve_inner(
    app: &App,
    scope: Scope,
    target: Target,
    config: &serde_yaml::Value,
) -> Result<ResourceSnapshot> {
    validate_scope(app, scope).await?;
    if scope.revision.is_none() {
        sqlx::query("DELETE FROM config_resource_snapshots WHERE user_id=$1 AND bundle_id=$2 AND revision_id IS NULL AND sealed IS NULL AND lease_until<now()")
            .bind(scope.user).bind(scope.bundle).execute(&app.db).await?;
    }
    let plan = config_compat::plan(config, target)?;
    validate_plan(config, &plan)?;
    let key = cache_key(scope, target, config, &plan)?;
    if plan.sources.is_empty() {
        return Ok(ResourceSnapshot {
            sources: ResolvedSources::new(),
            fingerprint: key,
            compiler_version: COMPILER_VERSION.into(),
            geosite_revision: GEOSITE_REVISION.into(),
            source_count: 0,
            total_bytes: 0,
        });
    }
    let claim = Uuid::new_v4();
    loop {
        if let Some(snapshot) = cached(app, scope, &key).await? {
            return Ok(snapshot);
        }
        if claim_snapshot(app, scope, &key, claim).await? {
            break;
        }
        tokio::time::sleep(Duration::from_millis(150)).await;
    }
    // Timeout is also inside the lease owner, allowing a failed lease to be
    // released immediately; cancellation/crash remains bounded by lease_until.
    let loaded = tokio::time::timeout(
        Duration::from_secs(40),
        load_sources(config, target, &plan, &key),
    )
    .await
    .map_err(|_| anyhow::anyhow!("规则资源下载超时，请稍后重试"))
    .and_then(|result| result);
    let snapshot = match loaded {
        Ok(snapshot) => snapshot,
        Err(error) => {
            let _ = sqlx::query("DELETE FROM config_resource_snapshots WHERE cache_key=$1 AND claim=$2 AND sealed IS NULL")
                .bind(&key).bind(claim).execute(&app.db).await;
            return Err(error);
        }
    };
    let sealed = app.vault.seal(&serde_json::to_value(&snapshot)?)?;
    let stored = sqlx::query("UPDATE config_resource_snapshots SET sealed=$3,updated_at=now() WHERE cache_key=$1 AND claim=$2 AND sealed IS NULL AND lease_until>now()")
        .bind(&key).bind(claim).bind(sealed).execute(&app.db).await?;
    ensure!(
        stored.rows_affected() == 1,
        "规则快照保存冲突或身份版本已过期，请重试"
    );
    if scope.revision.is_none() {
        // Preview inputs are private, short-lived cache entries, never a public
        // resource store. Keep a small bounded history per identity.
        sqlx::query("DELETE FROM config_resource_snapshots WHERE user_id=$1 AND bundle_id=$2 AND revision_id IS NULL AND sealed IS NOT NULL AND cache_key NOT IN (SELECT cache_key FROM config_resource_snapshots WHERE user_id=$1 AND bundle_id=$2 AND revision_id IS NULL ORDER BY updated_at DESC LIMIT 8)")
            .bind(scope.user).bind(scope.bundle).execute(&app.db).await?;
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use camofy::config_compat::RuleBehavior;

    #[tokio::test]
    async fn incomplete_dns_compilation_never_produces_a_persistable_snapshot() {
        let config = camofy::engine::parse("rule-providers: {sample: {type: http, url: 'https://rules.example/source', format: text, behavior: classical}}\ndns: {fake-ip-filter: ['rule-set:sample']}\nrules: ['MATCH,DIRECT']").unwrap();
        let plan = config_compat::plan(&config, Target::Shadowrocket).unwrap();
        let failed = load_sources_with(
            &config,
            Target::Shadowrocket,
            &plan,
            "synthetic-key",
            |_| async { Ok(b"DOMAIN-KEYWORD,sensitive.example\n".to_vec()) },
        )
        .await;
        let error = failed
            .err()
            .expect("an unrepresentable DNS source must fail before sealing")
            .to_string();
        assert!(error.contains("cannot be represented"));
        assert!(!error.contains("sensitive"));
        // A corrected upstream response must remain retryable on the same key.
        let corrected = load_sources_with(
            &config,
            Target::Shadowrocket,
            &plan,
            "synthetic-key",
            |_| async { Ok(b"DOMAIN,example.org\n".to_vec()) },
        )
        .await
        .unwrap();
        assert_eq!(corrected.source_count, 1);
    }

    fn provider(url: &str) -> SourceRequest {
        SourceRequest {
            key: "provider:sample".into(),
            kind: SourceKind::RuleProvider,
            url: Some(url.into()),
            format: SourceFormat::Text,
            behavior: RuleBehavior::Classical,
            geosite_tag: None,
        }
    }

    #[test]
    fn source_urls_are_pinned_and_keep_attribute_semantics() {
        let ordinary = geosite_url("geolocation-!cn").unwrap();
        assert!(ordinary.path().contains(GEOSITE_REVISION));
        assert!(ordinary.path().contains("/geo/geosite/classical/"));
        assert!(ordinary.path().ends_with("geolocation-!cn.list"));
        assert!(
            geosite_url("adobe@cn")
                .unwrap()
                .path()
                .ends_with("adobe@cn.list")
        );
        for tag in [
            "../private",
            "category?token=sample",
            "!cn",
            "category/other",
            "",
        ] {
            assert!(geosite_url(tag).is_err());
        }
        for url in [
            "file:///example",
            "https://demo:sample@example.invalid/rules",
            "https://example.invalid/rules#fragment",
        ] {
            assert!(source_url(&provider(url)).is_err());
        }
        let custom = SourceRequest {
            kind: SourceKind::Geosite,
            format: SourceFormat::Dat,
            url: Some("https://example.invalid/custom.dat".into()),
            ..provider("https://example.invalid/rules")
        };
        let error = source_url(&custom).unwrap_err().to_string();
        assert!(error.contains("DAT"));
        assert!(!error.contains("example.invalid"));
    }

    #[test]
    fn unsupported_request_semantics_are_not_ignored() {
        let plan = DependencyPlan {
            sources: vec![provider("https://example.invalid/rules")],
        };
        for declaration in [
            "type: file",
            "type: http, proxy: Route",
            "type: http, header: {Authorization: ['sample']}",
        ] {
            let config =
                camofy::engine::parse(&format!("rule-providers: {{sample: {{{declaration}}}}}"))
                    .unwrap();
            assert!(validate_plan(&config, &plan).is_err());
        }
        let config =
            camofy::engine::parse("rule-providers: {sample: {type: http, header: {}}}").unwrap();
        validate_plan(&config, &plan).unwrap();
    }

    #[test]
    fn cache_fingerprints_isolate_identity_revision_target_and_draft_inputs() {
        let scope = Scope {
            user: Uuid::new_v4(),
            bundle: Uuid::new_v4(),
            revision: Some(Uuid::new_v4()),
        };
        let config = camofy::engine::parse("rules: ['GEOSITE,cn,DIRECT']").unwrap();
        let plan = config_compat::plan(&config, Target::Shadowrocket).unwrap();
        let key = cache_key(scope, Target::Shadowrocket, &config, &plan).unwrap();
        for other in [
            Scope {
                user: Uuid::new_v4(),
                ..scope
            },
            Scope {
                bundle: Uuid::new_v4(),
                ..scope
            },
            Scope {
                revision: Some(Uuid::new_v4()),
                ..scope
            },
            Scope {
                revision: None,
                ..scope
            },
        ] {
            assert_ne!(
                key,
                cache_key(other, Target::Shadowrocket, &config, &plan).unwrap()
            );
        }
        let draft = Scope {
            revision: None,
            ..scope
        };
        let mut changed = config.clone();
        changed["mode"] = "rule".into();
        assert_ne!(
            cache_key(draft, Target::Shadowrocket, &config, &plan).unwrap(),
            cache_key(draft, Target::Shadowrocket, &changed, &plan).unwrap()
        );
    }

    #[tokio::test]
    #[ignore = "requires TEST_DATABASE_URL pointing to disposable PostgreSQL"]
    async fn config_resource_snapshot_persistence_and_tenant_scope() {
        use base64::{Engine, engine::general_purpose::STANDARD};
        use std::sync::Arc;
        let db = sqlx::postgres::PgPoolOptions::new()
            .max_connections(4)
            .connect(&std::env::var("TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&db).await.unwrap();
        let app = App {
            db: db.clone(),
            vault: crate::security::Vault::new(&STANDARD.encode([43; 32])).unwrap(),
            origin: "https://cloud.example".into(),
            legacy_origins: vec![],
            secure: false,
            registration: false,
            private_egress: false,
            workers: 1,
            captcha: None,
            topics: Default::default(),
            hash_slots: Arc::new(Semaphore::new(1)),
        };
        let scope = Scope {
            user: Uuid::new_v4(),
            bundle: Uuid::new_v4(),
            revision: Some(Uuid::new_v4()),
        };
        sqlx::query("INSERT INTO users(id,email,password) VALUES($1,$2,'unused')")
            .bind(scope.user)
            .bind(format!("{}@example.test", scope.user))
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO resources(id,user_id,kind,data) VALUES($1,$2,'bundle',$3)")
            .bind(scope.bundle)
            .bind(scope.user)
            .bind(app.vault.seal(&json!({"name":"fixture"})).unwrap())
            .execute(&db)
            .await
            .unwrap();
        sqlx::query("INSERT INTO revisions(id,user_id,bundle_id,artifacts) VALUES($1,$2,$3,$4)")
            .bind(scope.revision)
            .bind(scope.user)
            .bind(scope.bundle)
            .bind(app.vault.seal(&json!({})).unwrap())
            .execute(&db)
            .await
            .unwrap();
        let config = camofy::engine::parse("rule-providers: {sample: {type: http, url: 'https://unavailable.example.invalid/rules', format: text, behavior: classical}}\nrules: ['RULE-SET,sample,DIRECT']").unwrap();
        let plan = config_compat::plan(&config, Target::Shadowrocket).unwrap();
        let key = cache_key(scope, Target::Shadowrocket, &config, &plan).unwrap();
        let content = "DOMAIN,example.test\n";
        let snapshot = ResourceSnapshot {
            sources: ResolvedSources::from([(
                "provider:sample".into(),
                ResolvedSource {
                    content: SourceContent::Text(content.into()),
                    hash: camofy::digest(content),
                    kind: SourceKind::RuleProvider,
                },
            )]),
            fingerprint: key.clone(),
            compiler_version: COMPILER_VERSION.into(),
            geosite_revision: GEOSITE_REVISION.into(),
            source_count: 1,
            total_bytes: content.len(),
        };
        let sealed = app
            .vault
            .seal(&serde_json::to_value(&snapshot).unwrap())
            .unwrap();
        assert!(!sealed.to_string().contains("example.test"));
        sqlx::query("INSERT INTO config_resource_snapshots(cache_key,user_id,bundle_id,revision_id,sealed,claim,lease_until) VALUES($1,$2,$3,$4,$5,$6,now())")
            .bind(&key).bind(scope.user).bind(scope.bundle).bind(scope.revision).bind(sealed).bind(Uuid::new_v4())
            .execute(&db).await.unwrap();
        // The deliberately unresolvable source proves warm reads need no network.
        for _ in 0..2 {
            let result = resolve(&app, scope, Target::Shadowrocket, &config)
                .await
                .unwrap();
            assert_eq!(result.fingerprint, key);
            assert_eq!(result.total_bytes, content.len());
        }
        assert!(
            resolve(
                &app,
                Scope {
                    user: Uuid::new_v4(),
                    ..scope
                },
                Target::Shadowrocket,
                &config
            )
            .await
            .is_err()
        );
        assert!(
            cached(
                &app,
                Scope {
                    user: Uuid::new_v4(),
                    ..scope
                },
                &key
            )
            .await
            .unwrap()
            .is_none()
        );
        let pending_key =
            camofy::digest("synthetic-concurrent-lease".to_owned() + &scope.bundle.to_string());
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let (left, right) = tokio::join!(
            claim_snapshot(&app, scope, &pending_key, first),
            claim_snapshot(&app, scope, &pending_key, second)
        );
        assert_ne!(
            left.unwrap(),
            right.unwrap(),
            "only one concurrent resolver may claim a cold input"
        );
        sqlx::query("UPDATE config_resource_snapshots SET lease_until=now()-interval '1 second' WHERE cache_key=$1")
            .bind(&pending_key).execute(&db).await.unwrap();
        assert!(
            claim_snapshot(&app, scope, &pending_key, Uuid::new_v4())
                .await
                .unwrap()
        );
        sqlx::query("DELETE FROM revisions WHERE id=$1")
            .bind(scope.revision)
            .execute(&db)
            .await
            .unwrap();
        assert!(cached(&app, scope, &key).await.unwrap().is_none());
        sqlx::query("DELETE FROM users WHERE id=$1")
            .bind(scope.user)
            .execute(&db)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn download_rejects_redirects_private_addresses_and_oversized_bodies() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for response in [
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1/private\r\nContent-Length: 0\r\n\r\n".as_slice(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 9000000\r\n\r\n".as_slice(),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = url::Url::parse(&format!("http://{}/rules", listener.local_addr().unwrap())).unwrap();
            let response = response.to_vec();
            let server = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                let _ = stream.read(&mut request).await;
                stream.write_all(&response).await.unwrap();
            });
            let error = fetch_source(&url, true).await.unwrap_err().to_string();
            assert!(!error.contains("127.0.0.1"));
            server.await.unwrap();
            assert!(fetch_source(&url, false).await.is_err());
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url =
            url::Url::parse(&format!("http://{}/rules", listener.local_addr().unwrap())).unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 2048];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n")
                .await
                .unwrap();
            let _ = stream.write_all(&vec![b'x'; MAX_SOURCE_BYTES + 1]).await;
        });
        assert!(
            fetch_source(&url, true)
                .await
                .unwrap_err()
                .to_string()
                .contains("8 MiB")
        );
        server.await.unwrap();
    }
}
