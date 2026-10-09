//! Synthetic configurations for the standalone Mihomo compatibility regression.
use camofy::{compatibility::Detection, config_compat, engine, node_filter};
use serde_json::json;

fn main() -> anyhow::Result<()> {
    let prefix = "proxies:\n- {name: modern, type: mieru, server: proxy.example, port: 443, username: example, password: example}\n- {name: legacy, type: ss, server: proxy.example, port: 443, cipher: aes-128-gcm, password: example}\n";
    let cases = [
        (
            "dynamic_empty",
            "proxy-groups: [{name: Choice, type: select, include-all-proxies: true, filter: '^modern$'}]\nrules: ['MATCH,Choice']\n",
        ),
        (
            "explicit_excluded_empty",
            "proxy-groups: [{name: Choice, type: select, proxies: [modern, legacy], exclude-filter: '^legacy$'}]\nrules: ['MATCH,Choice']\n",
        ),
        (
            "empty_provider",
            "proxy-providers: {pool: {type: inline, payload: [{name: only, type: mieru, server: proxy.example, port: 443, username: example, password: example}]}}\nproxy-groups: [{name: Choice, type: select, use: [pool]}]\nrules: ['MATCH,Choice']\n",
        ),
        (
            "provider_dialer_override",
            "proxy-providers: {pool: {type: inline, override: {dialer-proxy: modern}, payload: [{name: member, type: ss, server: proxy.example, port: 443, cipher: aes-128-gcm, password: example}]}}\nproxy-groups: [{name: Choice, type: select, use: [pool]}]\nrules: ['MATCH,Choice']\n",
        ),
        (
            "legacy_dynamic_empty",
            "proxy-groups: [{name: Choice, type: select, include-all-proxies: true, filter: '^missing$'}]\nrules: ['MATCH,Choice']\n",
        ),
    ];
    let mut fixtures = vec![];
    for (name, suffix) in cases {
        let mut input = engine::parse(&format!("{prefix}{suffix}"))?;
        if name == "legacy_dynamic_empty" {
            input["proxies"].as_sequence_mut().unwrap().remove(0);
        }
        let policy = node_filter::Policy {
            auto: false,
            exclude_types: if name == "legacy_dynamic_empty" {
                vec![]
            } else {
                vec!["mieru".into()]
            },
        };
        let filtered = node_filter::apply(&input, &policy, Detection::default())?;
        anyhow::ensure!(filtered.report.blocked_groups == ["Choice"]);
        let mut config = filtered.config;
        config["geo-auto-update"] = false.into();
        config["profile"] = serde_yaml::from_str("store-selected: false\nstore-fake-ip: false")?;
        config["dns"] = serde_yaml::from_str("enable: false")?;
        fixtures.push(
            json!({"name":name, "config":engine::mihomo(&config, false)?, "expected":["REJECT"]}),
        );
    }
    let source = engine::parse(
        r#"
mode: rule
log-level: silent
allow-lan: false
bind-address: 127.0.0.1
geo-auto-update: false
ipv6: false
find-process-mode: off
profile: {store-selected: false, store-fake-ip: false}
dns:
  enable: true
  ipv6: false
  use-hosts: false
  use-system-hosts: false
  enhanced-mode: redir-host
  default-nameserver: ['127.0.0.1:__DNS_PORT__']
  nameserver: ['udp://127.0.0.1:__DNS_PORT__']
proxy-groups:
  - {name: First, type: select, proxies: [DIRECT]}
  - {name: Nested, type: select, proxies: [DIRECT]}
  - {name: IpNoResolve, type: select, proxies: [DIRECT]}
  - {name: Fallback, type: select, proxies: [DIRECT]}
rule-providers:
  sites:
    type: inline
    behavior: classical
    payload:
      - DOMAIN,first.example
      - DOMAIN,site.example
      - DOMAIN,excluded.example
      - DOMAIN-SUFFIX,suffix.example
      - DOMAIN-REGEX,^node[0-9]{1,2}\.regex\.example$
  networks:
    type: inline
    behavior: ipcidr
    payload: [127.0.0.0/8]
rules:
  - DOMAIN,first.example,First
  - AND,((RULE-SET,sites),(NOT,((DOMAIN,excluded.example)))),Nested
  - RULE-SET,networks,IpNoResolve,no-resolve
  - MATCH,Fallback
"#,
    )?;
    let compiled = config_compat::compile(
        &source,
        config_compat::Target::Shadowrocket,
        &config_compat::ResolvedSources::new(),
    )?;
    fixtures.push(json!({
        "name": "rule_compilation_equivalence", "kind": "rule_equivalence",
        "source": serde_yaml::to_string(&source)?, "compiled": serde_yaml::to_string(&compiled.config)?,
        "requests": [
            {"host":"first.example", "policy":"First"},
            {"host":"site.example", "policy":"Nested"},
            {"host":"a.suffix.example", "policy":"Nested"},
            {"host":"node42.regex.example", "policy":"Nested"},
            {"host":"excluded.example", "policy":"Fallback"},
            {"host":"unmatched.example", "policy":"Fallback"},
            {"host":"127.0.0.1", "policy":"IpNoResolve"},
        ]
    }));
    println!("{}", serde_json::to_string(&fixtures)?);
    Ok(())
}
