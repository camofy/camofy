//! Pure, bounded configuration compilation. Resource fetching and persistence
//! belong to the caller; this module never performs network or filesystem I/O.
#[cfg(test)]
mod tests;
use anyhow::{Result, bail, ensure};
use serde::{Deserialize, Serialize};
use serde_yaml::{Mapping, Value};
use std::collections::{BTreeMap, BTreeSet};

const MAX_DEPTH: usize = 24;
const MAX_RULES: usize = 250_000;
const MAX_OUTPUT_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Shadowrocket,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Geosite,
    RuleProvider,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SourceFormat {
    Text,
    Yaml,
    Dat,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RuleBehavior {
    Classical,
    Domain,
    Ipcidr,
}

/// These fields can contain private source locations. Never log this structure.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SourceRequest {
    pub key: String,
    pub kind: SourceKind,
    pub url: Option<String>,
    pub format: SourceFormat,
    pub behavior: RuleBehavior,
    pub geosite_tag: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DependencyPlan {
    pub sources: Vec<SourceRequest>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceContent {
    Text(String),
    /// Policy-free classical rules, e.g. DOMAIN-SUFFIX,example.org. A DAT
    /// decoder must retain Full/RootDomain/Plain/Regex distinctions here.
    Rules(Vec<String>),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResolvedSource {
    pub content: SourceContent,
    pub hash: String,
    pub kind: SourceKind,
}

pub type ResolvedSources = BTreeMap<String, ResolvedSource>;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Diagnostic {
    pub code: String,
    pub message: String,
    pub context: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CompilationStats {
    pub input_rules: usize,
    pub output_rules: usize,
    pub expanded_geosite: usize,
    pub expanded_rule_sets: usize,
    pub expanded_dns_selectors: usize,
    pub resolved_sources: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Compilation {
    pub config: Value,
    pub stats: CompilationStats,
    pub diagnostics: Vec<Diagnostic>,
    /// No URLs or names: safe provenance summaries for the response metadata.
    pub source_hashes: Vec<String>,
}

#[derive(Clone, Debug)]
enum Expr {
    Atom {
        kind: String,
        value: String,
        no_resolve: bool,
    },
    All(Vec<Expr>),
    Any(Vec<Expr>),
    Not(Box<Expr>),
    True,
    False,
}

#[derive(Clone)]
struct Rule {
    expr: Expr,
    policy: String,
    no_resolve: bool,
}

fn split_top(value: &str) -> Result<Vec<&str>> {
    let mut depth = 0usize;
    let mut start = 0;
    let mut result = Vec::new();
    let mut escaped = false;
    let mut character_class = false;
    for (index, ch) in value.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '[' {
            character_class = true;
        }
        if ch == ']' {
            character_class = false;
        }
        if character_class {
            continue;
        }
        match ch {
            '(' => {
                depth += 1;
                ensure!(depth <= MAX_DEPTH, "rule nesting limit exceeded");
            }
            ')' => {
                ensure!(depth > 0, "invalid rule parentheses");
                depth -= 1;
            }
            ',' if depth == 0 => {
                result.push(value[start..index].trim());
                start = index + 1;
            }
            _ => {}
        }
    }
    ensure!(depth == 0, "invalid rule parentheses");
    result.push(value[start..].trim());
    Ok(result)
}

fn unwrap_parens(value: &str) -> Result<&str> {
    let value = value.trim();
    ensure!(
        value.starts_with('(') && value.ends_with(')'),
        "invalid logical rule operand"
    );
    Ok(&value[1..value.len() - 1])
}

fn parse_expr(value: &str, depth: usize) -> Result<Expr> {
    ensure!(
        depth <= MAX_DEPTH && value.len() <= 64 * 1024,
        "rule complexity limit exceeded"
    );
    // Mihomo rules/common/base.go treats regex payloads as the entire remainder;
    // commas in quantifiers and character classes are not option separators.
    if let Some((kind, payload)) = value.split_once(',')
        && is_regex_rule(&kind.trim().to_ascii_uppercase())
    {
        ensure!(
            !payload.trim().is_empty() && !payload.chars().any(char::is_control),
            "invalid regex rule payload"
        );
        return Ok(Expr::Atom {
            kind: kind.trim().to_ascii_uppercase(),
            value: payload.trim().into(),
            no_resolve: false,
        });
    }
    let parts = split_top(value)?;
    let normalized_kind = parts[0].to_ascii_uppercase();
    let kind = normalized_kind.as_str();
    ensure!(
        !kind.is_empty()
            && kind
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b == b'-' || b.is_ascii_digit()),
        "invalid rule type"
    );
    if matches!(kind, "AND" | "OR" | "NOT") {
        ensure!(parts.len() == 2, "invalid logical rule shape");
        let operands = split_top(unwrap_parens(parts[1])?)?;
        ensure!(
            !operands.is_empty() && operands.iter().all(|p| !p.is_empty()),
            "empty logical rule"
        );
        let children = operands
            .into_iter()
            .map(|p| parse_expr(unwrap_parens(p)?, depth + 1))
            .collect::<Result<Vec<_>>>()?;
        return match kind {
            "AND" => Ok(Expr::All(children)),
            "OR" => Ok(Expr::Any(children)),
            _ => {
                ensure!(children.len() == 1, "NOT requires one operand");
                Ok(Expr::Not(Box::new(children.into_iter().next().unwrap())))
            }
        };
    }
    ensure!(
        !matches!(kind, "SUB-RULE" | "SCRIPT"),
        "rule control flow cannot be compiled for this target"
    );
    if matches!(kind, "MATCH" | "FINAL") {
        ensure!(parts.len() == 1, "invalid final rule");
        return Ok(Expr::True);
    }
    ensure!(
        parts.len() == 2 || (parts.len() == 3 && parts[2] == "no-resolve"),
        "invalid rule fields or unsupported option"
    );
    ensure!(
        !parts[1].is_empty() && !parts[1].chars().any(char::is_control),
        "invalid empty rule value"
    );
    let kind = if kind == "IP6-CIDR" { "IP-CIDR6" } else { kind };
    Ok(Expr::Atom {
        kind: kind.into(),
        value: parts[1].into(),
        no_resolve: parts.len() == 3,
    })
}

fn parse_rule(value: &str) -> Result<Rule> {
    ensure!(value.len() <= 64 * 1024, "rule complexity limit exceeded");
    let normalized_kind = value
        .split(',')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_uppercase();
    let kind = normalized_kind.as_str();
    if is_regex_rule(kind) {
        let (expr, policy) = value
            .rsplit_once(',')
            .ok_or_else(|| anyhow::anyhow!("rule requires a policy"))?;
        ensure!(
            !policy.trim().is_empty() && !policy.chars().any(char::is_control),
            "invalid rule policy"
        );
        return Ok(Rule {
            expr: parse_expr(expr, 0)?,
            policy: policy.trim().into(),
            no_resolve: false,
        });
    }
    let mut parts = split_top(value)?;
    let no_resolve = parts.len() >= 4
        && !matches!(kind, "AND" | "OR" | "NOT" | "MATCH" | "FINAL")
        && parts.last() == Some(&"no-resolve");
    if no_resolve {
        parts.pop();
    }
    ensure!(parts.len() >= 2, "rule requires a policy");
    let policy = parts.pop().unwrap();
    ensure!(
        !policy.is_empty() && !policy.chars().any(char::is_control),
        "invalid rule policy"
    );
    Ok(Rule {
        expr: parse_expr(&parts.join(","), 0)?,
        policy: policy.into(),
        no_resolve,
    })
}

fn is_regex_rule(kind: &str) -> bool {
    matches!(
        kind,
        "DOMAIN-REGEX" | "PROCESS-NAME-REGEX" | "PROCESS-PATH-REGEX"
    )
}

fn provider<'a>(config: &'a Value, name: &str) -> Result<&'a Value> {
    config
        .get("rule-providers")
        .and_then(Value::as_mapping)
        .and_then(|m| m.get(name))
        .ok_or_else(|| anyhow::anyhow!("referenced rule provider is missing"))
}

fn provider_schema(value: &Value) -> Result<(RuleBehavior, SourceFormat)> {
    ensure!(value.is_mapping(), "invalid rule provider definition");
    let behavior = match value.get("behavior").and_then(Value::as_str) {
        Some("classical") => RuleBehavior::Classical,
        Some("domain") => RuleBehavior::Domain,
        Some("ipcidr") => RuleBehavior::Ipcidr,
        _ => bail!("unsupported rule provider behavior"),
    };
    let format = match value.get("format") {
        None | Some(Value::Null) => SourceFormat::Yaml,
        Some(Value::String(format)) if format == "yaml" => SourceFormat::Yaml,
        Some(Value::String(format)) if format == "text" => SourceFormat::Text,
        _ => bail!("unsupported rule provider format"),
    };
    Ok((behavior, format))
}

fn request(config: &Value, kind: &str, value: &str) -> Result<SourceRequest> {
    if kind == "GEOSITE" {
        let tag = value.to_ascii_lowercase();
        ensure!(
            !tag.starts_with('!')
                && tag.len() <= 256
                && tag
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-!@".contains(&b)),
            "unsupported geosite selector syntax"
        );
        let custom = config.get("geox-url").and_then(|g| g.get("geosite"));
        let url = match custom {
            None | Some(Value::Null) => None,
            Some(Value::String(url)) if !url.is_empty() => Some(url.clone()),
            _ => bail!("invalid geosite database URL"),
        };
        return Ok(SourceRequest {
            key: format!("geosite:{tag}"),
            kind: SourceKind::Geosite,
            format: if url.is_some() {
                SourceFormat::Dat
            } else {
                SourceFormat::Text
            },
            url,
            behavior: RuleBehavior::Classical,
            geosite_tag: Some(tag),
        });
    }
    let definition = provider(config, value)?;
    let (behavior, format) = provider_schema(definition)?;
    ensure!(
        definition.get("type").and_then(Value::as_str) == Some("http"),
        "only inline and HTTP rule providers can be resolved"
    );
    let url = definition
        .get("url")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("HTTP rule provider has no URL"))?;
    Ok(SourceRequest {
        key: format!("provider:{value}"),
        kind: SourceKind::RuleProvider,
        url: Some(url.into()),
        format,
        behavior,
        geosite_tag: None,
    })
}

