use semver::Version;
use serde::Deserialize;
use std::time::Duration;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPOSITORY_URL: &str = "https://github.com/yan-ad/ulos";
const ENDPOINT: &str = "https://api.github.com/repos/yan-ad/ulos/releases/latest";
const MAX_BODY: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub url: String,
}
#[derive(Deserialize)]
struct Response {
    tag_name: String,
    html_url: String,
    draft: bool,
    prerelease: bool,
}
fn parse(current: &str, bytes: &[u8]) -> Result<Option<Release>, &'static str> {
    if bytes.len() > MAX_BODY {
        return Err("Release information is too large.");
    }
    let release: Response =
        serde_json::from_slice(bytes).map_err(|_| "Invalid release information.")?;
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let version = release
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&release.tag_name);
    let version = Version::parse(version).map_err(|_| "Invalid release version.")?;
    let current = Version::parse(current).map_err(|_| "Invalid application version.")?;
    let expected = format!("{REPOSITORY_URL}/releases/tag/{}", release.tag_name);
    if release.html_url != expected
        || !release
            .tag_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b".-+".contains(&b))
    {
        return Err("Invalid release link.");
    }
    if version <= current {
        return Ok(None);
    }
    Ok(Some(Release {
        version: version.to_string(),
        url: expected,
    }))
}

pub async fn check(current: &str) -> Result<Option<Release>, &'static str> {
    request(current, ENDPOINT).await
}
async fn request(current: &str, endpoint: &str) -> Result<Option<Release>, &'static str> {
    let client = reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("Ulos/{VERSION}"))
        .build()
        .map_err(|_| "Could not prepare the update check.")?;
    let mut response = client
        .get(endpoint)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .send()
        .await
        .map_err(|_| "Could not reach GitHub. Try again later.")?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if response.status() != reqwest::StatusCode::OK {
        return Err("GitHub could not provide release information. Try again later.");
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BODY as u64)
    {
        return Err("Release information is too large.");
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read release information.")?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_BODY {
            return Err("Release information is too large.");
        }
        bytes.extend_from_slice(&chunk);
    }
    parse(current, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn response(tag: &str) -> Vec<u8> {
        serde_json::to_vec(&serde_json::json!({"tag_name":tag,"html_url":format!("{REPOSITORY_URL}/releases/tag/{tag}"),"draft":false,"prerelease":false})).unwrap()
    }
    #[test]
    fn compares_semver_including_normal_releases_with_pre_tags() {
        assert!(
            parse("0.0.1-pre.0", &response("v0.0.1-pre.0"))
                .unwrap()
                .is_none()
        );
        assert!(
            parse("0.0.1-pre.0", &response("v0.0.1-pre.1"))
                .unwrap()
                .is_some()
        );
        assert!(parse("0.0.1-pre.0", &response("v0.0.1")).unwrap().is_some());
        assert!(parse("0.0.2", &response("v0.0.1")).unwrap().is_none());
        assert!(parse("0.0.1", &response("bad-tag")).is_err());
    }
    #[test]
    fn release_links_and_payloads_fail_closed() {
        for link in [
            "https://evil.test/release",
            "https://github.com/other/ulos/releases/tag/v1.0.0",
            "https://github.com/yan-ad/ulos/releases/tag/v1.0.0?redirect=x",
        ] {
            let mut body: serde_json::Value = serde_json::from_slice(&response("v1.0.0")).unwrap();
            body["html_url"] = link.into();
            assert!(parse("0.0.1", &serde_json::to_vec(&body).unwrap()).is_err());
        }
        for flag in ["draft", "prerelease"] {
            let mut body: serde_json::Value = serde_json::from_slice(&response("v1.0.0")).unwrap();
            body[flag] = true.into();
            assert!(
                parse("0.0.1", &serde_json::to_vec(&body).unwrap())
                    .unwrap()
                    .is_none()
            );
        }
        assert!(parse("0.0.1", &vec![b' '; MAX_BODY + 1]).is_err());
        assert!(parse("0.0.1", b"not json").is_err());
    }
    #[tokio::test]
    async fn bounded_http_handles_missing_redirect_error_and_success() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        for (status, body, result) in [
            ("200 OK", response("v0.0.2"), 1),
            ("404 Not Found", vec![], 0),
            ("302 Found", vec![], -1),
            ("500 Error", vec![], -1),
            ("200 OK", vec![b'x'; MAX_BODY + 1], -1),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("http://{}/latest", listener.local_addr().unwrap());
            let worker = tokio::spawn(async move {
                let (mut stream, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    request.push(stream.read_u8().await.unwrap());
                    assert!(request.len() < 16384);
                }
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(&body).await;
            });
            let checked = request("0.0.1-pre.0", &url).await;
            match result {
                1 => assert!(checked.unwrap().is_some()),
                0 => assert!(checked.unwrap().is_none()),
                _ => assert!(checked.is_err()),
            }
            worker.await.unwrap();
        }
    }
}
