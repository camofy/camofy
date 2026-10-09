use super::*;

#[test]
fn rule_type_case_and_reserved_policy_names_follow_source_parser() {
    let input = config(
        "rules: ['domain,example.org,no-resolve', 'domain-regex,^x{1,2}\\.example$,and', 'match,OR']",
    );
    let result = compile(&input, Target::Shadowrocket, &ResolvedSources::new()).unwrap();
    assert_eq!(
        rules(&result),
        [
            "DOMAIN,example.org,no-resolve",
            "DOMAIN-REGEX,^x{1,2}\\.example$,and",
            "MATCH,OR"
        ]
    );
    assert_eq!(
        rule_capabilities(&input),
        BTreeSet::from(
            [
                "clash.rule.domain",
                "clash.rule.domain_regex",
                "clash.rule.match"
            ]
            .map(str::to_owned)
        )
    );
}

#[test]
fn capability_inspection_ignores_policy_names_and_payloads() {
    let input = config(
        "rules: ['DOMAIN,OR,AND', 'PROCESS-NAME,RULE-SET,NOT', 'AND,((IP-CIDR,192.0.2.0/24,no-resolve),(DOMAIN-KEYWORD,GEOSITE)),route', 'IP6-CIDR,2001:db8::/32,DIRECT', 'MATCH,DOMAIN-REGEX']",
    );
    assert_eq!(
        rule_capabilities(&input),
        BTreeSet::from(
            [
                "clash.rule.domain",
                "clash.rule.process_name",
                "clash.rule.and",
                "clash.rule.ipcidr",
                "clash.rule.no_resolve",
                "clash.rule.domain_keyword",
                "clash.rule.ipcidr6",
                "clash.rule.match",
            ]
            .map(str::to_owned)
        )
    );
    assert_eq!(
        rule_capabilities(&config("rules: ['broken payload']")),
        BTreeSet::from(["clash.rule.unknown".into()])
    );
}

#[test]
fn regex_commas_are_payload_and_source_no_resolve_only_applies_to_ip_rules() {
    let input = config(
        r#"
rule-providers:
  mixed: {type: inline, behavior: classical, payload: ['DOMAIN-REGEX,^node[0-9]{1,2}\.example$', 'IP-CIDR,192.0.2.0/24']}
rules:
  - DOMAIN-REGEX,^node[0-9]{1,2}\.example$,AND
  - RULE-SET,mixed,route,no-resolve
"#,
    );
    let result = compile(&input, Target::Shadowrocket, &ResolvedSources::new()).unwrap();
    assert_eq!(
        rules(&result),
        [
            "DOMAIN-REGEX,^node[0-9]{1,2}\\.example$,AND",
            "DOMAIN-REGEX,^node[0-9]{1,2}\\.example$,route",
            "IP-CIDR,192.0.2.0/24,route,no-resolve"
        ]
    );
    assert_eq!(
        rule_capabilities(&input),
        BTreeSet::from(
            [
                "clash.rule.domain_regex",
                "clash.rule.rule_set",
                "clash.rule.no_resolve"
            ]
            .map(str::to_owned)
        )
    );
}

fn config(text: &str) -> Value {
    serde_yaml::from_str(text).unwrap()
}

fn resolved(key: &str, content: SourceContent) -> ResolvedSources {
    BTreeMap::from([(
        key.into(),
        ResolvedSource {
            content,
            hash: "a".repeat(64),
            kind: if key.starts_with("geosite:") {
                SourceKind::Geosite
            } else {
                SourceKind::RuleProvider
            },
        },
    )])
}

fn rules(result: &Compilation) -> Vec<&str> {
    result.config["rules"]
        .as_sequence()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect()
}