fn payload_strings(value: &Value) -> Result<Vec<String>> {
    let list = value
        .as_sequence()
        .ok_or_else(|| anyhow::anyhow!("rule provider payload must be a list"))?;
    ensure!(list.len() <= MAX_RULES, "rule source entry limit exceeded");
    let mut bytes = 0usize;
    list.iter()
        .map(|v| {
            let text = v
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("rule provider payload item must be a string"))?;
            bytes = bytes.saturating_add(text.len());
            ensure!(
                text.len() <= 64 * 1024 && bytes <= MAX_OUTPUT_BYTES,
                "rule source byte limit exceeded"
            );
            Ok(text.to_owned())
        })
        .collect()
}

fn source_lines(source: &ResolvedSource, request: &SourceRequest) -> Result<Vec<String>> {
    ensure!(
        source.kind == request.kind
            && source.hash.len() == 64
            && source.hash.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid resolved rule source metadata"
    );
    match &source.content {
        SourceContent::Rules(rules) => {
            ensure!(rules.len() <= MAX_RULES, "rule source entry limit exceeded");
            ensure!(
                rules.iter().all(|r| r.len() <= 64 * 1024)
                    && rules.iter().map(String::len).sum::<usize>() <= MAX_OUTPUT_BYTES,
                "rule source byte limit exceeded"
            );
            Ok(rules.clone())
        }
        SourceContent::Text(text) => {
            ensure!(
                text.len() <= MAX_OUTPUT_BYTES,
                "rule source byte limit exceeded"
            );
            match request.format {
                SourceFormat::Dat => bail!("geosite DAT must be decoded before compilation"),
                SourceFormat::Yaml => {
                    let value: Value = serde_yaml::from_str(text)
                        .map_err(|_| anyhow::anyhow!("invalid rule provider YAML"))?;
                    payload_strings(value.get("payload").unwrap_or(&value))
                }
                SourceFormat::Text => Ok(text
                    .trim_start_matches('\u{feff}')
                    .lines()
                    .map(str::trim)
                    .filter(|s| !s.is_empty() && !s.starts_with('#') && !s.starts_with("//"))
                    .map(str::to_owned)
                    .collect()),
            }
        }
    }
}

