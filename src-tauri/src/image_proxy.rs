use reqwest::{
    blocking::{Client, Response},
    redirect::Policy,
    StatusCode, Url,
};
use std::io::Read;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs};
use std::time::Duration;

const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_REDIRECTS: usize = 5;

pub struct ImageResponse {
    pub content_type: String,
    pub body: Vec<u8>,
}

pub enum ProxyError {
    Invalid(String),
    Blocked(String),
    Fetch(String),
}

impl ProxyError {
    pub fn status(&self) -> u16 {
        match self {
            Self::Invalid(_) => 400,
            Self::Blocked(_) => 403,
            Self::Fetch(_) => 502,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Invalid(message) | Self::Blocked(message) | Self::Fetch(message) => message,
        }
    }
}

fn is_public_ipv4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(a == 0
        || a == 10
        || a == 127
        || a >= 224
        || (a == 100 && (64..=127).contains(&b))
        || (a == 169 && b == 254)
        || (a == 172 && (16..=31).contains(&b))
        || (a == 192 && b == 168)
        || (a == 192 && b == 0 && c == 0)
        || (a == 192 && b == 0 && c == 2)
        || (a == 198 && (b == 18 || b == 19))
        || (a == 198 && b == 51 && c == 100)
        || (a == 203 && b == 0 && c == 113))
}

fn is_public_ipv6(ip: Ipv6Addr) -> bool {
    if ip.is_unspecified() || ip.is_loopback() || ip.to_ipv4().is_some() {
        return false;
    }
    let first = ip.segments()[0];
    (0x2000..=0x3fff).contains(&first) && ip.segments()[..2] != [0x2001, 0x0db8]
}

fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => is_public_ipv4(ip),
        IpAddr::V6(ip) => is_public_ipv6(ip),
    }
}

fn normalized_host(host: &str) -> String {
    host.trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn validate_url(raw_url: &str) -> Result<Url, ProxyError> {
    let url = Url::parse(raw_url).map_err(|_| ProxyError::Invalid("Invalid image URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(ProxyError::Invalid(
            "Only HTTP and HTTPS images are supported".into(),
        ));
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(ProxyError::Invalid(
            "Image URLs must not contain credentials".into(),
        ));
    }
    let expected_port = if url.scheme() == "https" { 443 } else { 80 };
    if url.port_or_known_default() != Some(expected_port) {
        return Err(ProxyError::Blocked("Non-standard image URL port".into()));
    }
    let host = url
        .host_str()
        .ok_or_else(|| ProxyError::Invalid("Image URL has no host".into()))?;
    if host.ends_with('.') {
        return Err(ProxyError::Invalid(
            "Image URL host must not have a trailing dot".into(),
        ));
    }
    let normalized = normalized_host(host);
    if normalized == "localhost" || normalized.ends_with(".localhost") {
        return Err(ProxyError::Blocked(
            "Local image hosts are not allowed".into(),
        ));
    }
    if let Ok(ip) = normalized.parse::<IpAddr>() {
        if !is_public_ip(ip) {
            return Err(ProxyError::Blocked(
                "Private image addresses are not allowed".into(),
            ));
        }
    }
    Ok(url)
}

fn resolve_public_host(url: &Url) -> Result<(String, Vec<SocketAddr>), ProxyError> {
    let host = url
        .host_str()
        .ok_or_else(|| ProxyError::Invalid("Image URL has no host".into()))?;
    let host = normalized_host(host);
    let port = url
        .port_or_known_default()
        .ok_or_else(|| ProxyError::Invalid("Image URL has no port".into()))?;
    let addresses: Vec<_> = (host.as_str(), port)
        .to_socket_addrs()
        .map_err(|error| ProxyError::Fetch(format!("Could not resolve image host: {error}")))?
        .collect();
    if addresses.is_empty() {
        return Err(ProxyError::Fetch(
            "Image host resolved to no addresses".into(),
        ));
    }
    if addresses.iter().any(|address| !is_public_ip(address.ip())) {
        return Err(ProxyError::Blocked(
            "Image host resolves to a private address".into(),
        ));
    }
    Ok((host, addresses))
}

// Redirects are followed by `fetch`, one hop at a time, so every hop gets the same checks.
fn image_client(host: &str, addresses: &[SocketAddr]) -> Result<Client, ProxyError> {
    Client::builder()
        .timeout(Duration::from_secs(30))
        .redirect(Policy::none())
        .resolve_to_addrs(host, addresses)
        .build()
        .map_err(|error| ProxyError::Fetch(format!("Could not create image client: {error}")))
}

