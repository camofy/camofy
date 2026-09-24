//! Bounded, first-valid DoH resolution with per-host/policy TTL caching and single-flight misses.
//! Only provider adapters opt in. No user URL, path, query, credential or account reaches DoH.
use super::{Failure, FailureKind};
use crate::security;
use futures_util::{StreamExt, stream::FuturesUnordered};
use hickory_proto::{
    op::{Edns, Message, MessageType, OpCode, Query, ResponseCode},
    rr::{
        DNSClass, Name, RData, RecordType,
        rdata::opt::{ClientSubnet, EdnsOption},
    },
};
use std::{
    collections::{HashMap, HashSet},
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::{sync::Mutex, time::Instant};

/// Geographic routing is an adapter decision, not a global default. None explicitly disables ECS.
#[derive(Clone, Copy, Debug, Default, Hash, PartialEq, Eq)]
pub struct DnsPolicy {
    pub subnet: Option<(Ipv4Addr, u8)>,
}

#[derive(Clone)]
struct Answer {
    ips: Vec<IpAddr>,
    expires: Instant,
}

#[derive(Hash, PartialEq, Eq)]
struct Key {
    host: String,
    policy: DnsPolicy,
}

type Slot = Arc<Mutex<Option<Answer>>>;

struct Resolver {
    endpoints: Vec<(&'static str, url::Url)>,
    cache: Mutex<HashMap<Key, Slot>>,
    timeout: Duration,
    // Only local test doubles may bypass public-address checks on the DoH endpoints themselves.
    private: bool,
}

const CACHE_LIMIT: usize = 128;
const MAX_TTL: Duration = Duration::from_secs(300);

fn resolver() -> &'static Resolver {
    static INSTANCE: OnceLock<Resolver> = OnceLock::new();
    INSTANCE.get_or_init(|| Resolver {
        endpoints: [
            ("google", "https://dns.google/dns-query"),
            ("alidns", "https://dns.alidns.com/dns-query"),
        ]
        .into_iter()
        .map(|(name, url)| (name, url.parse().unwrap()))
        .collect(),
        cache: Mutex::new(HashMap::new()),
        timeout: Duration::from_secs(4),
        private: false,
    })
}

pub(super) async fn resolve(
    target: &url::Url,
    policy: DnsPolicy,
) -> Result<Vec<SocketAddr>, Failure> {
    resolver().resolve(target, policy).await
}

pub(super) async fn invalidate(target: &url::Url, policy: DnsPolicy) {
    resolver().invalidate(target, policy).await;
}

impl Resolver {
    fn key(target: &url::Url, policy: DnsPolicy) -> Key {
        Key {
            host: target
                .host_str()
                .unwrap_or_default()
                .trim_end_matches('.')
                .to_ascii_lowercase(),
            policy,
        }
    }

    async fn invalidate(&self, target: &url::Url, policy: DnsPolicy) {
        self.cache.lock().await.remove(&Self::key(target, policy));
    }

    async fn resolve(
        &self,
        target: &url::Url,
        policy: DnsPolicy,
    ) -> Result<Vec<SocketAddr>, Failure> {
        // Includes waiting for another lookup; callers cannot get stuck behind a cache miss.
        tokio::time::timeout(self.timeout, self.lookup(target, policy))
            .await
            .map_err(|_| Failure::transient(FailureKind::DnsTimeout))?
    }

    async fn lookup(
        &self,
        target: &url::Url,
        policy: DnsPolicy,
    ) -> Result<Vec<SocketAddr>, Failure> {
        let key = Self::key(target, policy);
        let port = target
            .port_or_known_default()
            .ok_or_else(|| Failure::permanent(FailureKind::Configuration))?;
        let query = query(&key.host, policy)?;
        let slot = {
            let mut cache = self.cache.lock().await;
            if let Some(slot) = cache.get(&key) {
                slot.clone()
            } else {
                // Reclaim only idle entries; never evict another caller's in-flight lookup.
                if cache.len() >= CACHE_LIMIT {
                    cache.retain(|_, slot| Arc::strong_count(slot) > 1);
                }
                let slot = Arc::new(Mutex::new(None));
                if cache.len() < CACHE_LIMIT {
                    cache.insert(key, slot.clone());
                }
                slot
            }
        };
        let mut cached = slot.lock().await;
        if let Some(answer) = cached.as_ref().filter(|a| a.expires > Instant::now()) {
            return Ok(answer
                .ips
                .iter()
                .map(|ip| SocketAddr::new(*ip, port))
                .collect());
        }
        *cached = None;
        let mut pending = self
            .endpoints
            .iter()
            .map(|(label, endpoint)| {
                let query = &query;
                async move {
                    let started = Instant::now();
                    let result = self.ask(endpoint, query).await;
                    tracing::debug!(
                        resolver = label,
                        elapsed_ms = started.elapsed().as_millis(),
                        outcome = result
                            .as_ref()
                            .err()
                            .map(|e| e.diagnostic().0)
                            .unwrap_or("ok"),
                        "provider DNS query finished"
                    );
                    result
                }
            })
            .collect::<FuturesUnordered<_>>();
        let mut last = Failure::permanent(FailureKind::DnsInvalid);
        while let Some(result) = pending.next().await {
            match result {
                Ok(answer) => {
                    let addresses = answer
                        .ips
                        .iter()
                        .map(|ip| SocketAddr::new(*ip, port))
                        .collect();
                    *cached = Some(answer);
                    // Dropping the remaining futures cancels unused DNS requests. No extraction
                    // request has been issued at this point.
                    return Ok(addresses);
                }
                Err(error) if error.delay.is_some() || last.delay.is_none() => last = error,
                Err(_) => {}
            }
        }
        Err(last)
    }

    async fn ask(&self, endpoint: &url::Url, query: &Message) -> Result<Answer, Failure> {
        let addrs: Vec<_> = security::addresses(endpoint, self.private)
            .await
            .map_err(|_| Failure::transient(FailureKind::DnsUnavailable))?
            .into_iter()
            .filter(SocketAddr::is_ipv4)
            .collect();
        if addrs.is_empty() {
            return Err(Failure::transient(FailureKind::DnsUnavailable));
        }
        let bytes = query
            .to_vec()
            .map_err(|_| Failure::permanent(FailureKind::DnsInvalid))?;
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(self.timeout)
            .connect_timeout(self.timeout.min(Duration::from_secs(2)))
            .resolve_to_addrs(endpoint.host_str().unwrap(), &addrs)
            .build()
            .map_err(|_| Failure::transient(FailureKind::DnsUnavailable))?;
        let mut response = client
            .post(endpoint.clone())
            .header("Content-Type", "application/dns-message")
            .header("Accept", "application/dns-message")
            .body(bytes)
            .send()
            .await
            .map_err(dns_transport)?;
        if !response.status().is_success() {
            return Err(Failure::transient(FailureKind::DnsUnavailable));
        }
        if !response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| {
                v.split(';')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .eq_ignore_ascii_case("application/dns-message")
            })
            || response.content_length().is_some_and(|n| n > 65536)
        {
            return Err(Failure::permanent(FailureKind::DnsInvalid));
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(dns_transport)? {
            if body.len() + chunk.len() > 65536 {
                return Err(Failure::permanent(FailureKind::DnsInvalid));
            }
            body.extend(chunk);
        }
        parse(&body, query)
    }
}

fn dns_transport(error: reqwest::Error) -> Failure {
    Failure::transient(if error.is_timeout() {
        FailureKind::DnsTimeout
    } else {
        FailureKind::DnsUnavailable
    })
}

fn query(host: &str, policy: DnsPolicy) -> Result<Message, Failure> {
    let invalid = || Failure::permanent(FailureKind::DnsInvalid);
    // Wire decoding always yields a fully-qualified name. Normalize before comparison/cache
    // lookup so a caller's omitted trailing dot cannot turn a valid response into a mismatch.
    let name =
        Name::from_ascii(format!("{}.", host.trim_end_matches('.'))).map_err(|_| invalid())?;
    if name.is_root()
        || host.parse::<IpAddr>().is_ok()
        || policy.subnet.is_some_and(|(_, prefix)| prefix > 32)
    {
        return Err(invalid());
    }
    let (address, prefix) = policy.subnet.unwrap_or((Ipv4Addr::UNSPECIFIED, 0));
    let mut edns = Edns::new();
    edns.options_mut()
        .insert(EdnsOption::Subnet(ClientSubnet::new(
            address.into(),
            prefix,
            0,
        )));
    let mut message = Message::new(0, MessageType::Query, OpCode::Query);
    message.metadata.recursion_desired = true;
    message
        .add_query(Query::query(name, RecordType::A))
        .set_edns(edns);
    Ok(message)
}

fn parse(bytes: &[u8], query: &Message) -> Result<Answer, Failure> {
    let invalid = || Failure::permanent(FailureKind::DnsInvalid);
    let response = Message::from_vec(bytes).map_err(|_| invalid())?;
    if response.metadata.id != query.metadata.id
        || response.metadata.message_type != MessageType::Response
        || response.metadata.op_code != OpCode::Query
        || response.metadata.truncation
        || response.queries != query.queries
    {
        return Err(invalid());
    }
    if response.metadata.response_code != ResponseCode::NoError {
        return Err(match response.metadata.response_code {
            ResponseCode::ServFail | ResponseCode::Refused => {
                Failure::transient(FailureKind::DnsUnavailable)
            }
            _ => invalid(),
        });
    }
    let mut name = query.queries[0].name().clone();
    let mut seen = HashSet::new();
    let mut ttl = MAX_TTL;
    // Walk only the queried CNAME chain; unrelated additional records are never destinations.
    for _ in 0..16 {
        if !seen.insert(name.clone()) {
            return Err(invalid());
        }
        let mut next = None;
        let mut ips = Vec::new();
        for record in response.answers.iter().filter(|r| r.name == name) {
            if record.dns_class != DNSClass::IN {
                return Err(invalid());
            }
            match &record.data {
                RData::CNAME(cname) => {
                    if next.is_some() {
                        return Err(invalid());
                    }
                    next = Some(cname.0.clone());
                    ttl = ttl.min(Duration::from_secs(record.ttl.into()));
                }
                RData::A(a) => {
                    let ip = IpAddr::V4(a.0);
                    if !security::public_ip(ip) {
                        return Err(invalid());
                    }
                    if !ips.contains(&ip) {
                        ips.push(ip);
                    }
                    ttl = ttl.min(Duration::from_secs(record.ttl.into()));
                }
                _ => {}
            }
        }
        match next {
            Some(cname) if ips.is_empty() => name = cname,
            None if !ips.is_empty() => {
                return Ok(Answer {
                    ips,
                    expires: Instant::now() + ttl,
                });
            }
            _ => return Err(invalid()),
        }
    }
    Err(invalid())
}

#[cfg(test)]
mod tests;
