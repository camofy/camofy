//! Local integration fixture. This is not Mihomo and must never be deployed.
use anyhow::{Result, ensure};
use serde_json::json;
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
#[tokio::main]
async fn main() -> Result<()> {
    let args = std::env::args().collect::<Vec<_>>();
    let value = |key: &str| {
        args.iter()
            .position(|x| x == key)
            .map(|i| PathBuf::from(&args[i + 1]))
            .unwrap()
    };
    let file = value("-f");
    let root = value("-d");
    let text = tokio::fs::read_to_string(file).await?;
    let config = camofy::engine::parse(&text)?;
    ensure!(
        !config["fixture-reject"].as_bool().unwrap_or(false),
        "fixture rejected candidate"
    );
    if args.iter().any(|x| x == "-t") {
        return Ok(());
    }
    let auth = format!("Bearer {}", config["secret"].as_str().unwrap());
    let listener =
        tokio::net::TcpListener::bind(config["external-controller"].as_str().unwrap()).await?;
    tokio::fs::write(root.join("mock-active.yaml"), &text).await?;
    let selections = std::sync::Arc::new(tokio::sync::Mutex::new(std::collections::BTreeMap::<
        String,
        String,
    >::new()));
    loop {
        let (mut stream, _) = listener.accept().await?;
        let root = root.clone();
        let auth = auth.clone();
        let selections = selections.clone();
        tokio::spawn(async move {
            let result=async {
            let mut bytes=Vec::new();while !bytes.ends_with(b"\r\n\r\n"){ensure!(bytes.len()<16384,"headers too large");bytes.push(stream.read_u8().await?);}
            let headers=String::from_utf8(bytes)?;let parts=headers.lines().next().unwrap().split_whitespace().collect::<Vec<_>>();let method=parts[0];let path=parts[1];
            let authorized=headers.lines().any(|l|l.split_once(':').is_some_and(|(k,v)|k.eq_ignore_ascii_case("authorization")&&v.trim()==auth));
            let len=headers.lines().find_map(|l|l.split_once(':').filter(|(k,_)|k.eq_ignore_ascii_case("content-length")).map(|(_,v)|v.trim().parse::<usize>().unwrap())).unwrap_or(0);
            let mut body=vec![0;len];stream.read_exact(&mut body).await?;
            let mut status=if authorized{200}else{401};let mut response=json!({"version":"fixture"});
            if authorized&&path.split('?').next()==Some("/configs")&&method=="PUT" {
                let v:serde_json::Value=serde_json::from_slice(&body)?;let text=tokio::fs::read_to_string(v["path"].as_str().unwrap()).await?;let next=camofy::engine::parse(&text)?;
                if next["fixture-fail-reload"].as_bool().unwrap_or(false){status=500;}else{tokio::fs::write(root.join("mock-active.yaml"),text).await?;}
            }
            if path.starts_with("/proxies/")&&method=="PUT"{
                tokio::fs::write(root.join("mock-selection.json"),&body).await?;
                let group=percent_encoding::percent_decode_str(path.trim_start_matches("/proxies/")).decode_utf8()?.into_owned();
                let value:serde_json::Value=serde_json::from_slice(&body)?;
                if value["name"]!="fixture-ignore"{selections.lock().await.insert(group,value["name"].as_str().unwrap_or("").into());}
            }
            if path=="/proxies"{
                let yaml=camofy::engine::parse(&tokio::fs::read_to_string(root.join("mock-active.yaml")).await?)?;
                let mut proxies=serde_json::Map::new();
                for node in yaml["proxies"].as_sequence().into_iter().flatten(){proxies.insert(node["name"].as_str().unwrap().into(),json!({"type":"Shadowsocks"}));}
                for group in yaml["proxy-groups"].as_sequence().into_iter().flatten(){
                    let name=group["name"].as_str().unwrap();
                    let members:Vec<_>=group["proxies"].as_sequence().into_iter().flatten().filter_map(|n|n.as_str()).collect();
                    let selected=selections.lock().await.get(name).cloned().filter(|n|members.contains(&n.as_str())).or_else(||members.first().map(|s|s.to_string()));
                    proxies.insert(name.into(),json!({"type":"Selector","all":members,"now":selected}));
                }
                response=json!({"proxies":proxies});
            }
            if path.contains("/delay?"){response=json!({"delay":42});}
            let body=serde_json::to_vec(&response)?;stream.write_all(format!("HTTP/1.1 {status} Fixture\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).as_bytes()).await?;stream.write_all(&body).await?;
            if authorized&&path=="/fixture/stop" {std::process::exit(0);}
            Ok::<_,anyhow::Error>(())
        }.await;
            if let Err(e) = result {
                eprintln!("fixture request: {e}");
            }
        });
    }
}