fn payload_expr(line: &str, behavior: RuleBehavior) -> Result<Expr> {
    match behavior {
        RuleBehavior::Classical => parse_expr(line.trim(), 0),
        RuleBehavior::Ipcidr => {
            let (address, prefix) = line
                .split_once('/')
                .ok_or_else(|| anyhow::anyhow!("invalid IP rule provider entry"))?;
            let address: std::net::IpAddr = address
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid IP rule provider entry"))?;
            let prefix: u8 = prefix
                .parse()
                .map_err(|_| anyhow::anyhow!("invalid IP rule provider prefix"))?;
            ensure!(
                prefix <= if address.is_ipv4() { 32 } else { 128 },
                "invalid IP rule provider prefix"
            );
            Ok(Expr::Atom {
                kind: if address.is_ipv4() {
                    "IP-CIDR"
                } else {
                    "IP-CIDR6"
                }
                .into(),
                value: line.into(),
                no_resolve: false,
            })
        }
        RuleBehavior::Domain => {
            ensure!(
                !line.is_empty()
                    && line.len() <= 64 * 1024
                    && !line
                        .chars()
                        .any(|c| c.is_control() || c.is_whitespace() || c == ',' || c == '/'),
                "invalid domain rule provider entry"
            );
            let normalized = line.to_lowercase();
            let line = normalized.as_str();
            let labels: Vec<_> = line.split('.').collect();
            ensure!(
                labels.iter().enumerate().all(|(index, label)| {
                    (!label.is_empty() || index == 0 && labels.len() > 1)
                        && (!label.contains('*') || *label == "*")
                        && (!label.contains('+') || *label == "+" && index == 0 && labels.len() > 1)
                }),
                "unsupported domain rule provider pattern"
            );
            let (kind, value) =
                if let Some(suffix) = line.strip_prefix("+.").filter(|s| !s.contains('*')) {
                    ("DOMAIN-SUFFIX", suffix.to_owned())
                } else if line.contains('*') || line.starts_with('.') {
                    // Mihomo component/trie/domain.go: '*' matches one label,
                    // a leading '.' one or more labels, and '+.' zero or more.
                    let mut pattern = String::from("(?i)^");
                    let value = if let Some(value) = line.strip_prefix("+.") {
                        pattern.push_str("(?:[^.]+\\.)*");
                        value
                    } else if let Some(value) = line.strip_prefix('.') {
                        pattern.push_str("(?:[^.]+\\.)+");
                        value
                    } else {
                        line
                    };
                    for ch in value.chars() {
                        if ch == '*' {
                            pattern.push_str("[^.]+");
                        } else {
                            pattern.push_str(&regex::escape(&ch.to_string()));
                        }
                    }
                    pattern.push('$');
                    ("DOMAIN-REGEX", pattern)
                } else {
                    ensure!(
                        !line.contains('+'),
                        "unsupported domain rule provider pattern"
                    );
                    ("DOMAIN", line.to_owned())
                };
            Ok(Expr::Atom {
                kind: kind.into(),
                value,
                no_resolve: false,
            })
        }
    }
}