#[test]
fn geosite_preserves_categories_order_nodes_and_groups() {
    let input = config(
        r#"
proxies:
  - {name: relay, type: socks5, server: proxy.example, port: 1080, dialer-proxy: gateway}
  - {name: gateway, type: mieru, server: transport.example, port: 8080, username: sample, password: sample}
proxy-groups:
  - {name: route, type: select, proxies: [relay, gateway]}
rules: ['DOMAIN,first.example,DIRECT', 'GEOSITE,sample,route', 'DOMAIN,last.example,REJECT', 'MATCH,route']
"#,
    );
    let sources = resolved("geosite:sample", SourceContent::Text(
        "# complete source\nDOMAIN,exact.example\nDOMAIN-SUFFIX,suffix.example\nDOMAIN-KEYWORD,example\nDOMAIN-REGEX,(^|\\.)example[0-9]\\.org$\n".into()));
    let result = compile(&input, Target::Shadowrocket, &sources).unwrap();
    assert_eq!(
        rules(&result),
        vec![
            "DOMAIN,first.example,DIRECT",
            "DOMAIN,exact.example,route",
            "DOMAIN-SUFFIX,suffix.example,route",
            "DOMAIN-KEYWORD,example,route",
            "DOMAIN-REGEX,(^|\\.)example[0-9]\\.org$,route",
            "DOMAIN,last.example,REJECT",
            "MATCH,route"
        ]
    );
    assert_eq!(result.config["proxies"], input["proxies"]);
    assert_eq!(result.config["proxy-groups"], input["proxy-groups"]);
    assert_eq!(
        (result.stats.input_rules, result.stats.output_rules),
        (4, 7)
    );
    assert_eq!(result.source_hashes, vec!["a".repeat(64)]);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "unverified_rule_type")
    );
}

#[test]
fn inline_provider_no_resolve_and_nested_logic_are_lossless() {
    let input = config(
        r#"
rule-providers:
  networks: {type: inline, behavior: classical, payload: ['IP-CIDR,192.0.2.0/24,no-resolve']}
  sites: {type: inline, behavior: domain, payload: [a.example, '+.b.example']}
rules:
  - RULE-SET,networks,DIRECT
  - RULE-SET,networks,REJECT,no-resolve
  - AND,((RULE-SET,networks),(DOMAIN,example.org)),route
  - AND,((RULE-SET,sites),(NOT,((DST-PORT,443)))),route
  - NOT,((RULE-SET,sites)),DIRECT
"#,
    );
    let result = compile(&input, Target::Shadowrocket, &ResolvedSources::new()).unwrap();
    assert_eq!(
        rules(&result),
        vec![
            "IP-CIDR,192.0.2.0/24,DIRECT,no-resolve",
            "IP-CIDR,192.0.2.0/24,REJECT,no-resolve",
            "AND,((IP-CIDR,192.0.2.0/24,no-resolve),(DOMAIN,example.org)),route",
            "AND,((OR,((DOMAIN,a.example),(DOMAIN-SUFFIX,b.example))),(NOT,((DST-PORT,443)))),route",
            "NOT,((OR,((DOMAIN,a.example),(DOMAIN-SUFFIX,b.example)))),DIRECT"
        ]
    );
    assert!(result.config.get("rule-providers").is_none());
}

#[test]
fn wildcard_domain_scope_is_preserved() {
    for (pattern, yes, no) in [
        (
            "*.example.org",
            vec!["a.example.org", "B.EXAMPLE.ORG"],
            vec!["example.org", "a.b.example.org"],
        ),
        (
            ".example.org",
            vec!["a.example.org", "a.b.example.org"],
            vec!["example.org", "notexample.org"],
        ),
        (
            "+.*.example.org",
            vec!["a.example.org", "a.b.example.org"],
            vec!["example.org"],
        ),
    ] {
        let Expr::Atom { value, .. } = payload_expr(pattern, RuleBehavior::Domain).unwrap() else {
            panic!()
        };
        let regex = regex::Regex::new(&value).unwrap();
        for host in yes {
            assert!(regex.is_match(host));
        }
        for host in no {
            assert!(!regex.is_match(host));
        }
    }
    for invalid in [
        "a*b.example",
        "example..org",
        "+",
        "example.org.",
        "https://example.org",
    ] {
        assert!(payload_expr(invalid, RuleBehavior::Domain).is_err());
    }
}

#[test]
fn empty_rule_sets_obey_boolean_semantics() {
    let input = config(
        r#"
rule-providers:
  empty: {type: inline, behavior: domain, payload: []}
rules:
  - RULE-SET,empty,REJECT
  - AND,((RULE-SET,empty),(DOMAIN,a.example)),REJECT
  - OR,((RULE-SET,empty),(DOMAIN,b.example)),route
  - NOT,((RULE-SET,empty)),DIRECT
"#,
    );
    let result = compile(&input, Target::Shadowrocket, &ResolvedSources::new()).unwrap();
    assert_eq!(
        rules(&result),
        vec!["DOMAIN,b.example,route", "MATCH,DIRECT"]
    );
}

