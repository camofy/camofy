//! Local, read-only migration validation. Never emits node credentials or URLs.
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let path = std::env::args()
        .nth(1)
        .context("usage: inspect-config <yaml>")?;
    let v = camofy::engine::parse(&std::fs::read_to_string(path)?)?;
    camofy::engine::validate(&v)?;
    println!(
        "{}",
        serde_json::json!({
            "nodes":v["proxies"].as_sequence().map(Vec::len).unwrap_or(0),
            "groups":v["proxy-groups"].as_sequence().map(Vec::len).unwrap_or(0),
            "rules":v["rules"].as_sequence().map(Vec::len).unwrap_or(0),
            "tun":v["tun"]["enable"].as_bool(),
            "mixed_port":v["mixed-port"].as_u64(),
            "allow_lan":v["allow-lan"].as_bool(),
            "dns_listen":v["dns"]["listen"].as_str(),
            "select_groups":v["proxy-groups"].as_sequence().into_iter().flatten()
                .filter(|g| g["type"].as_str()==Some("select"))
                .map(|g|g["name"].as_str().unwrap_or("")).collect::<Vec<_>>()
        })
    );
    Ok(())
}
