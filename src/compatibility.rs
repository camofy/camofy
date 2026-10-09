//! Public evidence, versioned independently from identity policy. Unknown is a
//! first-class result: UA heuristics never prove support for an unverified build.
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Debug, Deserialize, Serialize)]
pub struct Registry {
    pub schema_version: u32,
    pub version: String,
    pub checked_at: String,
    pub protocols: Vec<String>,
    pub profiles: BTreeMap<String, Profile>,
    pub clients: Vec<Client>,
    pub references: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Profile {
    pub name: String,
    pub protocols: Vec<String>,
    pub complete_protocols: bool,
    #[serde(default)]
    pub unsupported_protocols: Vec<String>,
    pub features: BTreeMap<String, bool>,
    pub evidence: Vec<String>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Client {
    pub id: String,
    pub name: String,
    pub tokens: Vec<String>,
    pub format: String,
    pub notes: String,
    pub releases: Vec<Release>,
    pub ranges: Vec<Range>,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Release {
    pub version: String,
    pub profile: Option<String>,
    pub evidence: String,
    pub confidence: String,
    #[serde(default)]
    pub prerelease: bool,
}
#[derive(Debug, Deserialize, Serialize)]
pub struct Range {
    pub min: String,
    pub max: String,
    pub profile: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Detection {
    pub family: Option<String>,
    pub name: Option<String>,
    pub version: Option<String>,
    pub profile: Option<String>,
    pub confidence: String,
    pub format: Option<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}
pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        serde_json::from_str(include_str!("compatibility/registry.json"))
            .expect("embedded compatibility registry must be valid")
    })
}
impl Profile {
    pub fn protocol(&self, kind: &str) -> Support {
        if self.protocols.iter().any(|s| s == kind) {
            Support::Supported
        } else if self.complete_protocols || self.unsupported_protocols.iter().any(|s| s == kind) {
            Support::Unsupported
        } else {
            Support::Unknown
        }
    }
    pub fn feature(&self, key: &str) -> Support {
        match self.features.get(key) {
            Some(true) => Support::Supported,
            Some(false) => Support::Unsupported,
            None => Support::Unknown,
        }
    }
}
impl Detection {
    pub fn capabilities(&self) -> Option<&'static Profile> {
        self.profile
            .as_ref()
            .and_then(|p| registry().profiles.get(p))
    }
}
fn version(s: &str) -> Option<([u32; 3], String)> {
    let s = s.strip_prefix('v').unwrap_or(s);
    let s = s
        .strip_suffix(".Meta")
        .or_else(|| s.strip_suffix(".meta"))
        .unwrap_or(s);
    let parts: Vec<_> = s.split('.').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut numbers = [0; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        numbers[i] = part.parse().ok()?;
    }
    Some((
        numbers,
        format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]),
    ))
}
fn token_version<'a>(ua: &'a str, lower: &str, token: &str) -> Option<(usize, Option<&'a str>)> {
    for (at, _) in lower.match_indices(token) {
        let end = at + token.len();
        if at > 0 && !matches!(lower.as_bytes()[at - 1], b' ' | b'(' | b';') {
            continue;
        }
        if end < lower.len() && !matches!(lower.as_bytes()[end], b'/' | b' ' | b';' | b')') {
            continue;
        }
        let tail = &ua[end..];
        if !tail.starts_with('/') && !tail.starts_with(" v") {
            return Some((at, None));
        }
        let value = tail.trim_start_matches(['/', ' ']);
        return Some((
            at,
            Some(value.split([' ', ';', ')', '(']).next().unwrap_or("")),
        ));
    }
    None
}
pub fn detect(ua: Option<&str>) -> Detection {
    let Some(ua) = ua.filter(|s| s.len() <= 512 && !s.chars().any(char::is_control)) else {
        return Detection {
            confidence: "unknown".into(),
            ..Default::default()
        };
    };
    let lower = ua.to_ascii_lowercase();
    let mut matches = Vec::new();
    for client in &registry().clients {
        for token in &client.tokens {
            if let Some((at, raw)) = token_version(ua, &lower, token) {
                matches.push((client, at, token.len(), raw.and_then(version)));
            }
        }
    }
    // A declared core version is more precise than an application's bundle.
    // Otherwise prefer the first product, then its most specific token.
    // Some wrappers append other product names for subscription negotiation.
    matches.sort_by_key(|(c, at, len, v)| {
        (
            c.id == "mihomo" && v.is_some(),
            std::cmp::Reverse(*at),
            *len,
        )
    });
    let Some((mut client, _, _, parsed)) = matches.pop() else {
        return Detection {
            confidence: "unknown".into(),
            ..Default::default()
        };
    };
    // App version spaces can overlap across platforms. Generic Surge UAs need
    // an explicit platform marker; Darwin alone identifies neither platform.
    if client.id == "surge" {
        let platform =
            if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ios;") {
                Some("surge-ios")
            } else if lower.contains("macintosh") || lower.contains("macos") {
                Some("surge-mac")
            } else {
                None
            };
        if let Some(platform) = platform {
            client = registry()
                .clients
                .iter()
                .find(|c| c.id == platform)
                .unwrap_or(client);
        }
    }
    if client.id == "stash" && (lower.contains("macintosh") || lower.contains("macos")) {
        client = registry()
            .clients
            .iter()
            .find(|c| c.id == "stash-mac")
            .unwrap_or(client);
    }
    let mut result = Detection {
        family: Some(client.id.clone()),
        name: Some(client.name.clone()),
        version: parsed.as_ref().map(|v| v.1.clone()),
        profile: None,
        confidence: "unknown".into(),
        format: Some(client.format.clone()),
    };
    if let Some((numbers, normalized)) = parsed {
        if let Some(release) = client.releases.iter().find(|r| r.version == normalized) {
            result.profile = release.profile.clone();
            if result.profile.is_some() {
                result.confidence = release.confidence.clone();
            }
        } else if let Some(range) = client.ranges.iter().find(|r| {
            version(&r.min).is_some_and(|v| numbers >= v.0)
                && version(&r.max).is_some_and(|v| numbers <= v.0)
        }) {
            result.profile = Some(range.profile.clone());
            result.confidence = "documented".into();
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn public_registry_links_every_known_profile_and_covers_version_boundaries() {
        let registry = registry();
        assert!(registry.clients.len() >= 15);
        assert!(
            registry
                .clients
                .iter()
                .map(|c| c.releases.len())
                .sum::<usize>()
                >= 150
        );
        for client in &registry.clients {
            for id in client
                .releases
                .iter()
                .filter_map(|r| r.profile.as_ref())
                .chain(client.ranges.iter().map(|r| &r.profile))
            {
                assert!(registry.profiles.contains_key(id), "missing profile {id}");
            }
        }
        let old = detect(Some("ClashMetaForAndroid/2.10.2.Meta"));
        assert_eq!(old.version.as_deref(), Some("2.10.2"));
        let old = old.capabilities().unwrap();
        assert_eq!(old.protocol("mieru"), Support::Unsupported);
        assert_eq!(old.protocol("anytls"), Support::Unsupported);
        assert_eq!(old.protocol("hysteria2"), Support::Supported);
        assert_eq!(
            detect(Some("mihomo/1.19.0"))
                .capabilities()
                .unwrap()
                .protocol("mieru"),
            Support::Supported
        );
        assert_eq!(
            detect(Some("mihomo/1.19.0"))
                .capabilities()
                .unwrap()
                .protocol("anytls"),
            Support::Unsupported
        );
        assert_eq!(
            detect(Some("mihomo/1.19.3"))
                .capabilities()
                .unwrap()
                .protocol("anytls"),
            Support::Supported
        );
        assert_eq!(
            detect(Some("mihomo/1.19.4"))
                .capabilities()
                .unwrap()
                .feature("mieru.udp-transport"),
            Support::Unsupported
        );
        assert_eq!(
            detect(Some("mihomo/1.19.17"))
                .capabilities()
                .unwrap()
                .feature("mieru.udp-transport"),
            Support::Supported
        );
    }
    #[test]
    fn unknown_and_ambiguous_agents_are_not_latest_versions() {
        for ua in [
            None,
            Some("clash.meta"),
            Some("Mozilla/5.0"),
            Some("ClashMetaForAndroid/99.0.0"),
            Some("mihomo/1.19.0-alpha"),
            Some("notmihomo/1.19.0"),
            Some("curl/8.0 https://mihomo/1.19.0"),
        ] {
            assert!(detect(ua).capabilities().is_none(), "{ua:?}");
        }
        assert_eq!(
            detect(Some("FlClash/v0.8.99 clash-verge"))
                .family
                .as_deref(),
            Some("flclash")
        );
        assert_eq!(
            detect(Some("clash-verge/v2.0.2 mihomo/1.19.17"))
                .family
                .as_deref(),
            Some("mihomo")
        );
        assert_eq!(
            detect(Some("notclash/1.0 FlClash/v0.8.99 Clash/1.18.0"))
                .family
                .as_deref(),
            Some("flclash")
        );
    }
    #[test]
    fn original_clash_xray_and_platform_version_spaces_remain_distinct() {
        let classic = detect(Some("Clash/1.18.0"));
        assert_eq!(classic.family.as_deref(), Some("clash-core"));
        assert_eq!(
            classic.capabilities().unwrap().protocol("vless"),
            Support::Unsupported
        );
        assert_eq!(
            classic.capabilities().unwrap().protocol("vmess"),
            Support::Supported
        );
        assert_eq!(
            detect(Some("v2rayNG/2.3.10"))
                .capabilities()
                .unwrap()
                .protocol("mieru"),
            Support::Unsupported
        );
        assert_eq!(
            detect(Some("v2rayNG/2.3.10"))
                .capabilities()
                .unwrap()
                .protocol("hysteria2"),
            Support::Supported
        );
        assert!(
            detect(Some("Surge/5.10.0 Darwin/24.1"))
                .capabilities()
                .is_none()
        );
        assert_eq!(
            detect(Some("Surge/5.10.0 (iPhone)"))
                .capabilities()
                .unwrap()
                .protocol("anytls"),
            Support::Unsupported
        );
        assert_eq!(
            detect(Some("Surge Mac/6.4.3"))
                .capabilities()
                .unwrap()
                .protocol("anytls"),
            Support::Supported
        );
        assert_eq!(
            detect(Some("Stash/3.3.0"))
                .capabilities()
                .unwrap()
                .feature("vless.reality"),
            Support::Unsupported
        );
        assert_eq!(
            detect(Some("Stash/3.3.3"))
                .capabilities()
                .unwrap()
                .feature("vless.reality"),
            Support::Supported
        );
        assert!(
            detect(Some("Stash/3.3.3 (Macintosh)"))
                .capabilities()
                .is_none()
        );
    }
    #[test]
    fn reviewed_source_branches_keep_legacy_trojan_and_vision_supported() {
        for ua in [
            "mihomo/1.15.0",
            "ClashMetaForAndroid/2.8.0.Meta",
            "ClashMetaForAndroid/2.8.8.Meta",
        ] {
            let profile = detect(Some(ua)).capabilities().unwrap();
            assert_eq!(profile.feature("trojan.ws"), Support::Supported, "{ua}");
            assert_eq!(profile.feature("trojan.grpc"), Support::Supported, "{ua}");
        }
        let profile = detect(Some("mihomo/1.19.0")).capabilities().unwrap();
        assert_eq!(
            profile.feature("vless.xtls-rprx-vision"),
            Support::Supported
        );
        // An unrecognized implementation shape or an absent field is not a
        // verified negative. Explicit TCP-only rejection remains a negative.
        assert_eq!(profile.feature("vless.xhttp"), Support::Unknown);
        assert_eq!(profile.feature("mieru.traffic-pattern"), Support::Unknown);
        assert_eq!(profile.feature("mieru.udp-transport"), Support::Unsupported);
    }
    #[test]
    fn stash_reality_support_is_scoped_to_its_transport() {
        for ua in ["Stash/3.3.3", "Stash/3.4.0", "Stash/3.5.0"] {
            let profile = detect(Some(ua)).capabilities().unwrap();
            assert_eq!(profile.feature("vless.reality.tcp"), Support::Supported);
            for network in ["ws", "h2", "http", "grpc", "xhttp", "httpupgrade"] {
                assert_eq!(
                    profile.feature(&format!("vless.reality.{network}")),
                    Support::Unsupported,
                    "{ua}: {network}"
                );
            }
        }
        let current = detect(Some("Stash/3.6.0")).capabilities().unwrap();
        assert_eq!(current.feature("vless.reality.ws"), Support::Supported);
        assert_eq!(current.feature("vless.reality.xhttp"), Support::Supported);
        assert_eq!(
            current.feature("vless.reality.unverified"),
            Support::Unknown
        );
        assert!(detect(Some("Stash/99.0.0")).capabilities().is_none());
    }
}