#[test]
fn remote_formats_respect_provider_behavior() {
    for (format, behavior, payload, expected) in [
        (
            "yaml",
            "domain",
            "payload: ['+.example.org']",
            "DOMAIN-SUFFIX,example.org,DIRECT",
        ),
        (
            "text",
            "domain",
            "\u{feff}# comment\n+.example.org\n",
            "DOMAIN-SUFFIX,example.org,DIRECT",
        ),
        (
            "yaml",
            "ipcidr",
            "payload: ['2001:db8::/32']",
            "IP-CIDR6,2001:db8::/32,DIRECT,no-resolve",
        ),
        (
            "text",
            "classical",
            "// comment\nDOMAIN-KEYWORD,example\n",
            "DOMAIN-KEYWORD,example,DIRECT",
        ),
    ] {
        let input = config(&format!(
            "rule-providers:\n  remote: {{type: http, behavior: {behavior}, format: {format}, url: 'https://rules.example/source'}}\nrules: ['RULE-SET,remote,DIRECT,no-resolve']"
        ));
        let sources = resolved("provider:remote", SourceContent::Text(payload.into()));
        assert_eq!(
            rules(&compile(&input, Target::Shadowrocket, &sources).unwrap()),
            vec![expected]
        );
    }
}

#[test]
fn planning_discovers_nested_references_and_preserves_custom_database() {
    let input = config(
        r#"
geox-url: {geosite: 'https://rules.example/custom.dat'}
rule-providers:
  remote: {type: http, behavior: classical, format: text, url: 'https://rules.example/source'}
rules: ['RULE-SET,remote,DIRECT', 'RULE-SET,remote,REJECT']
"#,
    );
    assert_eq!(plan(&input, Target::Shadowrocket).unwrap().sources.len(), 1);
    let sources = resolved(
        "provider:remote",
        SourceContent::Text("GEOSITE,sample@category\n".into()),
    );
    let next = plan_with_sources(&input, Target::Shadowrocket, &sources).unwrap();
    assert_eq!(next.sources.len(), 2);
    let geosite = next
        .sources
        .iter()
        .find(|s| s.kind == SourceKind::Geosite)
        .unwrap();
    assert_eq!(geosite.format, SourceFormat::Dat);
    assert_eq!(
        geosite.url.as_deref(),
        Some("https://rules.example/custom.dat")
    );
    assert_eq!(geosite.geosite_tag.as_deref(), Some("sample@category"));
    assert!(compile(&input, Target::Shadowrocket, &sources).is_err());
}

#[test]
fn dns_references_expand_in_supported_contexts() {
    let input = config(
        r#"
rule-providers:
  sites: {type: inline, behavior: domain, payload: [exact.example, '+.suffix.example']}
dns:
  nameserver-policy: {'rule-set:sites': ['https://dns.example/query']}
  proxy-server-nameserver-policy: {'GEOSITE:sample': [192.0.2.53]}
  fake-ip-filter: ['+.local.example', 'geosite:sample']
  fallback-filter: {geoip: true, domain: [kept.example], geosite: [sample]}
rules: ['MATCH,DIRECT']
"#,
    );
    let sources = resolved(
        "geosite:sample",
        SourceContent::Rules(vec![
            "DOMAIN,dns.example".into(),
            "DOMAIN-SUFFIX,zone.example".into(),
        ]),
    );
    let result = compile(&input, Target::Shadowrocket, &sources).unwrap();
    let dns = &result.config["dns"];
    assert!(dns["nameserver-policy"].get("exact.example").is_some());
    assert!(dns["nameserver-policy"].get("+.suffix.example").is_some());
    assert!(
        dns["proxy-server-nameserver-policy"]
            .get("+.zone.example")
            .is_some()
    );
    assert_eq!(
        dns["fake-ip-filter"],
        config("['+.local.example', dns.example, '+.zone.example']")
    );
    assert_eq!(
        dns["fallback-filter"]["domain"],
        config("[kept.example, dns.example, '+.zone.example']")
    );
    assert!(dns["fallback-filter"].get("geosite").is_none());
    assert_eq!(result.stats.expanded_dns_selectors, 4);
}