struct Compiler<'a> {
    config: &'a Value,
    resolved: &'a ResolvedSources,
    sources: BTreeMap<String, SourceRequest>,
    active: BTreeSet<String>,
    cache: BTreeMap<String, Expr>,
    planning: bool,
    stats: CompilationStats,
    diagnostics: Vec<Diagnostic>,
    output_bytes: usize,
    unknown: BTreeSet<String>,
    work_nodes: usize,
    work_bytes: usize,
}

impl<'a> Compiler<'a> {
    fn new(config: &'a Value, resolved: &'a ResolvedSources, planning: bool) -> Self {
        Self {
            config,
            resolved,
            sources: BTreeMap::new(),
            active: BTreeSet::new(),
            cache: BTreeMap::new(),
            planning,
            stats: CompilationStats::default(),
            diagnostics: Vec::new(),
            output_bytes: 0,
            unknown: BTreeSet::new(),
            work_nodes: 0,
            work_bytes: 0,
        }
    }

    fn charge(&mut self, nodes: usize, bytes: usize) -> Result<()> {
        self.work_nodes = self.work_nodes.saturating_add(nodes);
        self.work_bytes = self.work_bytes.saturating_add(bytes);
        ensure!(
            self.work_nodes <= MAX_RULES * 4 && self.work_bytes <= MAX_OUTPUT_BYTES * 4,
            "rule expansion work limit exceeded"
        );
        Ok(())
    }

    fn expand(&mut self, expr: Expr, depth: usize) -> Result<Expr> {
        ensure!(depth <= MAX_DEPTH, "rule expansion depth limit exceeded");
        self.charge(
            1,
            if let Expr::Atom { kind, value, .. } = &expr {
                kind.len() + value.len()
            } else {
                0
            },
        )?;
        match expr {
            Expr::Atom {
                kind,
                value,
                no_resolve,
            } if matches!(kind.as_str(), "GEOSITE" | "RULE-SET") => {
                if kind == "GEOSITE" {
                    self.stats.expanded_geosite += 1;
                } else {
                    self.stats.expanded_rule_sets += 1;
                }
                let key = if kind == "GEOSITE" {
                    format!("geosite:{}", value.to_ascii_lowercase())
                } else {
                    format!("provider:{value}")
                };
                if let Some(cached) = self.cache.get(&key) {
                    let (nodes, bytes) = expr_size(cached);
                    self.charge(nodes, bytes)?;
                    return Ok(with_no_resolve(self.cache[&key].clone(), no_resolve));
                }
                ensure!(
                    self.active.insert(key.clone()),
                    "cyclic rule source reference"
                );
                let inline = if kind == "RULE-SET" {
                    let definition = provider(self.config, &value)?;
                    let (behavior, _) = provider_schema(definition)?;
                    if definition.get("type").and_then(Value::as_str) == Some("inline") {
                        Some((
                            payload_strings(definition.get("payload").ok_or_else(|| {
                                anyhow::anyhow!("inline rule provider has no payload")
                            })?)?,
                            behavior,
                        ))
                    } else {
                        None
                    }
                } else {
                    None
                };
                let payload = if let Some(inline) = inline {
                    inline
                } else {
                    let request = request(self.config, &kind, &value)?;
                    self.sources.insert(key.clone(), request.clone());
                    if let Some(source) = self.resolved.get(&key) {
                        let behavior = if matches!(source.content, SourceContent::Rules(_)) {
                            RuleBehavior::Classical
                        } else {
                            request.behavior
                        };
                        (source_lines(source, &request)?, behavior)
                    } else if self.planning {
                        self.active.remove(&key);
                        return Ok(Expr::False);
                    } else {
                        bail!("required rule source snapshot is missing");
                    }
                };
                ensure!(
                    payload.0.len() <= MAX_RULES,
                    "rule source entry limit exceeded"
                );
                let mut children = Vec::new();
                for line in payload.0 {
                    children.push(self.expand(payload_expr(&line, payload.1)?, depth + 1)?);
                }
                let expanded = simplify(Expr::Any(children));
                self.active.remove(&key);
                let (nodes, bytes) = expr_size(&expanded);
                self.charge(nodes, bytes)?;
                self.cache.insert(key, expanded.clone());
                Ok(with_no_resolve(expanded, no_resolve))
            }
            Expr::All(children) => Ok(simplify(Expr::All(
                children
                    .into_iter()
                    .map(|e| self.expand(e, depth + 1))
                    .collect::<Result<_>>()?,
            ))),
            Expr::Any(children) => Ok(simplify(Expr::Any(
                children
                    .into_iter()
                    .map(|e| self.expand(e, depth + 1))
                    .collect::<Result<_>>()?,
            ))),
            Expr::Not(child) => Ok(simplify(Expr::Not(Box::new(
                self.expand(*child, depth + 1)?,
            )))),
            other => Ok(other),
        }
    }

