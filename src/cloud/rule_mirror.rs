//! Immutable mirror of a public ruleset. No identity data or upstream URL input.
use axum::{
    body::{Body, Bytes},
    extract::Path,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use std::{io::Read, sync::OnceLock};

pub const REVISION: &str = "ad2798bba7340c09298f364bebedebbfa4398f5b";
pub const SHA256: &str = "5a992276c844c69afad4bff79aeb7acd807249990d049e1373e54c24aecd0aed";
pub const SOURCE_URL: &str = "https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/ad2798bba7340c09298f364bebedebbfa4398f5b/geo/geosite/classical/cn.list";
const LENGTH: usize = 2_965_298;
const COMPRESSED: &[u8] = include_bytes!("../../data/rules/geosite/cn.list.gz");

pub fn body() -> anyhow::Result<&'static Bytes> {
    static BODY: OnceLock<Result<Bytes, String>> = OnceLock::new();
    BODY.get_or_init(|| {
        let mut bytes = Vec::with_capacity(LENGTH);
        flate2::read::GzDecoder::new(COMPRESSED)
            .take(LENGTH as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| "bundled CN mirror cannot be decoded".to_owned())?;
        if bytes.len() != LENGTH || camofy::digest(&bytes) != SHA256 {
            return Err("bundled CN mirror checksum differs".to_owned());
        }
        Ok(Bytes::from(bytes))
    })
    .as_ref()
    .map_err(|error| anyhow::anyhow!(error.clone()))
}

pub fn descriptor(origin: &str) -> camofy::config_compat::CnMirror {
    camofy::config_compat::CnMirror {
        url: format!(
            "{}/api/rules/geosite/{REVISION}/cn.list",
            origin.trim_end_matches('/')
        ),
        expected_source_hash: SHA256.into(),
    }
}

pub async fn download(
    Path(revision): Path<String>,
    headers: HeaderMap,
) -> Result<Response, crate::Error> {
    if revision != REVISION {
        return Err(crate::Error::not_found());
    }
    let bytes = body()
        .map_err(|_| crate::Error::new(StatusCode::SERVICE_UNAVAILABLE, "规则镜像暂不可用"))?;
    let etag = format!("\"{SHA256}\"");
    let unchanged = headers.get_all(header::IF_NONE_MATCH).iter().any(|value| {
        value.to_str().is_ok_and(|value| {
            value.split(',').any(|tag| {
                let tag = tag.trim();
                tag == "*" || tag.strip_prefix("W/").unwrap_or(tag) == etag
            })
        })
    });
    let mut response = if unchanged {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Response::new(Body::from(bytes.clone()))
    };
    let headers = response.headers_mut();
    headers.insert(
        header::CONTENT_TYPE,
        "text/plain; charset=utf-8".parse().unwrap(),
    );
    headers.insert(
        header::CACHE_CONTROL,
        "public, max-age=31536000, immutable".parse().unwrap(),
    );
    headers.insert(header::ETAG, etag.parse().unwrap());
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, "nosniff".parse().unwrap());
    if !unchanged {
        headers.insert(header::CONTENT_LENGTH, LENGTH.to_string().parse().unwrap());
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, http::Request, routing::get};
    use tower::ServiceExt;

    #[tokio::test]
    async fn public_mirror_is_complete_immutable_and_has_no_fallback() {
        let router = Router::new().route("/api/rules/geosite/:revision/cn.list", get(download));
        let path = format!("/api/rules/geosite/{REVISION}/cn.list");
        let response = router
            .clone()
            .oneshot(Request::builder().uri(&path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let etag = response.headers()[header::ETAG].clone();
        assert_eq!(
            response.headers()[header::CACHE_CONTROL],
            "public, max-age=31536000, immutable"
        );
        let content = axum::body::to_bytes(response.into_body(), LENGTH + 1)
            .await
            .unwrap();
        assert_eq!(camofy::digest(&content), SHA256);
        let text = std::str::from_utf8(&content).unwrap();
        assert_eq!(text.lines().count(), 111_224);
        assert!(text.lines().all(|line| line.starts_with("DOMAIN-SUFFIX,")));
        let conditional = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&path)
                    .header(
                        header::IF_NONE_MATCH,
                        format!("\"other\", W/{}", etag.to_str().unwrap()),
                    )
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(conditional.status(), StatusCode::NOT_MODIFIED);
        assert!(
            axum::body::to_bytes(conditional.into_body(), 1)
                .await
                .unwrap()
                .is_empty()
        );
        let head = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("HEAD")
                    .uri(&path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(head.headers()[header::CONTENT_LENGTH], LENGTH.to_string());
        assert!(
            axum::body::to_bytes(head.into_body(), 1)
                .await
                .unwrap()
                .is_empty()
        );
        let missing = router
            .oneshot(
                Request::builder()
                    .uri("/api/rules/geosite/unknown/cn.list")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    }
}