#[test]
fn dns_unrepresentable_values_and_conflicting_policies_fail_privately() {
    for rule in [
        "DOMAIN-KEYWORD,sensitive.example",
        "DOMAIN-REGEX,sensitive[0-9]\\.example$",
    ] {
        let input = config("dns: {fake-ip-filter: ['geosite:sample']}\nrules: ['MATCH,DIRECT']");
        let sources = resolved("geosite:sample", SourceContent::Rules(vec![rule.into()]));
        let error = compile(&input, Target::Shadowrocket, &sources)
            .unwrap_err()
            .to_string();
        assert!(error.contains("cannot be represented"));
        assert!(!error.contains("sensitive"));
    }
    let input = config(
        "dns: {nameserver-policy: {'geosite:sample': [192.0.2.53], dns.example: [192.0.2.54]}}\nrules: ['MATCH,DIRECT']",
    );
    let sources = resolved(
        "geosite:sample",
        SourceContent::Rules(vec!["DOMAIN,dns.example".into()]),
    );
    assert!(
        compile(&input, Target::Shadowrocket, &sources)
            .unwrap_err()
            .to_string()
            .contains("conflicting")
    );
}

#[test]
fn unknown_rules_are_preserved_with_private_diagnostics() {
    let input =
        config("rules: ['PROCESS-NAME,sample-app,DIRECT', 'DST-PORT,443,route', 'MATCH,route']");
    let result = compile(&input, Target::Shadowrocket, &ResolvedSources::new()).unwrap();
    assert_eq!(result.config, input);
    assert_eq!(result.diagnostics.len(), 1);
    assert_eq!(result.diagnostics[0].context, "rules[0]");
    assert!(
        !serde_json::to_string(&result.diagnostics)
            .unwrap()
            .contains("sample-app")
    );
}

#[test]
fn bad_sources_cycles_and_unsupported_contexts_fail_privately() {
    for input in [
        "rule-providers: {private-source: {type: file, behavior: domain, path: private-file}}\nrules: ['RULE-SET,private-source,DIRECT']",
        "rule-providers: {private-source: {type: http, behavior: domain, format: mrs, url: 'https://private.example'}}\nrules: ['RULE-SET,private-source,DIRECT']",
        "rule-providers: {private-source: {type: inline, behavior: classical, payload: ['RULE-SET,private-source']}}\nrules: ['RULE-SET,private-source,DIRECT']",
        "rules: ['GEOSITE,!private-source,DIRECT']",
        "rules: ['AND,((DOMAIN,private.example),DIRECT']",
        "sub-rules: {private-source: ['MATCH,DIRECT']}\nrules: ['MATCH,DIRECT']",
        "dns: {fake-ip-filter-mode: rule, fake-ip-filter: ['GEOSITE,private-source,real-ip']}\nrules: ['MATCH,DIRECT']",
    ] {
        let error = compile(
            &config(input),
            Target::Shadowrocket,
            &ResolvedSources::new(),
        )
        .unwrap_err()
        .to_string();
        assert!(!error.contains("private"), "{error}");
    }
}

#[test]
fn complex_expressions_are_bounded() {
    let mut rule = "DOMAIN,example.org".to_owned();
    for _ in 0..MAX_DEPTH {
        rule = format!("NOT,(({rule}))");
    }
    assert!(parse_expr(&rule, 0).is_err());
    let input = config("rules: ['NOT,((GEOSITE,sample)),DIRECT']");
    let source = (0..4_000)
        .map(|i| format!("DOMAIN,domain{i}.example"))
        .collect();
    let sources = resolved("geosite:sample", SourceContent::Rules(source));
    assert!(
        compile(&input, Target::Shadowrocket, &sources)
            .unwrap_err()
            .to_string()
            .contains("expression limit")
    );
}

#[test]
fn large_geosite_expansion_keeps_surrounding_rule_order() {
    let input =
        config("rules: ['DOMAIN,first.example,REJECT', 'GEOSITE,sample,DIRECT', 'MATCH,route']");
    let source = (0..111_224)
        .map(|i| format!("DOMAIN-SUFFIX,domain{i}.example"))
        .collect();
    let sources = resolved("geosite:sample", SourceContent::Rules(source));
    let result = compile(&input, Target::Shadowrocket, &sources).unwrap();
    let output = rules(&result);
    assert_eq!(output.len(), 111_226);
    assert_eq!(output[0], "DOMAIN,first.example,REJECT");
    assert_eq!(output[1], "DOMAIN-SUFFIX,domain0.example,DIRECT");
    assert_eq!(output[111_224], "DOMAIN-SUFFIX,domain111223.example,DIRECT");
    assert_eq!(output.last(), Some(&"MATCH,route"));
}
