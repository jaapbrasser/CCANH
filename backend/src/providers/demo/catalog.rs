//! The demo fleet: about 24 cameras across the Site A / Site B hierarchy from
//! SPEC §9, with a few deliberately misbehaving ones (SPEC §12.2).
//!
//! Addresses come from the RFC 5737 documentation ranges (192.0.2.0/24,
//! 198.51.100.0/24), which never route on a real network.

use super::models::ModelKey;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scenario {
    /// Always online with ordinary response-time jitter.
    Healthy,
    /// Online but slow to respond.
    Slow,
    /// Never reachable.
    Offline,
    /// Alternates online/offline every `period_secs`.
    Flapping { period_secs: u64 },
    /// Reachable, but rejects the configured credentials.
    AuthFailure,
}

#[derive(Debug, Clone, Copy)]
pub struct DemoCamera {
    pub name: &'static str,
    pub host: &'static str,
    pub model: ModelKey,
    pub scenario: Scenario,
}

const fn cam(
    name: &'static str,
    host: &'static str,
    model: ModelKey,
    scenario: Scenario,
) -> DemoCamera {
    DemoCamera {
        name,
        host,
        model,
        scenario,
    }
}

use ModelKey::*;
use Scenario::*;

pub const FLEET: &[DemoCamera] = &[
    // Site A / Building 1
    cam("A1 Crane 1 – Hook", "192.0.2.11", PtzQ6135, Healthy),
    cam("A1 Crane 1 – Cabin", "192.0.2.12", MiniDomeM3086, Healthy),
    cam("A1 Crane 2 – Hook", "192.0.2.13", PtzQ6135, Healthy),
    cam("A1 Crane 2 – Cabin", "192.0.2.14", MiniDomeM3086, Healthy),
    // Site A / Building 2
    cam("A2 Crane 3 – Hook", "192.0.2.21", BoxQ1656, Healthy),
    cam("A2 Crane 3 – Trolley", "192.0.2.22", DomeP3265, Healthy),
    cam("A2 Crane 4 – Hook", "192.0.2.23", BoxQ1656, Healthy),
    cam("A2 Crane 4 – Cabin", "192.0.2.24", MiniDomeM3086, Healthy),
    // Site A / Yard
    cam("A Yard – Gate North", "192.0.2.31", BulletP1468, Healthy),
    cam("A Yard – Gate South", "192.0.2.32", BulletP1468, Healthy),
    cam("A Yard – Overview", "192.0.2.33", BoxQ1656, Slow),
    cam("A Yard – Rail Spur", "192.0.2.34", ModularF2105, Offline),
    // Site B / Loading Area
    cam("B Loading – Dock 1", "198.51.100.11", DomeP3265, Healthy),
    cam("B Loading – Dock 2", "198.51.100.12", DomeP3265, Healthy),
    cam(
        "B Loading – Dock 3",
        "198.51.100.13",
        DomeP3265,
        Flapping { period_secs: 45 },
    ),
    cam("B Loading – Dock 4", "198.51.100.14", DomeP3265, Healthy),
    // Site B / Storage Area
    cam(
        "B Storage – Aisle 1",
        "198.51.100.21",
        MiniDomeM3086,
        Healthy,
    ),
    cam(
        "B Storage – Aisle 2",
        "198.51.100.22",
        MiniDomeM3086,
        Healthy,
    ),
    cam(
        "B Storage – Aisle 3",
        "198.51.100.23",
        MiniDomeM3086,
        Healthy,
    ),
    cam(
        "B Storage – Aisle 4",
        "198.51.100.24",
        ModularF2105,
        AuthFailure,
    ),
    cam("B Storage – Overview", "198.51.100.25", BoxQ1656, Healthy),
    // Site B / Perimeter
    cam(
        "B Perimeter – Fence East",
        "198.51.100.31",
        BulletP1468,
        Healthy,
    ),
    cam(
        "B Perimeter – Fence West",
        "198.51.100.32",
        BulletP1468,
        Offline,
    ),
    cam("B Perimeter – Entrance", "198.51.100.33", PtzQ6135, Healthy),
];

pub fn find(host: &str) -> Option<&'static DemoCamera> {
    FLEET.iter().find(|c| c.host.eq_ignore_ascii_case(host))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fleet_has_variety() {
        assert_eq!(FLEET.len(), 24);
        assert!(FLEET.iter().any(|c| c.scenario == Offline));
        assert_eq!(
            FLEET
                .iter()
                .filter(|c| matches!(c.scenario, Flapping { .. }))
                .count(),
            1
        );
        assert_eq!(
            FLEET.iter().filter(|c| c.scenario == AuthFailure).count(),
            1
        );
        let mut hosts: Vec<_> = FLEET.iter().map(|c| c.host).collect();
        hosts.sort_unstable();
        hosts.dedup();
        assert_eq!(hosts.len(), FLEET.len(), "hosts are unique");
    }

    #[test]
    fn addresses_are_documentation_ranges() {
        for c in FLEET {
            assert!(
                c.host.starts_with("192.0.2.") || c.host.starts_with("198.51.100."),
                "{} uses a routable address",
                c.name
            );
        }
    }
}
