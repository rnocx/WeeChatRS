use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use url::Url;

// Reject URLs that would let a remote chat participant point us at internal
// infrastructure (cloud metadata, private LAN, the loopback interface, etc.).
//
// `is_safe_public_url` looks at the URL text only, so it stops a pasted literal address
// or `localhost`. A name that resolves to a private address, and a public URL that
// redirects to one, are stopped by `public_client`: it re-checks every redirect and
// resolves names itself, dropping any address that is not public. Fetch untrusted URLs
// with that client and nothing else.
pub fn is_safe_public_url(url_str: &str) -> bool {
    let url = match Url::parse(url_str) {
        Ok(u) => u,
        Err(_) => return false,
    };

    if url.scheme() != "http" && url.scheme() != "https" {
        return false;
    }

    let host = match url.host_str() {
        Some(h) => h,
        None => return false,
    };

    if matches!(host.to_ascii_lowercase().as_str(),
        "localhost" | "ip6-localhost" | "ip6-loopback" | "broadcasthost") {
        return false;
    }

    if let Ok(ip) = host.parse::<IpAddr>() {
        return is_public_ip(ip);
    }
    if let Some(stripped) = host.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
        if let Ok(ip) = stripped.parse::<IpAddr>() {
            return is_public_ip(ip);
        }
    }

    true
}

/// A redirect hops at most this many times.
const MAX_REDIRECTS: usize = 5;

/// Whether a redirect from `previous` hops to `next` may be followed.
fn redirect_allowed(next: &Url, hops: usize) -> bool {
    hops < MAX_REDIRECTS && is_safe_public_url(next.as_str())
}

/// Resolves names for `public_client`, keeping only public addresses. The connection is
/// made to the addresses returned here, so a name cannot be one thing when checked and
/// another when connected to (DNS rebinding).
struct PublicOnlyResolver;