    fn check_atoms(&mut self, expr: &Expr, context: &str) {
        match expr {
            Expr::Atom { kind, .. }
                if ![
                    "DOMAIN",
                    "DOMAIN-SUFFIX",
                    "DOMAIN-KEYWORD",
                    "IP-CIDR",
                    "IP-CIDR6",
                    "GEOIP",
                    "DST-PORT",
                ]
                .contains(&kind.as_str())
                    && self.unknown.insert(kind.clone()) =>
            {
                self.diagnostics.push(Diagnostic {
                    code: "unverified_rule_type".into(),
                    message:
                        "A rule type has no verified target capability; its syntax was preserved."
                            .into(),
                    context: context.into(),
                });
            }
            Expr::All(items) | Expr::Any(items) => {
                for item in items {
                    self.check_atoms(item, context);
                }
            }
            Expr::Not(item) => self.check_atoms(item, context),
            _ => {}
        }
    }

    fn run_rules(&mut self) -> Result<Option<Value>> {
        let Some(rules) = self.config.get("rules").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        let rules = rules
            .as_sequence()
            .ok_or_else(|| anyhow::anyhow!("rules must be a list"))?;
        ensure!(rules.len() <= MAX_RULES, "rule count limit exceeded");
        self.stats.input_rules = rules.len();
        let mut output = Vec::new();
        for (index, raw) in rules.iter().enumerate() {
            let result = (|| {
                let raw = raw
                    .as_str()
                    .ok_or_else(|| anyhow::anyhow!("rule must be a string"))?;
                let rule = parse_rule(raw)?;
                let expr = self.expand(with_no_resolve(rule.expr, rule.no_resolve), 0)?;
                self.check_atoms(&expr, &format!("rules[{index}]"));
                let items = if let Expr::Any(items) = expr {
                    items
                } else {
                    vec![expr]
                };
                for expr in items {
                    if matches!(expr, Expr::False) {
                        continue;
                    }
                    let mut line = match &expr {
                        Expr::Atom { kind, value, .. } => format!("{kind},{value},{}", rule.policy),
                        other => format!("{},{}", render(other), rule.policy),
                    };
                    if matches!(
                        expr,
                        Expr::Atom {
                            no_resolve: true,
                            ..
                        }
                    ) {
                        line.push_str(",no-resolve");
                    }
                    ensure!(
                        line.len() <= 64 * 1024,
                        "expanded rule expression limit exceeded"
                    );
                    self.output_bytes += line.len();
                    ensure!(
                        output.len() < MAX_RULES && self.output_bytes <= MAX_OUTPUT_BYTES,
                        "expanded rule size limit exceeded"
                    );
                    output.push(Value::String(line));
                }
                Ok(())
            })();
            result.map_err(|e: anyhow::Error| anyhow::anyhow!("rules[{index}]: {e}"))?;
        }
        self.stats.output_rules = output.len();
        Ok(Some(Value::Sequence(output)))
    }

    fn dns_selectors(&mut self, value: &str) -> Result<Option<Vec<String>>> {
        let lower = value.to_ascii_lowercase();
        let (kind, selectors) = if lower.starts_with("geosite:") {
            ("GEOSITE", &value[8..])
        } else if lower.starts_with("rule-set:") {
            ("RULE-SET", &value[9..])
        } else {
            return Ok(None);
        };
        let mut output = Vec::new();
        for selector in selectors.split(',') {
            ensure!(!selector.trim().is_empty(), "empty DNS rule selector");
            let expanded = self.expand(
                Expr::Atom {
                    kind: kind.into(),
                    value: selector.trim().into(),
                    no_resolve: false,
                },
                0,
            )?;
            if self.planning {
                continue;
            }
            domain_selectors(expanded, &mut output)?;
        }
        self.stats.expanded_dns_selectors += 1;
        ensure!(
            output.len() <= MAX_RULES,
            "DNS selector expansion limit exceeded"
        );
        Ok(Some(output))
    }

