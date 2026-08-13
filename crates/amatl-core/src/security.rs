use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use url::{Host, Url};

pub fn validate_search_url(raw: &str) -> Result<Url, &'static str> {
    let url = Url::parse(raw).map_err(|_| "invalid_url")?;
    validate_deep_url(&url)?;
    Ok(url)
}

pub fn validate_deep_url(url: &Url) -> Result<(), &'static str> {
    if !matches!(url.scheme(), "http" | "https") {
        return Err("scheme_not_allowed");
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("embedded_credentials");
    }
    let host = url.host().ok_or("missing_host")?;
    match host {
        Host::Domain(domain) if blocked_hostname(domain) => Err("host_blocked"),
        Host::Ipv4(address) if !is_public_ip(IpAddr::V4(address)) => Err("address_blocked"),
        Host::Ipv6(address) if !is_public_ip(IpAddr::V6(address)) => Err("address_blocked"),
        _ => Ok(()),
    }
}

pub fn validate_resolved_addresses(addresses: &[IpAddr]) -> Result<(), &'static str> {
    if addresses.is_empty() {
        return Err("dns_empty");
    }
    if addresses.iter().any(|address| !is_public_ip(*address)) {
        return Err("address_blocked");
    }
    Ok(())
}

fn blocked_hostname(host: &str) -> bool {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    !host.contains('.')
        || host == "localhost"
        || [
            ".localhost",
            ".local",
            ".localdomain",
            ".internal",
            ".intranet",
            ".lan",
            ".home",
        ]
        .iter()
        .any(|suffix| host.ends_with(suffix))
}

pub fn is_public_ip(address: IpAddr) -> bool {
    match address {
        IpAddr::V4(address) => public_v4(address),
        IpAddr::V6(address) => public_v6(address),
    }
}

fn public_v4(address: Ipv4Addr) -> bool {
    !(address.is_private()
        || address.is_loopback()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_documentation()
        || address.is_unspecified()
        || address.is_multicast()
        || address.octets()[0] == 0
        || address.octets()[0] >= 240
        || (address.octets()[0] == 100 && (64..=127).contains(&address.octets()[1]))
        || (address.octets()[0] == 192 && address.octets()[1] == 0 && address.octets()[2] == 0)
        || (address.octets()[0] == 198 && (18..=19).contains(&address.octets()[1])))
}

fn public_v6(address: Ipv6Addr) -> bool {
    if let Some(mapped) = address.to_ipv4_mapped() {
        return public_v4(mapped);
    }
    let segments = address.segments();
    !(address.is_loopback()
        || address.is_unspecified()
        || address.is_multicast()
        || (segments[0] & 0xfe00) == 0xfc00
        || (segments[0] & 0xffc0) == 0xfe80
        || (segments[0] == 0x2001 && segments[1] == 0x0db8))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_non_http_credentials_and_internal_names() {
        assert!(validate_search_url("file:///etc/passwd").is_err());
        assert!(validate_search_url("https://u:p@example.com").is_err());
        assert!(validate_search_url("http://localhost/admin").is_err());
        assert!(validate_search_url("http://service.internal/admin").is_err());
        assert!(validate_search_url("http://router/admin").is_err());
        assert!(validate_search_url("http://service.localdomain/admin").is_err());
        assert!(validate_search_url("http://service.intranet/admin").is_err());
    }

    #[test]
    fn blocks_private_loopback_link_local_and_mapped_addresses() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.1.1",
            "100.127.0.1",
            "198.18.0.1",
            "::1",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
        ] {
            assert!(!is_public_ip(address.parse().unwrap()), "{address}");
        }
        assert!(is_public_ip("93.184.216.34".parse().unwrap()));
    }

    #[test]
    fn rejects_entire_dns_answer_if_any_address_is_private() {
        assert!(validate_resolved_addresses(&[
            "93.184.216.34".parse().unwrap(),
            "127.0.0.1".parse().unwrap(),
        ])
        .is_err());
    }
}
