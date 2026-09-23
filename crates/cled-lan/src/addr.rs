//! Choosing which of a device's addresses to try first.
//!
//! Devices announce every address they have: Wi-Fi and Ethernet, but also IPv6 link-local
//! addresses and virtual networks (Docker bridges, VM networks, VPNs). Only some are reachable
//! from here. Addresses are tried in order, best first.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Sorts `addresses` best first and removes duplicates and unusable ones (loopback, unless
/// nothing else is left).
pub(crate) fn rank(addresses: impl IntoIterator<Item = SocketAddr>) -> Vec<SocketAddr> {
    let local = local_ipv4_networks();
    rank_with(addresses, &local)
}

fn rank_with(
    addresses: impl IntoIterator<Item = SocketAddr>,
    local: &[(Ipv4Addr, Ipv4Addr)],
) -> Vec<SocketAddr> {
    let mut ranked: Vec<(u8, SocketAddr)> = Vec::new();
    for address in addresses {
        if !ranked.iter().any(|(_, a)| *a == address) {
            ranked.push((score(&address, local), address));
        }
    }
    // Stable: equally good addresses keep their announced order.
    ranked.sort_by_key(|(score, _)| *score);
    ranked.into_iter().map(|(_, address)| address).collect()
}

/// Lower is better.
fn score(address: &SocketAddr, local: &[(Ipv4Addr, Ipv4Addr)]) -> u8 {
    match address.ip() {
        IpAddr::V4(ip) if ip.is_loopback() => 6,
        IpAddr::V4(ip) if ip.is_link_local() => 4,
        // Same subnet as one of this machine's interfaces: almost certainly reachable directly.
        IpAddr::V4(ip) if local.iter().any(|(net, mask)| same_subnet(ip, *net, *mask)) => 0,
        IpAddr::V4(ip) if ip.is_private() => 1,
        IpAddr::V4(_) => 2,
        IpAddr::V6(ip) if ip.is_loopback() => 6,
        // Link-local IPv6 works only with the right interface (scope) attached.
        IpAddr::V6(ip) if ip.is_unicast_link_local() => match address {
            SocketAddr::V6(v6) if v6.scope_id() != 0 => 5,
            _ => 7,
        },
        IpAddr::V6(_) => 3,
    }
}

fn same_subnet(ip: Ipv4Addr, network: Ipv4Addr, mask: Ipv4Addr) -> bool {
    let (ip, network, mask) = (u32::from(ip), u32::from(network), u32::from(mask));
    mask != 0 && ip & mask == network & mask
}

/// This machine's IPv4 interfaces as (address, netmask).
fn local_ipv4_networks() -> Vec<(Ipv4Addr, Ipv4Addr)> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| !interface.is_loopback())
        .filter_map(|interface| match interface.addr {
            if_addrs::IfAddr::V4(v4) => Some((v4.ip, v4.netmask)),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::{Ipv6Addr, SocketAddrV6};

    use super::*;

    const WIFI: (Ipv4Addr, Ipv4Addr) = (
        Ipv4Addr::new(192, 168, 1, 20),
        Ipv4Addr::new(255, 255, 255, 0),
    );

    fn v4(a: u8, b: u8, c: u8, d: u8) -> SocketAddr {
        SocketAddr::from(([a, b, c, d], 4000))
    }

    fn link_local_v6(scope: u32) -> SocketAddr {
        SocketAddr::V6(SocketAddrV6::new(
            "fe80::85a:3e9c:172e:87a3".parse::<Ipv6Addr>().unwrap(),
            4000,
            0,
            scope,
        ))
    }

    #[test]
    fn same_subnet_ipv4_beats_docker_bridges() {
        // What a Fedora machine running Docker announces.
        let announced = [v4(172, 17, 0, 1), v4(172, 20, 0, 1), v4(192, 168, 1, 34)];
        assert_eq!(rank_with(announced, &[WIFI])[0], v4(192, 168, 1, 34));
    }

    #[test]
    fn ipv4_beats_link_local_ipv6() {
        // What a Mac announces: a link-local IPv6 address first, its Wi-Fi IPv4 later.
        let announced = [link_local_v6(3), v4(192, 168, 1, 40)];
        assert_eq!(
            rank_with(announced, &[WIFI]),
            [v4(192, 168, 1, 40), link_local_v6(3)]
        );
    }

    #[test]
    fn link_local_ipv6_without_scope_is_tried_last() {
        let announced = [link_local_v6(0), link_local_v6(3)];
        assert_eq!(
            rank_with(announced, &[]),
            [link_local_v6(3), link_local_v6(0)]
        );
    }

    #[test]
    fn duplicates_are_removed() {
        let announced = [v4(192, 168, 1, 40), v4(192, 168, 1, 40)];
        assert_eq!(rank_with(announced, &[WIFI]).len(), 1);
    }

    #[test]
    fn subnet_math() {
        let mask = Ipv4Addr::new(255, 255, 255, 0);
        assert!(same_subnet(Ipv4Addr::new(192, 168, 1, 9), WIFI.0, mask));
        assert!(!same_subnet(Ipv4Addr::new(192, 168, 2, 9), WIFI.0, mask));
        assert!(!same_subnet(
            Ipv4Addr::new(10, 0, 0, 1),
            WIFI.0,
            Ipv4Addr::UNSPECIFIED
        ));
    }
}