/// The validated target of a redirect response, or `None` when the response is not a redirect.
fn redirect_target(
    url: &Url,
    status: StatusCode,
    location: Option<&str>,
) -> Result<Option<Url>, ProxyError> {
    if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
        return Ok(None);
    }
    let target = location
        .and_then(|location| url.join(location).ok())
        .ok_or_else(|| ProxyError::Fetch("Image redirect has no usable target".into()))?;
    validate_url(target.as_str()).map(Some)
}

pub fn fetch(raw_url: &str) -> Result<ImageResponse, ProxyError> {
    let mut url = validate_url(raw_url)?;
    for _ in 0..=MAX_REDIRECTS {
        // Each hop is resolved, checked for private addresses and pinned to that answer,
        // so a redirect cannot reach a host that a direct image URL could not.
        let (host, addresses) = resolve_public_host(&url)?;
        let client = image_client(&host, &addresses)?;
        let response = client
            .get(url.clone())
            .send()
            .map_err(|error| ProxyError::Fetch(format!("Image request failed: {error}")))?;
        let location = response
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|value| value.to_str().ok());
        match redirect_target(&url, response.status(), location)? {
            Some(target) => url = target,
            None => return read_image(response),
        }
    }
    Err(ProxyError::Fetch("Image redirected too many times".into()))
}

fn read_image(mut response: Response) -> Result<ImageResponse, ProxyError> {
    if !response.status().is_success() {
        return Err(ProxyError::Fetch(format!(
            "Image server returned {}",
            response.status()
        )));
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_IMAGE_BYTES)
    {
        return Err(ProxyError::Blocked("Image exceeds the 20 MiB limit".into()));
    }
    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type.to_ascii_lowercase().starts_with("image/") {
        return Err(ProxyError::Blocked(
            "Remote resource is not an image".into(),
        ));
    }
    let mut body = Vec::new();
    response
        .by_ref()
        .take(MAX_IMAGE_BYTES + 1)
        .read_to_end(&mut body)
        .map_err(|error| ProxyError::Fetch(format!("Could not read image: {error}")))?;
    if body.len() as u64 > MAX_IMAGE_BYTES {
        return Err(ProxyError::Blocked("Image exceeds the 20 MiB limit".into()));
    }
    Ok(ImageResponse { content_type, body })
}

#[cfg(test)]
mod tests {
    use super::{is_public_ip, redirect_target, validate_url};
    use reqwest::{StatusCode, Url};
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

    #[test]
    fn blocks_local_and_private_networks() {
        for ip in [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V6(Ipv6Addr::LOCALHOST),
            "fc00::1".parse().unwrap(),
            "fe80::1".parse().unwrap(),
        ] {
            assert!(!is_public_ip(ip), "{ip}");
        }
        assert!(is_public_ip("93.184.216.34".parse().unwrap()));
        assert!(is_public_ip(
            "2606:2800:220:1:248:1893:25c8:1946".parse().unwrap()
        ));
    }

    #[test]
    fn validates_only_public_web_image_urls() {
        for url in [
            "file:///etc/passwd",
            "http://localhost/image.png",
            "http://127.0.0.1/image.png",
            "http://[::1]/image.png",
            "http://example.com:8080/image.png",
            "https://example.com./image.png",
            "https://user:pass@example.com/image.png",
        ] {
            assert!(validate_url(url).is_err(), "{url}");
        }
        assert!(validate_url("https://example.com/image.png").is_ok());
        assert!(validate_url("http://93.184.216.34/image.png").is_ok());
    }

    #[test]
    fn redirects_are_followed_to_other_public_hosts_only() {
        let url = Url::parse("https://example.com/a/image.png").unwrap();
        let target = |status: u16, location: Option<&str>| {
            redirect_target(&url, StatusCode::from_u16(status).unwrap(), location)
        };
        let ok = |status, location| target(status, Some(location)).ok().unwrap().unwrap();
        assert_eq!(
            ok(301, "https://cdn.example.net/image.png").as_str(),
            "https://cdn.example.net/image.png"
        );
        assert_eq!(
            ok(302, "/b/image.png").as_str(),
            "https://example.com/b/image.png"
        );
        assert_eq!(
            ok(307, "other.png").as_str(),
            "https://example.com/a/other.png"
        );
        assert!(target(200, None).ok().unwrap().is_none());
        assert!(target(304, None).ok().unwrap().is_none());
        for location in [
            "http://localhost/image.png",
            "http://127.0.0.1/image.png",
            "http://169.254.169.254/latest/meta-data",
            "http://[::1]/image.png",
            "https://example.net:8443/image.png",
            "https://user:pass@example.net/image.png",
            "file:///etc/passwd",
        ] {
            assert!(target(302, Some(location)).is_err(), "{location}");
        }
        assert!(target(302, None).is_err());
    }
}