    fn run_dns(&mut self) -> Result<Option<Value>> {
        let Some(dns) = self.config.get("dns").filter(|v| !v.is_null()) else {
            return Ok(None);
        };
        ensure!(dns.is_mapping(), "dns must be a mapping");
        let mut output = dns.clone();
        for field in ["nameserver-policy", "proxy-server-nameserver-policy"] {
            if let Some(policy) = dns.get(field).filter(|v| !v.is_null()) {
                let policy = policy
                    .as_mapping()
                    .ok_or_else(|| anyhow::anyhow!("DNS nameserver-policy must be a mapping"))?;
                let mut expanded = Mapping::new();
                for (index, (key, servers)) in policy.iter().enumerate() {
                    let key = key
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("DNS selector must be a string"))?;
                    let selectors = self
                        .dns_selectors(key)
                        .map_err(|e| anyhow::anyhow!("dns.{field}[{index}]: {e}"))?;
                    for selector in selectors.unwrap_or_else(|| vec![key.into()]) {
                        let key = Value::String(selector);
                        ensure!(
                            expanded
                                .get(&key)
                                .is_none_or(|existing| existing == servers),
                            "conflicting DNS selector expansions"
                        );
                        expanded.insert(key, servers.clone());
                        ensure!(
                            expanded.len() <= MAX_RULES,
                            "DNS selector expansion limit exceeded"
                        );
                    }
                }
                output[field] = Value::Mapping(expanded);
            }
        }
        for field in ["fake-ip-filter"] {
            if let Some(filters) = dns.get(field).filter(|v| !v.is_null()) {
                let filters = filters
                    .as_sequence()
                    .ok_or_else(|| anyhow::anyhow!("DNS filter must be a list"))?;
                if dns.get("fake-ip-filter-mode").and_then(Value::as_str) == Some("rule") {
                    ensure!(
                        !contains_rule_reference(&Value::Sequence(filters.clone())),
                        "DNS fake-IP rule-mode references cannot be compiled for this target"
                    );
                    continue;
                }
                let mut expanded = Vec::new();
                for (index, filter) in filters.iter().enumerate() {
                    let filter = filter
                        .as_str()
                        .ok_or_else(|| anyhow::anyhow!("DNS filter item must be a string"))?;
                    let selectors = self
                        .dns_selectors(filter)
                        .map_err(|e| anyhow::anyhow!("dns.{field}[{index}]: {e}"))?;
                    expanded.extend(
                        selectors
                            .unwrap_or_else(|| vec![filter.into()])
                            .into_iter()
                            .map(Value::String),
                    );
                    ensure!(
                        expanded.len() <= MAX_RULES,
                        "DNS filter expansion limit exceeded"
                    );
                }
                output[field] = Value::Sequence(expanded);
            }
        }
        if let Some(fallback) = dns.get("fallback-filter").filter(|v| !v.is_null()) {
            ensure!(
                fallback.is_mapping(),
                "DNS fallback-filter must be a mapping"
            );
            if let Some(tags) = fallback.get("geosite").filter(|v| !v.is_null()) {
                let tags = tags
                    .as_sequence()
                    .ok_or_else(|| anyhow::anyhow!("DNS fallback geosite must be a list"))?;
                let mut domains = match fallback.get("domain").filter(|v| !v.is_null()) {
                    Some(domain) => payload_strings(domain)?
                        .into_iter()
                        .map(Value::String)
                        .collect::<Vec<_>>(),
                    None => Vec::new(),
                };
                for (index, tag) in tags.iter().enumerate() {
                    let tag = tag.as_str().ok_or_else(|| {
                        anyhow::anyhow!("DNS fallback geosite selector must be a string")
                    })?;
                    let selectors = self.dns_selectors(&format!("geosite:{tag}")).map_err(|e| {
                        anyhow::anyhow!("dns.fallback-filter.geosite[{index}]: {e}")
                    })?;
                    domains.extend(selectors.unwrap_or_default().into_iter().map(Value::String));
                    ensure!(
                        domains.len() <= MAX_RULES,
                        "DNS fallback expansion limit exceeded"
                    );
                }
                let fallback = output["fallback-filter"].as_mapping_mut().unwrap();
                fallback.remove("geosite");
                fallback.insert(Value::String("domain".into()), Value::Sequence(domains));
            }
        }
        if self.stats.expanded_dns_selectors > 0 {
            self.diagnostics.push(Diagnostic { code: "unverified_dns_mapping".into(), message: "DNS selectors were expanded without changing their matching scope; target DNS-field support still requires verification.".into(), context: "dns".into() });
        }
        Ok(Some(output))
    }
}

