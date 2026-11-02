//! Shared visualization identities and deferred desktop workload commands.
//!
//! A catalogue maps the graphical choices to prepared host commands. It never
//! starts a workload: the selected environment owns execution, and the chooser
//! only returns an identity. Browser hosts use those same identities without
//! desktop commands.
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::{ffi::OsString, path::PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Visualization {
    #[serde(rename = "client-movement")]
    Movement,
    #[serde(rename = "service-uptime")]
    Uptime,
    #[serde(rename = "account-balances")]
    Accounts,
    #[serde(rename = "robotics")]
    Robotics,
}
impl Visualization {
    pub const ALL: [Self; 4] = [Self::Movement, Self::Uptime, Self::Accounts, Self::Robotics];
    pub const fn label(self) -> &'static str {
        match self {
            Self::Movement => "Client movement",
            Self::Uptime => "Service uptime",
            Self::Accounts => "Account balances",
            Self::Robotics => "Robotics / delayed perception",
        }
    }
    pub const fn description(self) -> &'static str {
        match self {
            Self::Robotics => {
                "A camera result arrives after the robot has moved. Compare replay from capture time with applying the same observation at the current time, and see estimates and route proposals change."
            }
            Self::Movement => {
                "Delayed movement inputs can put a client on the wrong path. Compare ConTime repairing the past with applying the same inputs only when they arrive."
            }
            Self::Accounts => {
                "Late payments can change provisional decisions. Compare timestamp reconciliation with arrival order, and see identical retries leave balances unchanged."
            }
            Self::Uptime => {
                "A late health report can reveal an outage after recovery. Compare corrected service history and downtime with the history built in arrival order."
            }
        }
    }
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Movement => "client-movement",
            Self::Uptime => "service-uptime",
            Self::Accounts => "account-balances",
            Self::Robotics => "robotics",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Navigation {
    Reset,
    Menu,
}
impl Navigation {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Reset => "reset",
            Self::Menu => "menu",
        }
    }
}
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Launch {
    pub visualization: Visualization,
    pub program: OsString,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
}
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Catalog {
    pub launches: Vec<Launch>,
}
#[cfg(not(target_arch = "wasm32"))]
impl Catalog {
    pub fn validate(&self) -> Result<(), String> {
        for visualization in Visualization::ALL {
            if self.launches.iter().filter(|launch| launch.visualization == visualization).count() != 1 {
                return Err(format!("Expected exactly one command for {}", visualization.slug()));
            }
        }
        if self.launches.iter().any(|launch| launch.program.is_empty()) {
            return Err("Visualization workload executable is empty".into());
        }
        Ok(())
    }
}
#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;
    fn launch(visualization: Visualization) -> Launch {
        Launch { visualization, program: "workload".into(), args: vec![], cwd: ".".into() }
    }
    #[test]
    fn catalog_requires_each_visualization_exactly_once() {
        assert!(Catalog { launches: vec![] }.validate().is_err());
        assert!(Catalog { launches: vec![launch(Visualization::Movement), launch(Visualization::Movement)] }.validate().is_err());
        let catalog = Catalog { launches: Visualization::ALL.map(launch).into() };
        catalog.validate().unwrap();
        let roundtrip: Catalog = serde_json::from_str(&serde_json::to_string(&catalog).unwrap()).unwrap();
        roundtrip.validate().unwrap();
        assert_eq!(roundtrip.launches[1].visualization.slug(), "service-uptime");
    }
}
