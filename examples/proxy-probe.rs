//! Temporary on-device network probe for firmware curl builds without proxy support.
//! Negative control proves requests cannot silently fall back to a direct connection.
use anyhow::{Result, ensure};
use std::time::Duration;
fn client(proxy: &str) -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .no_proxy()
        .proxy(reqwest::Proxy::all(proxy)?)
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::none())
        .build()?)
}
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<()> {
    let invalid = client("http://127.0.0.1:1")?;
    ensure!(
        invalid
            .get("https://www.gstatic.com/generate_204")
            .send()
            .await
            .is_err(),
        "negative control bypassed proxy"
    );
    println!("negative_control=passed");
    let proxy = client("http://127.0.0.1:17890")?;
    let response = proxy
        .get("https://www.gstatic.com/generate_204")
        .send()
        .await?;
    ensure!(
        response.status().as_u16() == 204,
        "unexpected probe status: {}",
        response.status()
    );
    println!("router_proxy_https_status=204");
    let trace = proxy
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let ip = trace
        .lines()
        .find_map(|l| l.strip_prefix("ip="))
        .ok_or_else(|| anyhow::anyhow!("trace lacks IP"))?;
    let _: std::net::IpAddr = ip.parse()?;
    let direct = reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(20))
        .build()?
        .get("https://www.cloudflare.com/cdn-cgi/trace")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let direct_ip = direct
        .lines()
        .find_map(|l| l.strip_prefix("ip="))
        .ok_or_else(|| anyhow::anyhow!("direct trace lacks IP"))?;
    ensure!(
        ip != direct_ip,
        "proxy and direct egress must differ for this probe"
    );
    println!("distinct_proxy_egress=true");
    println!(
        "proxy_location={}",
        trace
            .lines()
            .find_map(|l| l.strip_prefix("loc="))
            .unwrap_or("unknown")
    );
    Ok(())
}