fn expr_size(expr: &Expr) -> (usize, usize) {
    match expr {
        Expr::Atom { kind, value, .. } => (1, kind.len() + value.len()),
        Expr::All(items) | Expr::Any(items) => items
            .iter()
            .map(expr_size)
            .fold((1, 0), |(n, b), (child_n, child_b)| {
                (n + child_n, b + child_b)
            }),
        Expr::Not(item) => {
            let (nodes, bytes) = expr_size(item);
            (nodes + 1, bytes)
        }
        _ => (1, 0),
    }
}

fn contains_rule_reference(value: &Value) -> bool {
    match value {
        Value::String(text) => {
            let lower = text.to_ascii_lowercase();
            lower.starts_with("geosite:")
                || lower.starts_with("rule-set:")
                || lower.starts_with("geosite,")
                || lower.starts_with("rule-set,")
                || lower.contains("(geosite,")
                || lower.contains("(rule-set,")
        }
        Value::Sequence(items) => items.iter().any(contains_rule_reference),
        Value::Mapping(items) => items
            .iter()
            .any(|(key, value)| contains_rule_reference(key) || contains_rule_reference(value)),
        Value::Tagged(tagged) => contains_rule_reference(&tagged.value),
        _ => false,
    }
}

fn validate_contexts(config: &Value) -> Result<()> {
    if let Some(sub_rules) = config.get("sub-rules").filter(|v| !v.is_null()) {
        ensure!(
            sub_rules.as_mapping().is_some_and(Mapping::is_empty),
            "sub-rule control flow cannot be compiled for this target"
        );
    }
    // Domain matchers in these contexts have different consumers. Leaving a
    // reference after deleting rule-providers would silently change behavior.
    for field in ["sniffer", "hosts", "tun"] {
        ensure!(
            !config.get(field).is_some_and(contains_rule_reference),
            "rule reference in an unsupported configuration context"
        );
    }
    Ok(())
}

fn domain_selectors(expr: Expr, output: &mut Vec<String>) -> Result<()> {
    match expr {
        Expr::Atom {
            kind,
            value,
            no_resolve: false,
        } if kind == "DOMAIN" => output.push(value),
        Expr::Atom {
            kind,
            value,
            no_resolve: false,
        } if kind == "DOMAIN-SUFFIX" => output.push(format!("+.{value}")),
        Expr::Any(items) => {
            for item in items {
                domain_selectors(item, output)?;
            }
        }
        Expr::False => {}
        _ => bail!("DNS rule selector cannot be represented as exact domains or suffixes"),
    }
    Ok(())
}

fn with_no_resolve(expr: Expr, no_resolve: bool) -> Expr {
    if !no_resolve {
        return expr;
    }
    match expr {
        Expr::Atom { ref kind, .. }
            if matches!(
                kind.as_str(),
                "DOMAIN"
                    | "DOMAIN-SUFFIX"
                    | "DOMAIN-KEYWORD"
                    | "DOMAIN-WILDCARD"
                    | "DOMAIN-REGEX"
                    | "PROCESS-NAME"
                    | "PROCESS-PATH"
                    | "PROCESS-NAME-REGEX"
                    | "PROCESS-PATH-REGEX"
                    | "DST-PORT"
                    | "SRC-PORT"
                    | "NETWORK"
                    | "IN-TYPE"
                    | "IN-NAME"
                    | "IN-USER"
            ) =>
        {
            expr
        }
        Expr::Atom {
            kind,
            value,
            no_resolve: _,
        } => Expr::Atom {
            kind,
            value,
            no_resolve: true,
        },
        Expr::All(items) => Expr::All(
            items
                .into_iter()
                .map(|e| with_no_resolve(e, true))
                .collect(),
        ),
        Expr::Any(items) => Expr::Any(
            items
                .into_iter()
                .map(|e| with_no_resolve(e, true))
                .collect(),
        ),
        Expr::Not(item) => Expr::Not(Box::new(with_no_resolve(*item, true))),
        other => other,
    }
}

