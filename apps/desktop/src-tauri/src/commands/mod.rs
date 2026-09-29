//! Tauri command groups.
//!
//! Commands are grouped by the workspace they serve, so a frontend module and a
//! backend module have a one-to-one correspondence. Business logic lives in the
//! `hex-*` crates; these functions convert between their typed structures and the
//! JSON the interface sends and receives.

pub mod analysis;
pub mod flight;
pub mod model;
pub mod project;
pub mod simulation;
pub mod telemetry;

/// Every channel name the plot panels can request, in display order.
pub fn viewer_channels() -> Vec<String> {
    simulation::viewer_channel_names()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_viewer_channel_list_is_shared_and_complete() {
        let channels = viewer_channels();
        assert!(channels.len() >= 30);
        for name in ["time", "altitude", "speed", "force", "moment"] {
            assert!(channels.contains(&name.to_string()), "{} missing", name);
        }
        assert_eq!(channels, simulation::all_channel_names());
    }

    #[test]
    fn every_command_module_exposes_its_group() {
        // A compile-time check that each module resolves, plus a smoke test of one
        // pure function from each group.
        assert!(!project::project_schema_version().is_empty());
        assert!(!model::model_example().unwrap().is_empty());
        assert!(simulation::selector_from_name("time").is_some());
        assert!(!telemetry::telemetry_destinations().is_empty());
        assert!(!flight::flight_role_options().is_empty());
        assert!(!analysis::comparison_channel_codes().is_empty());
    }
}