impl reqwest::dns::Resolve for PublicOnlyResolver {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async move {
            let public: Vec<SocketAddr> = tokio::net::lookup_host((name.as_str(), 0)).await?
                .filter(|addr| is_public_ip(addr.ip()))
                .collect();
            if public.is_empty() {
                return Err("the host only resolves to non-public addresses".into());
            }
            Ok(Box::new(public.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// An HTTP client for a URL a chat participant chose. It refuses redirects to anything
/// `is_safe_public_url` refuses, and connects only to public addresses. Check the first
/// URL with `is_safe_public_url` before using it: a literal address in it is not
/// resolved, so only that check sees it.
pub fn public_client(timeout: Duration, user_agent: &str) -> Result<reqwest::Client, reqwest::Error> {
    reqwest::Client::builder()
        .timeout(timeout)
        .user_agent(user_agent)
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if redirect_allowed(attempt.url(), attempt.previous().len()) {
                attempt.follow()
            } else {
                attempt.error("redirected to a non-public address, or too many times")
            }
        }))
        .dns_resolver(Arc::new(PublicOnlyResolver))
        .build()
}

/// Whether `host` is this machine: localhost, a 127.x.x.x address or ::1, written bare
/// or with the brackets of an IPv6 URL host. A connection to one never leaves the
/// machine, so nothing on a network can read it.
pub fn is_loopback_host(host: &str) -> bool {
    let host = host.trim().trim_end_matches('.').to_ascii_lowercase();
    let host = host.strip_prefix('[').and_then(|h| h.strip_suffix(']')).unwrap_or(&host);
    host == "localhost" || host.ends_with(".localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

pub(crate) fn is_public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            !(v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_multicast()
                || v4.is_unspecified()
                || v4.is_documentation()
                // Carrier-grade NAT 100.64.0.0/10
                || (o[0] == 100 && (o[1] & 0xC0) == 64)
                // Benchmarking 198.18.0.0/15
                || (o[0] == 198 && (o[1] == 18 || o[1] == 19))
                // Reserved
                || o[0] == 0
                || o[0] >= 240)
        }
        IpAddr::V6(v6) => {
            !(v6.is_loopback()
                || v6.is_unspecified()
                || v6.is_multicast()
                // Unique local fc00::/7
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // Link-local fe80::/10
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // IPv4-mapped — check the embedded v4
                || v6.to_ipv4_mapped().map(|v4| !is_public_ip(IpAddr::V4(v4))).unwrap_or(false))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_localhost_and_private() {
        assert!(!is_safe_public_url("http://localhost/"));
        assert!(!is_safe_public_url("http://127.0.0.1/"));
        assert!(!is_safe_public_url("http://127.1.2.3/"));
        assert!(!is_safe_public_url("http://10.0.0.1/"));
        assert!(!is_safe_public_url("http://192.168.1.1/"));
        assert!(!is_safe_public_url("http://172.16.0.1/"));
        assert!(!is_safe_public_url("http://169.254.169.254/"));
        assert!(!is_safe_public_url("http://100.64.0.1/"));
        assert!(!is_safe_public_url("http://[::1]/"));
        assert!(!is_safe_public_url("http://[fc00::1]/"));
        assert!(!is_safe_public_url("http://[fe80::1]/"));
    }

    #[test]
    fn blocks_non_http() {
        assert!(!is_safe_public_url("file:///etc/passwd"));
        assert!(!is_safe_public_url("ftp://example.com/"));
        assert!(!is_safe_public_url("javascript:alert(1)"));
        assert!(!is_safe_public_url("not a url"));
    }

    #[test]
    fn loopback_hosts() {
        for host in ["localhost", "LocalHost", "localhost.", "irc.localhost", "127.0.0.1", "127.9.9.9", "::1", "[::1]", " 127.0.0.1 "] {
            assert!(is_loopback_host(host), "{host}");
        }
        for host in ["", "example.com", "192.168.1.5", "10.0.0.1", "0.0.0.0", "notlocalhost", "localhost.example.com", "[fe80::1]"] {
            assert!(!is_loopback_host(host), "{host}");
        }
    }

    #[test]
    fn redirects_follow_the_same_rule_and_stop_after_five() {
        let url = |s: &str| Url::parse(s).unwrap();
        assert!(redirect_allowed(&url("https://example.com/next"), 0));
        assert!(redirect_allowed(&url("https://example.com/next"), 4));
        assert!(!redirect_allowed(&url("https://example.com/next"), 5));
        for target in ["http://127.0.0.1/", "http://192.168.1.1/admin", "http://169.254.169.254/latest/meta-data/", "http://[::1]/", "http://localhost/", "file:///etc/passwd", "ftp://example.com/"] {
            assert!(!redirect_allowed(&url(target), 0), "{target}");
        }
    }

    /// A server on this machine that answers every request with a redirect to `location`.
    async fn redirecting_to(location: &'static str) -> u16 {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut buf = [0u8; 1024];
                let _ = stream.read(&mut buf).await;
                let reply = format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                let _ = stream.write_all(reply.as_bytes()).await;
            }
        });
        port
    }

    #[tokio::test]
    async fn a_public_url_cannot_redirect_the_client_to_a_private_address() {
        let client = public_client(Duration::from_secs(5), "test").unwrap();
        for target in ["http://192.168.1.1/admin", "http://169.254.169.254/latest/meta-data/", "http://127.0.0.1:1/"] {
            let port = redirecting_to(target).await;
            // The first hop is a literal address on this machine: the caller's job to
            // have checked it. What matters is that the redirect from it is refused.
            let err = client.get(format!("http://127.0.0.1:{port}/")).send().await.unwrap_err();
            assert!(err.is_redirect(), "{target}: {err}");
        }
    }

    #[tokio::test]
    async fn a_name_that_resolves_to_this_machine_is_not_connected_to() {
        let port = redirecting_to("https://example.com/").await;
        let client = public_client(Duration::from_secs(5), "test").unwrap();
        // localhost resolves to 127.0.0.1 and ::1, where the server above is listening.
        let err = client.get(format!("http://localhost:{port}/")).send().await.unwrap_err();
        assert!(err.is_connect(), "{err:?}");
        assert!(format!("{err:?}").contains("non-public"), "{err:?}");
    }

    #[test]
    fn allows_public() {
        assert!(is_safe_public_url("https://example.com/"));
        assert!(is_safe_public_url("http://1.1.1.1/"));
        assert!(is_safe_public_url("https://[2606:4700:4700::1111]/"));
    }
}
