//! Address ranges for discovery (SPEC §5.1): IPv4 CIDR blocks, single
//! addresses and dash ranges (`10.0.0.10-10.0.0.20` or `10.0.0.10-20`).

use std::collections::BTreeSet;
use std::net::Ipv4Addr;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RangeError {
    #[error("enter at least one address range")]
    Empty,
    #[error("\"{0}\" isn't an IPv4 address, CIDR block (e.g. 192.0.2.0/24) or range (e.g. 192.0.2.10-20)")]
    Syntax(String),
    #[error("\"{0}\": the range ends before it starts")]
    Backwards(String),
    #[error("\"{0}\": prefixes shorter than /8 are too large to scan")]
    TooWide(String),
}

/// One parsed range, as an inclusive span of addresses to scan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Span {
    first: u32,
    last: u32,
}

fn parse_one(input: &str) -> Result<Span, RangeError> {
    let s = input.trim();
    let syntax = || RangeError::Syntax(s.to_owned());
    if let Some((addr, prefix)) = s.split_once('/') {
        let base: Ipv4Addr = addr.trim().parse().map_err(|_| syntax())?;
        let prefix: u32 = prefix.trim().parse().map_err(|_| syntax())?;
        if prefix > 32 {
            return Err(syntax());
        }
        if prefix < 8 {
            return Err(RangeError::TooWide(s.to_owned()));
        }
        let mask = if prefix == 0 {
            0
        } else {
            u32::MAX << (32 - prefix)
        };
        let network = u32::from(base) & mask;
        let broadcast = network | !mask;
        // Skip the network and broadcast addresses of real subnets.
        return Ok(if prefix <= 30 {
            Span {
                first: network + 1,
                last: broadcast - 1,
            }
        } else {
            Span {
                first: network,
                last: broadcast,
            }
        });
    }
    if let Some((a, b)) = s.split_once('-') {
        let first: Ipv4Addr = a.trim().parse().map_err(|_| syntax())?;
        let b = b.trim();
        // `10.0.0.10-20` is short for `10.0.0.10-10.0.0.20`.
        let last: Ipv4Addr = if b.contains('.') {
            b.parse().map_err(|_| syntax())?
        } else {
            let last_octet: u8 = b.parse().map_err(|_| syntax())?;
            let o = first.octets();
            Ipv4Addr::new(o[0], o[1], o[2], last_octet)
        };
        let (first, last) = (u32::from(first), u32::from(last));
        if last < first {
            return Err(RangeError::Backwards(s.to_owned()));
        }
        return Ok(Span { first, last });
    }
    let ip: Ipv4Addr = s.parse().map_err(|_| syntax())?;
    Ok(Span {
        first: u32::from(ip),
        last: u32::from(ip),
    })
}

/// Parsed, de-duplicated ranges.
#[derive(Debug, Clone)]
pub struct Ranges {
    spans: Vec<Span>,
}

impl Ranges {
    /// Parse ranges given one per entry (entries may also hold several,
    /// separated by commas, spaces or newlines).
    pub fn parse(inputs: &[String]) -> Result<Self, RangeError> {
        let mut spans = Vec::new();
        for entry in inputs {
            for part in entry.split(|c: char| c == ',' || c.is_whitespace()) {
                if !part.trim().is_empty() {
                    spans.push(parse_one(part)?);
                }
            }
        }
        if spans.is_empty() {
            return Err(RangeError::Empty);
        }
        // Merge overlaps so the count (and the scan) doesn't repeat addresses.
        spans.sort_by_key(|s| s.first);
        let mut merged: Vec<Span> = Vec::new();
        for s in spans {
            match merged.last_mut() {
                Some(m) if u64::from(s.first) <= u64::from(m.last) + 1 => {
                    m.last = m.last.max(s.last)
                }
                _ => merged.push(s),
            }
        }
        Ok(Self { spans: merged })
    }

    /// How many addresses would be scanned (without expanding them).
    pub fn count(&self) -> u64 {
        self.spans
            .iter()
            .map(|s| u64::from(s.last) - u64::from(s.first) + 1)
            .sum()
    }

    /// Every address, in order. Call only after checking `count()`.
    pub fn addresses(&self) -> impl Iterator<Item = Ipv4Addr> + '_ {
        self.spans
            .iter()
            .flat_map(|s| (s.first..=s.last).map(Ipv4Addr::from))
    }
}

/// Unique, sorted list (tests and display).
pub fn expand(inputs: &[&str]) -> Result<Vec<Ipv4Addr>, RangeError> {
    let owned: Vec<String> = inputs.iter().map(|s| (*s).to_owned()).collect();
    Ok(Ranges::parse(&owned)?
        .addresses()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count(inputs: &[&str]) -> Result<u64, RangeError> {
        let owned: Vec<String> = inputs.iter().map(|s| (*s).to_owned()).collect();
        Ranges::parse(&owned).map(|r| r.count())
    }

    #[test]
    fn cidr_skips_network_and_broadcast() {
        assert_eq!(count(&["192.0.2.0/24"]), Ok(254));
        // 192.0.2.76/30: network .76 and broadcast .79 are skipped.
        assert_eq!(
            expand(&["192.0.2.77/30"]).unwrap(),
            [Ipv4Addr::new(192, 0, 2, 77), Ipv4Addr::new(192, 0, 2, 78)]
        );
        assert_eq!(count(&["192.0.2.5/32"]), Ok(1));
        assert_eq!(count(&["192.0.2.4/31"]), Ok(2));
        assert_eq!(count(&["10.0.0.0/20"]), Ok(4094));
    }

    #[test]
    fn singles_and_dash_ranges() {
        assert_eq!(
            expand(&["192.0.2.10-12", "198.51.100.1"]).unwrap(),
            [
                Ipv4Addr::new(192, 0, 2, 10),
                Ipv4Addr::new(192, 0, 2, 11),
                Ipv4Addr::new(192, 0, 2, 12),
                Ipv4Addr::new(198, 51, 100, 1),
            ]
        );
        assert_eq!(count(&["192.0.2.250-192.0.3.5"]), Ok(12));
        assert_eq!(count(&["192.0.2.1, 192.0.2.2\n192.0.2.3"]), Ok(3));
    }

    #[test]
    fn overlaps_count_once() {
        assert_eq!(
            count(&["192.0.2.0/24", "192.0.2.10-20", "192.0.2.255"]),
            Ok(255)
        );
    }

    #[test]
    fn rejects_bad_input() {
        assert_eq!(count(&[]), Err(RangeError::Empty));
        assert_eq!(count(&["  "]), Err(RangeError::Empty));
        assert!(matches!(
            count(&["camera.local"]),
            Err(RangeError::Syntax(_))
        ));
        assert!(matches!(
            count(&["192.0.2.0/33"]),
            Err(RangeError::Syntax(_))
        ));
        assert!(matches!(
            count(&["2001:db8::/64"]),
            Err(RangeError::Syntax(_))
        ));
        assert!(matches!(
            count(&["192.0.2.20-10"]),
            Err(RangeError::Backwards(_))
        ));
        assert!(matches!(count(&["0.0.0.0/0"]), Err(RangeError::TooWide(_))));
        // Huge but countable without allocating.
        assert_eq!(count(&["10.0.0.0/8"]), Ok(16_777_214));
    }
}
