//! Upstream quota metadata, independent of configuration revisions and device traffic.
use crate::{App, Error, store::Resource};
use axum::http::HeaderMap;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct Sample {
    pub upload: Option<u64>,
    pub download: Option<u64>,
    pub total: Option<u64>,
    pub expire: Option<u64>,
}
impl Sample {
    fn complete(&self) -> bool {
        self.upload.is_some()
            && self.download.is_some()
            && self.total.is_some_and(|v| v > 0)
            && self
                .upload
                .unwrap()
                .checked_add(self.download.unwrap())
                .is_some()
    }
}
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Snapshot {
    pub status: String,
    pub sample: Option<Sample>,
    pub observed_at: u64,
    #[serde(default)]
    pub source_fingerprint: String,
}
pub fn parse_headers(headers: &HeaderMap, now: u64) -> Snapshot {
    let standard: Vec<_> = headers.get_all("subscription-userinfo").iter().collect();
    let values = if standard.is_empty() {
        [
            "x-amz-meta-subscription-userinfo",
            "x-obs-meta-subscription-userinfo",
            "x-cos-meta-subscription-userinfo",
        ]
        .iter()
        .flat_map(|name| headers.get_all(*name).iter())
        .collect::<Vec<_>>()
    } else {
        standard
    };
    let mut result = Snapshot {
        status: "missing".into(),
        sample: None,
        observed_at: now,
        source_fingerprint: String::new(),
    };
    if values.is_empty() {
        return result;
    }
    result.status = "invalid".into();
    if values.len() != 1 {
        return result;
    }
    let Ok(text) = values[0].to_str() else {
        return result;
    };
    if text.len() > 4096 {
        return result;
    }
    let mut fields = BTreeMap::new();
    for part in text.split(';').filter(|s| !s.trim().is_empty()) {
        let Some((key, value)) = part.trim().split_once('=') else {
            return result;
        };
        let key = key.trim().to_ascii_lowercase();
        if !["upload", "download", "total", "expire"].contains(&key.as_str()) {
            continue;
        }
        let value = value.trim();
        if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
            return result;
        }
        let Ok(n) = value.parse::<u64>() else {
            return result;
        };
        if fields.insert(key, n).is_some() {
            return result;
        }
    }
    if fields.is_empty() {
        return result;
    }
    let sample = Sample {
        upload: fields.get("upload").copied(),
        download: fields.get("download").copied(),
        total: fields.get("total").copied(),
        expire: fields.get("expire").copied().filter(|v| *v > 0),
    };
    if sample.expire.is_some_and(|v| v > 253402300799) {
        return result;
    }
    result.status = "ok".into();
    result.sample = Some(sample);
    result
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
pub struct Source {
    pub profile_id: Uuid,
    pub fingerprint: String,
}
pub fn fingerprint(app: &App, user: Uuid, p: &Resource) -> String {
    app.vault
        .source_fingerprint(user, p.data["url"].as_str().unwrap_or(""))
}
pub fn sources(app: &App, user: Uuid, records: &[Resource], data: &Value) -> Vec<Source> {
    data["profiles"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|b| b["enabled"] == true)
        .filter_map(|b| {
            records.iter().find(|r| {
                r.kind == "profile"
                    && r.data["type"] == "source"
                    && Some(r.id.to_string()).as_deref() == b["profile_id"].as_str()
            })
        })
        .map(|p| Source {
            profile_id: p.id,
            fingerprint: p.data["content_fingerprint"]
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| fingerprint(app, user, p)),
        })
        .collect()
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Pool {
    pub profile_ids: Vec<Uuid>,
    pub names: Vec<String>,
    pub status: String,
    pub upload: Option<String>,
    pub download: Option<String>,
    pub total: Option<String>,
    pub expire: Option<u64>,
    pub updated_at: Option<u64>,
    pub expired: bool,
    pub stale: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct Summary {
    pub status: String,
    pub known_pools: usize,
    pub total_pools: usize,
    pub upload: String,
    pub download: String,
    pub total: String,
    pub remaining: String,
    pub expire: Option<u64>,
    pub next_expire: Option<u64>,
    pub updated_at: Option<u64>,
    pub pools: Vec<Pool>,
    #[serde(skip)]
    pub header: Option<String>,
}
pub fn summarize(
    app: &App,
    user: Uuid,
    records: &[Resource],
    refs: Option<&[Source]>,
    now: u64,
) -> Summary {
    let mut result = Summary {
        status: if refs.is_none() {
            "unavailable"
        } else {
            "empty"
        }
        .into(),
        known_pools: 0,
        total_pools: 0,
        upload: "0".into(),
        download: "0".into(),
        total: "0".into(),
        remaining: "0".into(),
        expire: None,
        next_expire: None,
        updated_at: None,
        pools: vec![],
        header: None,
    };
    let Some(refs) = refs else {
        return result;
    };
    let mut seen = HashSet::new();
    let refs: Vec<_> = refs.iter().filter(|s| seen.insert(s.profile_id)).collect();
    let records: Vec<_> = refs
        .iter()
        .map(|s| {
            records
                .iter()
                .find(|p| p.id == s.profile_id && p.kind == "profile" && p.data["type"] == "source")
        })
        .collect();
    // Union by both canonical source and explicit quota pool. A manual label cannot
    // accidentally defeat automatic same-URL deduplication.
    let mut parents: Vec<_> = (0..refs.len()).collect();
    fn root(parents: &[usize], mut n: usize) -> usize {
        while parents[n] != n {
            n = parents[n];
        }
        n
    }
    let mut keys = BTreeMap::new();
    for (i, source) in refs.iter().enumerate() {
        let mut aliases = vec![format!("url:{}", source.fingerprint)];
        if let Some(pool) = records[i]
            .and_then(|p| p.data["usage_pool"].as_str())
            .filter(|s| !s.is_empty())
        {
            aliases.push(format!("pool:{pool}"));
        }
        for key in aliases {
            if let Some(j) = keys.insert(key, i) {
                let r = root(&parents, j);
                parents[r] = root(&parents, i);
            }
        }
    }
    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..refs.len() {
        groups.entry(root(&parents, i)).or_default().push(i);
    }
    let (mut upload, mut download, mut total, mut remaining) = (0u64, 0u64, 0u64, 0u64);
    let mut overflow = false;
    let mut expiries = vec![];
    for members in groups.values() {
        let mut pool = Pool {
            status: "missing".into(),
            ..Default::default()
        };
        let mut candidates = vec![];
        for &i in members {
            pool.profile_ids.push(refs[i].profile_id);
            let Some(p) = records[i] else {
                pool.names.push("已删除的订阅".into());
                pool.status = "source_changed".into();
                continue;
            };
            pool.names
                .push(p.data["name"].as_str().unwrap_or("订阅").into());
            if refs[i].fingerprint != fingerprint(app, user, p) {
                pool.status = "source_changed".into();
                continue;
            }
            let Ok(snapshot) = serde_json::from_value::<Snapshot>(p.data["usage"].clone()) else {
                continue;
            };
            if snapshot.source_fingerprint != refs[i].fingerprint {
                pool.status = "source_changed".into();
                continue;
            }
            pool.updated_at = Some(pool.updated_at.unwrap_or(0).max(snapshot.observed_at));
            pool.status.clone_from(&snapshot.status);
            if !["ok", "stale"].contains(&snapshot.status.as_str()) {
                continue;
            }
            let age_limit = p.data["interval_seconds"]
                .as_u64()
                .unwrap_or(3600)
                .saturating_mul(2)
                .max(3600);
            if now.saturating_sub(snapshot.observed_at) > age_limit {
                pool.status = "outdated".into();
                continue;
            }
            let Some(sample) = snapshot.sample.clone() else {
                continue;
            };
            if !sample.complete() {
                pool.status = "incomplete".into();
                pool.upload = sample.upload.map(|v| v.to_string());
                pool.download = sample.download.map(|v| v.to_string());
                pool.total = sample.total.map(|v| v.to_string());
                pool.expire = sample.expire;
                continue;
            }
            candidates.push((snapshot.observed_at, sample, snapshot.status == "stale"));
        }
        candidates.sort_by_key(|c| (std::cmp::Reverse(c.0), c.2));
        if let Some((at, sample, stale)) = candidates.first() {
            if candidates.iter().any(|c| c.0 == *at && c.1 != *sample) {
                pool.status = "conflict".into();
            } else {
                pool.status = if *stale { "stale" } else { "ok" }.into();
                pool.stale = *stale;
                pool.updated_at = Some(*at);
                pool.upload = sample.upload.map(|v| v.to_string());
                pool.download = sample.download.map(|v| v.to_string());
                pool.total = sample.total.map(|v| v.to_string());
                pool.expire = sample.expire;
                result.known_pools += 1;
                result.updated_at = Some(result.updated_at.unwrap_or(*at).min(*at));
                expiries.push(sample.expire);
                let (u, d, t) = (
                    sample.upload.unwrap(),
                    sample.download.unwrap(),
                    sample.total.unwrap(),
                );
                if let (Some(a), Some(b), Some(c), Some(e)) = (
                    upload.checked_add(u),
                    download.checked_add(d),
                    total.checked_add(t),
                    remaining.checked_add(t.saturating_sub(u + d)),
                ) {
                    (upload, download, total, remaining) = (a, b, c, e);
                } else {
                    overflow = true;
                }
            }
        }
        pool.expired = pool.expire.is_some_and(|t| t <= now);
        result.pools.push(pool);
    }
    result.total_pools = result.pools.len();
    if result.total_pools == 0 {
        return result;
    }
    result.next_expire = result.pools.iter().filter_map(|p| p.expire).min();
    result.status = if overflow || upload.checked_add(download).is_none() {
        "overflow"
    } else if result.known_pools != result.total_pools {
        "partial"
    } else if result.pools.iter().any(|p| p.stale) {
        "stale"
    } else {
        "ok"
    }
    .into();
    if result.status == "overflow" {
        return result;
    }
    result.upload = upload.to_string();
    result.download = download.to_string();
    result.total = total.to_string();
    result.remaining = remaining.to_string();
    if ["ok", "stale"].contains(&result.status.as_str()) {
        result.expire = expiries
            .first()
            .copied()
            .flatten()
            .filter(|e| expiries.iter().all(|x| *x == Some(*e)));
        result.header = Some(format!(
            "upload={upload}; download={download}; total={total}{}",
            result
                .expire
                .map(|e| format!("; expire={e}"))
                .unwrap_or_default()
        ));
    }
    result
}

pub async fn published(
    app: &App,
    conn: &mut sqlx::PgConnection,
    user: Uuid,
    records: &[Resource],
    bundle: &Resource,
) -> Result<Summary, Error> {
    let revision = bundle.data["published_revision"]
        .as_str()
        .and_then(|s| Uuid::parse_str(s).ok());
    let refs: Option<Value> = sqlx::query_scalar(
        "SELECT usage_sources FROM revisions WHERE id=$1 AND user_id=$2 AND bundle_id=$3",
    )
    .bind(revision)
    .bind(user)
    .bind(bundle.id)
    .fetch_optional(&mut *conn)
    .await?
    .flatten();
    let refs = refs.and_then(|v| serde_json::from_value::<Vec<Source>>(v).ok());
    Ok(summarize(app, user, records, refs.as_deref(), crate::now()))
}
pub fn profile_view(app: &App, user: Uuid, p: &Resource) -> Summary {
    summarize(
        app,
        user,
        std::slice::from_ref(p),
        Some(&[Source {
            profile_id: p.id,
            fingerprint: fingerprint(app, user, p),
        }]),
        crate::now(),
    )
}
pub fn invalidate(data: &mut Value) {
    data["usage"] = Value::Null;
    data["usage_previous"] = Value::Null;
}
pub fn store_snapshot(data: &mut Value, mut snapshot: Snapshot, fingerprint: String) {
    snapshot.source_fingerprint = fingerprint;
    if snapshot.status != "ok" && data["usage"]["sample"].is_object() {
        data["usage_previous"] = data["usage"].clone();
    }
    data["usage"] = json!(snapshot);
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{Engine, engine::general_purpose::STANDARD};
    fn app() -> App {
        App {
            db: sqlx::postgres::PgPoolOptions::new()
                .connect_lazy("postgres://test:test@localhost/test")
                .unwrap(),
            vault: crate::security::Vault::new(&STANDARD.encode([42; 32])).unwrap(),
            origin: "https://cloud.example".into(),
            legacy_origins: vec![],
            secure: true,
            registration: true,
            private_egress: false,
            workers: 1,
            topics: Default::default(),
            hash_slots: std::sync::Arc::new(tokio::sync::Semaphore::new(1)),
        }
    }
    fn source(app: &App, user: Uuid, url: &str, value: &str) -> Resource {
        let mut p = Resource {
            id: Uuid::new_v4(),
            kind: "profile".into(),
            version: 1,
            data: json!({"name":"plan","type":"source","url":url,"interval_seconds":300}),
        };
        let mut headers = HeaderMap::new();
        headers.insert("subscription-userinfo", value.parse().unwrap());
        let fp = fingerprint(app, user, &p);
        store_snapshot(&mut p.data, parse_headers(&headers, 10000), fp);
        p
    }
    fn refs(app: &App, user: Uuid, ps: &[Resource]) -> Vec<Source> {
        ps.iter()
            .map(|p| Source {
                profile_id: p.id,
                fingerprint: fingerprint(app, user, p),
            })
            .collect()
    }
    #[test]
    fn strict_header_parser_and_priority() {
        for value in [
            "upload=-1;download=2;total=3",
            "upload=1; upload=2;total=3",
            "upload=18446744073709551616",
            "upload=1.5",
            "",
            "garbage",
        ] {
            let mut h = HeaderMap::new();
            h.insert("subscription-userinfo", value.parse().unwrap());
            assert_eq!(parse_headers(&h, 10).status, "invalid", "{value}");
        }
        let mut h = HeaderMap::new();
        assert_eq!(parse_headers(&h, 1).status, "missing");
        h.insert(
            "x-amz-meta-subscription-userinfo",
            "upload=1;download=2;total=3;expire=0".parse().unwrap(),
        );
        assert_eq!(parse_headers(&h, 1).sample.unwrap().expire, None);
        h.insert(
            "x-obs-meta-subscription-userinfo",
            "upload=1;download=2;total=3".parse().unwrap(),
        );
        assert_eq!(parse_headers(&h, 1).status, "invalid");
        h.insert(
            "subscription-userinfo",
            "upload=9;download=2;total=30;foo=bar".parse().unwrap(),
        );
        assert_eq!(parse_headers(&h, 1).sample.unwrap().upload, Some(9));
        h.append("subscription-userinfo", "upload=1".parse().unwrap());
        assert_eq!(parse_headers(&h, 1).status, "invalid");
    }
    #[tokio::test]
    async fn sums_dedupes_pools_and_omits_mixed_expiry() {
        let a = app();
        let user = Uuid::new_v4();
        let first = source(
            &a,
            user,
            "https://a.example/x?token=one",
            "upload=10;download=20;total=100;expire=20000",
        );
        let second = source(
            &a,
            user,
            "https://b.example/x",
            "upload=30;download=40;total=200;expire=30000",
        );
        let mut ps = vec![first.clone(), second.clone()];
        let rs = refs(&a, user, &ps);
        let result = summarize(&a, user, &ps, Some(&rs), 10001);
        assert_eq!(
            result.header.as_deref(),
            Some("upload=40; download=60; total=300")
        );
        assert_eq!(result.remaining, "200");
        assert_eq!(result.next_expire, Some(20000));
        let mut duplicate = first.clone();
        duplicate.id = Uuid::new_v4();
        duplicate.data["usage_pool"] = json!("same-plan");
        ps.push(duplicate);
        let rs = refs(&a, user, &ps);
        assert_eq!(summarize(&a, user, &ps, Some(&rs), 10001).total_pools, 2);
        ps[1].data["usage_pool"] = json!("same-plan");
        ps[1].data["usage"]["observed_at"] = json!(10001);
        let result = summarize(&a, user, &ps, Some(&rs), 10002);
        assert_eq!(result.total_pools, 1);
        assert_eq!(
            result.header.as_deref(),
            Some("upload=30; download=40; total=200; expire=30000")
        );
        ps[1].data["usage"]["observed_at"] = json!(10000);
        assert!(summarize(&a, user, &ps, Some(&rs), 10002).header.is_none());
    }
    #[tokio::test]
    async fn missing_stale_changed_and_overflow_never_fabricate_totals() {
        let a = app();
        let user = Uuid::new_v4();
        let p = source(
            &a,
            user,
            "https://a.example/x",
            "upload=10;download=20;total=100",
        );
        let mut ps = vec![p];
        let rs = refs(&a, user, &ps);
        ps[0].data["usage"]["status"] = json!("stale");
        assert_eq!(summarize(&a, user, &ps, Some(&rs), 10001).status, "stale");
        assert!(summarize(&a, user, &ps, Some(&rs), 13601).header.is_none());
        ps[0].data["url"] = json!("https://a.example/changed");
        assert_eq!(
            summarize(&a, user, &ps, Some(&rs), 10001).pools[0].status,
            "source_changed"
        );
        for header in [
            "upload=1;download=2;total=0",
            "upload=1;total=10",
            "upload=18446744073709551615;download=1;total=10",
        ] {
            ps = vec![source(&a, user, "https://b.example", header)];
            assert!(
                summarize(&a, user, &ps, Some(&refs(&a, user, &ps)), 10001)
                    .header
                    .is_none()
            );
        }
        ps = vec![
            source(
                &a,
                user,
                "https://a.example",
                "upload=0;download=0;total=18446744073709551615",
            ),
            source(&a, user, "https://b.example", "upload=1;download=0;total=2"),
        ];
        assert_eq!(
            summarize(&a, user, &ps, Some(&refs(&a, user, &ps)), 10001).status,
            "overflow"
        );
        assert_eq!(summarize(&a, user, &[], Some(&[]), 10001).status, "empty");
    }
    #[tokio::test]
    async fn fingerprint_is_private_and_disabled_overlays_do_not_participate() {
        let a = app();
        let user = Uuid::new_v4();
        let p = source(
            &a,
            user,
            "https://EXAMPLE.com:443/a?b=1&c=2",
            "upload=1;download=2;total=3",
        );
        assert_eq!(
            fingerprint(&a, user, &p),
            a.vault
                .source_fingerprint(user, "https://example.com/a?b=1&c=2")
        );
        assert_ne!(
            fingerprint(&a, user, &p),
            fingerprint(&a, Uuid::new_v4(), &p)
        );
        assert_ne!(
            fingerprint(&a, user, &p),
            a.vault
                .source_fingerprint(user, "https://example.com/a?c=2&b=1")
        );
        let mut overlay = p.clone();
        overlay.id = Uuid::new_v4();
        overlay.data["type"] = json!("overlay");
        let data = json!({"profiles":[{"profile_id":p.id,"enabled":false},{"profile_id":overlay.id,"enabled":true}]});
        assert!(sources(&a, user, &[p, overlay], &data).is_empty());
    }
}