/// Inspect parsed rule types and options, never policy names or payload text.
/// Malformed rules yield one generic unknown capability for the caller to report.
pub fn rule_capabilities(config: &Value) -> BTreeSet<String> {
    fn visit(expr: &Expr, output: &mut BTreeSet<String>) {
        let kind = match expr {
            Expr::Atom {
                kind, no_resolve, ..
            } => {
                if *no_resolve {
                    output.insert("clash.rule.no_resolve".into());
                }
                kind.to_ascii_lowercase().replace('-', "_")
            }
            Expr::All(items) | Expr::Any(items) => {
                for item in items {
                    visit(item, output);
                }
                if matches!(expr, Expr::All(_)) {
                    "and"
                } else {
                    "or"
                }
                .into()
            }
            Expr::Not(item) => {
                visit(item, output);
                "not".into()
            }
            Expr::True => "match".into(),
            Expr::False => return,
        };
        let kind = match kind.as_str() {
            "ip_cidr" => "ipcidr",
            "ip_cidr6" => "ipcidr6",
            other => other,
        };
        output.insert(format!("clash.rule.{kind}"));
    }
    let mut output = BTreeSet::new();
    let Some(raw) = config.get("rules").filter(|v| !v.is_null()) else {
        return output;
    };
    let Some(rules) = raw.as_sequence() else {
        output.insert("clash.rule.unknown".into());
        return output;
    };
    for rule in rules {
        match rule.as_str().map(parse_rule) {
            Some(Ok(rule)) => {
                if rule.no_resolve {
                    output.insert("clash.rule.no_resolve".into());
                }
                visit(&rule.expr, &mut output);
            }
            _ => {
                output.insert("clash.rule.unknown".into());
            }
        }
    }
    output
}

fn simplify(expr: Expr) -> Expr {
    let is_all = matches!(&expr, Expr::All(_));
    match expr {
        Expr::All(items) | Expr::Any(items) => {
            let mut kept = Vec::new();
            for item in items {
                if matches!((&item, is_all), (Expr::False, true) | (Expr::True, false)) {
                    return if is_all { Expr::False } else { Expr::True };
                }
                if !matches!((&item, is_all), (Expr::True, true) | (Expr::False, false)) {
                    kept.push(item);
                }
            }
            match kept.len() {
                0 => {
                    if is_all {
                        Expr::True
                    } else {
                        Expr::False
                    }
                }
                1 => kept.pop().unwrap(),
                _ => {
                    if is_all {
                        Expr::All(kept)
                    } else {
                        Expr::Any(kept)
                    }
                }
            }
        }
        Expr::Not(item) => match *item {
            Expr::True => Expr::False,
            Expr::False => Expr::True,
            other => Expr::Not(Box::new(other)),
        },
        other => other,
    }
}

fn render(expr: &Expr) -> String {
    match expr {
        Expr::Atom {
            kind,
            value,
            no_resolve,
        } => format!(
            "{kind},{value}{}",
            if *no_resolve { ",no-resolve" } else { "" }
        ),
        Expr::All(items) | Expr::Any(items) => format!(
            "{},({})",
            if matches!(expr, Expr::All(_)) {
                "AND"
            } else {
                "OR"
            },
            items
                .iter()
                .map(|e| format!("({})", render(e)))
                .collect::<Vec<_>>()
                .join(",")
        ),
        Expr::Not(item) => format!("NOT,(({}))", render(item)),
        Expr::True => "MATCH".into(),
        Expr::False => unreachable!("false expressions must be simplified before rendering"),
    }
}

pub fn plan(config: &Value, target: Target) -> Result<DependencyPlan> {
    plan_with_sources(config, target, &ResolvedSources::new())
}

/// Repeat after resolving the returned requests when a classical provider itself
/// refers to another provider or GEOSITE tag. The caller bounds fetch rounds.
pub fn plan_with_sources(
    config: &Value,
    _target: Target,
    resolved: &ResolvedSources,
) -> Result<DependencyPlan> {
    ensure!(config.is_mapping(), "configuration must be a mapping");
    validate_contexts(config)?;
    let mut compiler = Compiler::new(config, resolved, true);
    compiler.run_rules()?;
    compiler.run_dns()?;
    Ok(DependencyPlan {
        sources: compiler.sources.into_values().collect(),
    })
}

pub fn compile(config: &Value, _target: Target, resolved: &ResolvedSources) -> Result<Compilation> {
    ensure!(config.is_mapping(), "configuration must be a mapping");
    validate_contexts(config)?;
    let mut compiler = Compiler::new(config, resolved, false);
    let rules = compiler.run_rules()?;
    let dns = compiler.run_dns()?;
    let mut output = config.clone();
    if let Some(rules) = rules {
        output["rules"] = rules;
    }
    if let Some(dns) = dns {
        output["dns"] = dns;
    }
    ensure!(
        !output.get("dns").is_some_and(contains_rule_reference),
        "rule reference remains in an unsupported DNS context"
    );
    output.as_mapping_mut().unwrap().remove("rule-providers");
    let source_hashes = compiler
        .sources
        .keys()
        .map(|key| resolved[key].hash.clone())
        .collect();
    compiler.stats.resolved_sources = compiler.sources.len();
    ensure!(
        serde_yaml::to_string(&output)
            .map_err(|_| anyhow::anyhow!("compiled configuration cannot be serialized"))?
            .len()
            <= MAX_OUTPUT_BYTES,
        "compiled configuration byte limit exceeded"
    );
    Ok(Compilation {
        config: output,
        stats: compiler.stats,
        diagnostics: compiler.diagnostics,
        source_hashes,
    })
}
