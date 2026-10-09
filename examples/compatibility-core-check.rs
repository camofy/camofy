//! Synthetic configurations for the standalone Mihomo compatibility regression.
use camofy::{compatibility::Detection, engine, node_filter};
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
    println!("{}", serde_json::to_string(&fixtures)?);
    Ok(())
}
