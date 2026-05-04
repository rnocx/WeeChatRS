use std::net::IpAddr;
use url::Url;

// Reject URLs that would let a remote chat participant point us at internal
// infrastructure (cloud metadata, private LAN, the loopback interface, etc.).
//
// Note: this validates by URL host string only. A hostile domain whose A/AAAA
// records resolve to a private IP at the moment of connect (DNS rebinding) is
// not blocked here; covering that requires a custom resolver. This guard does
// stop the common in-channel attacks where an attacker pastes a literal-IP or
// `localhost` URL.
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

fn is_public_ip(ip: IpAddr) -> bool {
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
    fn allows_public() {
        assert!(is_safe_public_url("https://example.com/"));
        assert!(is_safe_public_url("http://1.1.1.1/"));
        assert!(is_safe_public_url("https://[2606:4700:4700::1111]/"));
    }
}
