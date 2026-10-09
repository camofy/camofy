//! Full-configuration contracts, independent of node protocol support.
//!
//! A recognized family is not proof of a particular build's parser. Native
//! syntax and Clash-import syntax are deliberately separate capabilities.
use crate::compatibility::Detection;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::OnceLock};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Registry {
    pub schema_version: u32,
    pub version: String,
    pub checked_at: String,
    pub dimensions: BTreeMap<String, Vec<String>>,
    pub profiles: BTreeMap<String, Profile>,
    pub clients: Vec<Client>,
    pub references: Vec<String>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Profile {
    pub name: String,
    pub capabilities: BTreeMap<String, Capability>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Capability {
    pub support: Support,
    pub evidence: Vec<String>,
    pub notes: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct Client {
    /// Shares only the family identifier with the node compatibility registry.
    pub id: String,
    pub name: String,
    pub renderer: Option<String>,
    pub syntax: Option<String>,
    /// Only explicit family-level evidence belongs here. This is never the
    /// latest version profile and is not inherited from a node profile.
    pub baseline: Option<String>,
    pub ranges: Vec<VersionRange>,
    pub notes: String,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct VersionRange {
    pub min: String,
    pub max: String,
    pub profile: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CapabilityAssessment {
    pub support: Support,
    /// `version_range`, `family_baseline`, or `unknown`.
    pub basis: String,
    pub evidence: Vec<String>,
    pub notes: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct FullConfigurationAssessment {
    pub family: Option<String>,
    pub renderer: Option<String>,
    pub support: Support,
    pub blocked: bool,
    pub warnings: Vec<String>,
    pub reasons: Vec<String>,
}

pub fn registry() -> &'static Registry {
    static REGISTRY: OnceLock<Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        serde_json::from_str(include_str!("compatibility/config-registry.json"))
            .expect("embedded configuration compatibility registry must be valid")
    })
}

fn version(value: &str) -> Option<[u32; 3]> {
    let value = value.strip_prefix('v').unwrap_or(value);
    let parts: Vec<_> = value.split('.').collect();
    if !(2..=3).contains(&parts.len()) {
        return None;
    }
    let mut result = [0; 3];
    for (index, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        result[index] = part.parse().ok()?;
    }
    Some(result)
}

fn unknown(notes: &str) -> CapabilityAssessment {
    CapabilityAssessment {
        support: Support::Unknown,
        basis: "unknown".into(),
        evidence: Vec::new(),
        notes: notes.into(),
    }
}

fn assess(entry: &Capability, basis: &str) -> CapabilityAssessment {
    CapabilityAssessment {
        support: entry.support,
        basis: basis.into(),
        evidence: entry.evidence.clone(),
        notes: entry.notes.clone(),
    }
}

/// Looks up one capability without converting or dropping any configuration.
/// Missing evidence is unknown, including versions outside all reviewed ranges.
pub fn capability(detection: &Detection, key: &str) -> CapabilityAssessment {
    let Some(client) = detection
        .family
        .as_deref()
        .and_then(|family| registry().clients.iter().find(|client| client.id == family))
    else {
        return unknown("客户端家族未识别；未推断配置能力。");
    };
    if let Some(version) = detection.version.as_deref().and_then(version) {
        // Ranges may add disjoint capabilities. Conflicting entries are caught
        // by registry tests rather than made dependent on their JSON order.
        for range in &client.ranges {
            let Some(min) = self::version(&range.min) else {
                continue;
            };
            let Some(max) = self::version(&range.max) else {
                continue;
            };
            if min <= version
                && version <= max
                && let Some(entry) = registry()
                    .profiles
                    .get(&range.profile)
                    .and_then(|profile| profile.capabilities.get(key))
            {
                return assess(entry, "version_range");
            }
        }
    }
    if let Some(entry) = client
        .baseline
        .as_ref()
        .and_then(|name| registry().profiles.get(name))
        .and_then(|profile| profile.capabilities.get(key))
    {
        return assess(entry, "family_baseline");
    }
    unknown("缺少该客户端版本及字段的可核实证据；保留为未知，不等于不支持。")
}

/// Chooses an implemented full renderer. Field-level compatibility still needs
/// separate assessment; a supported syntax never implies all fields work.
/// This function deliberately has no node-only fallback.
pub fn full_configuration(detection: &Detection) -> FullConfigurationAssessment {
    let mut result = FullConfigurationAssessment {
        family: detection.family.clone(),
        renderer: None,
        support: Support::Unknown,
        blocked: true,
        warnings: Vec::new(),
        reasons: Vec::new(),
    };
    let Some(client) = detection
        .family
        .as_deref()
        .and_then(|family| registry().clients.iter().find(|client| client.id == family))
    else {
        result
            .reasons
            .push("无法识别客户端，不能选择完整配置格式。".into());
        return result;
    };
    let (Some(renderer), Some(syntax)) = (&client.renderer, &client.syntax) else {
        result
            .reasons
            .push("尚未实现该客户端的完整配置输出。".into());
        return result;
    };
    let assessment = capability(detection, syntax);
    result.support = assessment.support;
    if assessment.support == Support::Unsupported {
        result.reasons.push(assessment.notes);
        return result;
    }
    if assessment.support == Support::Unknown {
        result
            .reasons
            .push("尚未核实该客户端的完整配置输入语法。".into());
        return result;
    }
    result.renderer = Some(renderer.clone());
    result.blocked = false;
    if detection.version.as_deref().and_then(version).is_none() {
        result
            .warnings
            .push("客户端版本未知；仅按家族的配置语法基线输出，未假定为最新版本。".into());
    } else if assessment.basis == "family_baseline" {
        result
            .warnings
            .push("客户端版本不在已核实的配置语法范围内；仅采用家族语法基线。".into());
    }
    if client.id == "shadowrocket" {
        result.warnings.push("Shadowrocket 支持导入 Clash YAML，但其逐字段解析范围未完全公开；原生规则支持不等于 Clash 导入支持。".into());
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    fn detected(family: &str, version: Option<&str>) -> Detection {
        Detection {
            family: Some(family.into()),
            version: version.map(str::to_owned),
            ..Detection::default()
        }
    }

    #[test]
    fn configuration_registry_covers_all_recognized_families() {
        let node_families: BTreeSet<_> = crate::compatibility::registry()
            .clients
            .iter()
            .map(|client| client.id.as_str())
            .collect();
        let config_families: BTreeSet<_> = registry()
            .clients
            .iter()
            .map(|client| client.id.as_str())
            .collect();
        assert_eq!(node_families, config_families);
        assert_eq!(registry().clients.len(), config_families.len());
    }

    #[test]
    fn shadowrocket_earlier_unverified_syntax_does_not_become_node_fallback() {
        let detection = detected("shadowrocket", Some("2.1.59"));
        let old = full_configuration(&detection);
        assert!(old.blocked);
        assert_eq!(old.support, Support::Unknown);
        assert_eq!(old.renderer, None);
        assert_eq!(
            capability(&detection, "syntax.clash_yaml").basis,
            "version_range"
        );
    }

    #[test]
    fn shadowrocket_import_evidence_does_not_imply_full_field_compatibility() {
        for release in ["2.1.60", "2.1.94", "2.1.95", "2.2.92"] {
            let detection = detected("shadowrocket", Some(release));
            let syntax = capability(&detection, "syntax.clash_yaml");
            assert_eq!(syntax.support, Support::Supported, "{release}");
            assert_eq!(syntax.basis, "version_range");
            assert!(
                syntax
                    .evidence
                    .iter()
                    .any(|url| url == "https://t.me/ShadowrocketNews/318")
            );
            let full = full_configuration(&detection);
            assert!(!full.blocked, "{release}");
            assert_eq!(full.renderer.as_deref(), Some("clash_yaml"));
            assert!(!full.warnings.is_empty());
            for key in registry().dimensions["clash_rule_providers"]
                .iter()
                .map(String::as_str)
                .chain([
                    "clash.rule.rule_set",
                    "clash.rule.domain_suffix",
                    "clash.dns.nameserver_policy",
                    "clash.group.select",
                ])
            {
                assert_eq!(
                    capability(&detection, key).support,
                    Support::Unknown,
                    "{release}: {key}"
                );
            }
        }
    }

    #[test]
    fn unknown_build_uses_only_explicit_syntax_baseline() {
        for release in [None, Some("3131"), Some("2.2.92-beta"), Some("99.0.0")] {
            let detection = detected("shadowrocket", release);
            let assessment = full_configuration(&detection);
            assert!(!assessment.blocked);
            assert!(!assessment.warnings.is_empty());
            assert_eq!(
                capability(&detection, "syntax.clash_yaml").basis,
                "family_baseline"
            );
            assert_eq!(
                capability(&detection, "clash.rule.process_name").support,
                Support::Unknown
            );
            assert_eq!(
                capability(&detection, "clash.rule.domain_regex").support,
                Support::Unknown
            );
        }
    }

    #[test]
    fn native_rule_support_is_not_clash_import_support() {
        let detection = detected("shadowrocket", Some("2.2.90"));
        assert_eq!(
            capability(&detection, "native.rule.and").support,
            Support::Supported
        );
        assert_eq!(
            capability(&detection, "clash.rule.and").support,
            Support::Unknown
        );
        assert_eq!(
            capability(&detection, "clash.dns.nameserver_policy").support,
            Support::Unknown
        );
        assert_eq!(
            capability(&detection, "unresearched.feature").support,
            Support::Unknown
        );
    }

    #[test]
    fn no_renderer_is_a_product_limit_not_false_client_unsupported() {
        let result = full_configuration(&detected("sing-box", Some("1.13.0")));
        assert!(result.blocked);
        assert_eq!(result.support, Support::Unknown);
        assert_eq!(result.renderer, None);
        assert!(!result.reasons.is_empty());
    }

    #[test]
    fn stash_mrs_boundary_and_platform_process_rules_are_separate() {
        assert_eq!(
            capability(
                &detected("stash", Some("3.0.0")),
                "clash.provider.domain_mrs"
            )
            .support,
            Support::Unsupported
        );
        assert_eq!(
            capability(
                &detected("stash", Some("3.1.0")),
                "clash.provider.domain_mrs"
            )
            .support,
            Support::Supported
        );
        assert_eq!(
            capability(&detected("stash", Some("3.6.0")), "clash.rule.process_name").support,
            Support::Unsupported
        );
        assert_eq!(
            capability(
                &detected("stash-mac", Some("4.3.0")),
                "clash.rule.process_name"
            )
            .support,
            Support::Supported
        );
        assert_eq!(
            capability(&detected("stash", None), "clash.provider.domain_mrs").support,
            Support::Unknown
        );
    }

    #[test]
    fn precise_core_evidence_does_not_spread_to_wrappers_or_future_versions() {
        for (release, expected) in [
            ("1.15.0", Support::Unsupported),
            ("1.15.1", Support::Unknown),
            ("1.19.17", Support::Supported),
            ("1.19.32", Support::Unknown),
        ] {
            assert_eq!(
                capability(
                    &detected("mihomo", Some(release)),
                    "clash.rule.domain_regex"
                )
                .support,
                expected
            );
        }
        for family in ["flclash", "verge", "cmfa", "clash-legacy"] {
            assert_eq!(
                capability(&detected(family, Some("1.19.17")), "clash.rule.geosite").support,
                Support::Unknown
            );
        }
    }

    #[test]
    fn group_structure_does_not_imply_every_group_type() {
        let mihomo = detected("mihomo", Some("1.19.17"));
        assert_eq!(
            capability(&mihomo, "clash.proxy_groups").support,
            Support::Supported
        );
        assert_eq!(
            capability(&mihomo, "clash.group.relay").support,
            Support::Unsupported
        );
        assert_eq!(
            capability(&mihomo, "clash.group.direct").support,
            Support::Unsupported
        );
        assert_eq!(
            capability(&detected("clash-core", Some("1.18.0")), "clash.group.relay").support,
            Support::Supported
        );
        assert_eq!(
            capability(
                &detected("shadowrocket", Some("2.2.90")),
                "clash.group.select"
            )
            .support,
            Support::Unknown
        );
    }

    #[test]
    fn registry_ranges_have_evidence_and_no_conflicting_overlap() {
        let valid_keys: BTreeSet<_> = registry()
            .dimensions
            .values()
            .flatten()
            .map(String::as_str)
            .collect();
        for profile in registry().profiles.values() {
            for (key, entry) in &profile.capabilities {
                assert!(
                    valid_keys.contains(key.as_str()),
                    "undeclared capability {key}"
                );
                assert!(!entry.notes.is_empty());
                if entry.support != Support::Unknown {
                    assert!(!entry.evidence.is_empty(), "missing evidence for {key}");
                }
                assert!(entry.evidence.iter().all(|url| url.starts_with("https://")));
            }
        }
        for client in &registry().clients {
            if let Some(profile) = &client.baseline {
                assert!(registry().profiles.contains_key(profile));
            }
            for (index, range) in client.ranges.iter().enumerate() {
                let min = version(&range.min).expect("valid minimum version");
                let max = version(&range.max).expect("valid maximum version");
                assert!(min <= max);
                let profile = registry()
                    .profiles
                    .get(&range.profile)
                    .expect("known profile");
                for other in client.ranges.iter().skip(index + 1) {
                    if min <= version(&other.max).unwrap() && version(&other.min).unwrap() <= max {
                        for key in registry().profiles[&other.profile].capabilities.keys() {
                            assert!(
                                !profile.capabilities.contains_key(key),
                                "overlapping {key} for {}",
                                client.id
                            );
                        }
                    }
                }
            }
        }
    }
}
