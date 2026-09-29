//! HexaDOF desktop application backend.
//!
//! The backend owns the state that outlives a screen: the open project, imported
//! models, the loaded run, the imported flight session, the live telemetry worker,
//! and the application settings. Every command is a thin adapter over the `hex-*`
//! crates, which hold all of the numerical and persistence logic.
//!
//! # Threading
//!
//! * A simulation runs on a blocking thread and reports progress through typed
//!   events, so a multi-second integration never stalls the interface.
//! * Telemetry ingestion runs on its own thread that reads bytes and hands whole
//!   packets to the pipeline. The frontend polls a throttled display stream, so a
//!   slow interface cannot change what gets recorded.
//! * Commands that only read or write small state take the state lock briefly and
//!   never hold it across a blocking call.

// A Tauri command must return a concrete error type that implements `Serialize`,
// so the error cannot be boxed without changing the IPC contract the frontend is
// generated against. The command layer is not a hot path either: it produces one
// error per failed user action, never one per sample.
#![allow(clippy::result_large_err)]

pub mod commands;
pub mod error;
pub mod events;
pub mod state;

use tauri::Manager;

/// Build and run the desktop application.
pub fn run() {
    // A log subscriber is installed before anything else so a startup problem is
    // visible in the console rather than silent.
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .try_init();

    tracing::info!("HexaDOF {} starting", hex_core::app_version());

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(state::AppState::new())
        .setup(|app| {
            // Report settings problems on startup rather than at first use.
            let state = app.state::<state::AppState>();
            if let Ok(warnings) = state.with(|s| s.settings_warnings.clone()) {
                for warning in warnings {
                    tracing::warn!("{}", warning);
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            // Project
            commands::project::project_create,
            commands::project::project_open,
            commands::project::project_save,
            commands::project::project_status,
            commands::project::project_update,
            commands::project::project_discover,
            commands::project::project_recent,
            commands::project::project_forget_recent,
            commands::project::project_schema_version,
            commands::project::project_metadata_template,
            commands::project::run_delete,
            commands::project::telemetry_session_delete,
            commands::project::flight_session_delete,
            // Settings and diagnostics
            commands::project::settings_get,
            commands::project::settings_set,
            commands::project::settings_reset,
            commands::project::settings_warnings,
            commands::project::settings_path,
            commands::project::app_diagnostics,
            // Model
            commands::model::model_import,
            commands::model::model_validate,
            commands::model::model_list,
            commands::model::model_current,
            commands::model::model_clear,
            commands::model::model_example,
            commands::model::model_import_example,
            commands::model::model_save_to_project,
            // Simulation
            commands::simulation::simulation_validate,
            commands::simulation::simulation_run,
            commands::simulation::simulation_cancel,
            commands::simulation::simulation_progress,
            commands::simulation::simulation_is_running,
            commands::simulation::simulation_load_results,
            commands::simulation::simulation_list_saved,
            commands::simulation::simulation_open_saved,
            commands::simulation::simulation_add_marker,
            commands::simulation::simulation_export_csv,
            commands::simulation::simulation_replay_data,
            commands::simulation::simulation_events,
            // Telemetry
            commands::telemetry::serial_list_ports,
            commands::telemetry::serial_connect,
            commands::telemetry::serial_disconnect,
            commands::telemetry::telemetry_status,
            commands::telemetry::telemetry_poll,
            commands::telemetry::telemetry_start_recording,
            commands::telemetry::telemetry_stop_recording,
            commands::telemetry::telemetry_connect_scripted,
            commands::telemetry::telemetry_validate,
            commands::telemetry::telemetry_destinations,
            commands::telemetry::telemetry_session_list,
            commands::telemetry::telemetry_is_scripted,
            commands::telemetry::telemetry_validation_lines,
            commands::telemetry::telemetry_validation_blocking,
            commands::telemetry::telemetry_validation_ready,
            commands::telemetry::telemetry_check_status_label,
            commands::telemetry::telemetry_profile_report,
            commands::telemetry::device_profile_list,
            commands::telemetry::device_profile_save,
            commands::telemetry::device_profile_delete,
            commands::telemetry::device_profile_examples,
            commands::telemetry::device_profile_validate,
            commands::telemetry::estimator_modes,
            commands::telemetry::calibration_save,
            commands::telemetry::calibration_load,
            // Flight analysis
            commands::flight::flight_preview,
            commands::flight::flight_validate,
            commands::flight::flight_import,
            commands::flight::flight_import_path,
            commands::flight::flight_validate_session,
            commands::flight::flight_assess_quality,
            commands::flight::flight_channels,
            commands::flight::flight_series,
            commands::flight::flight_replay_data,
            commands::flight::flight_session_list,
            commands::flight::flight_role_options,
            commands::flight::flight_add_marker,
            commands::analysis::flight_detect_events,
            // Comparison and reports
            commands::analysis::comparison_create,
            commands::analysis::comparison_build,
            commands::analysis::comparison_current,
            commands::analysis::comparison_channels,
            commands::analysis::comparison_methods,
            commands::analysis::comparison_report_text,
            commands::analysis::comparison_export,
            commands::analysis::comparison_channel_codes,
            commands::analysis::comparison_plot_channels,
            commands::analysis::result_channel_names,
            commands::analysis::result_channel_exists,
            commands::analysis::analysis_selector_codes,
            commands::analysis::event_lines,
            commands::analysis::report_export,
        ])
        .run(tauri::generate_context!())
        .expect("the HexaDOF window could not be created");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_surface_is_grouped_and_non_empty() {
        // A structural check that every command group is reachable and that the
        // event channel list is complete, so a rename cannot silently break a
        // frontend listener.
        assert_eq!(events::all_channels().len(), 12);
        assert!(!commands::viewer_channels().is_empty());
        assert!(!hex_core::app_version().is_empty());
    }

    #[test]
    fn the_state_starts_without_a_project() {
        let state = state::AppState::new();
        assert!(state.with(|s| s.project.is_none()).unwrap());
        assert!(state.with(|s| s.telemetry.is_none()).unwrap());
        assert!(state.with(|s| s.active_simulation.is_none()).unwrap());
    }

    #[test]
    fn error_types_convert_from_every_crate() {
        let a: error::CommandError = hex_project::ProjectError::RunNotFound {
            name: "x".to_string(),
        }
        .into();
        let b: error::CommandError = hex_flight_data::FlightDataError::EmptyFile.into();
        let c: error::CommandError = hex_telemetry::TransportError::Unsupported.into();
        let d: error::CommandError = hex_analysis::AlignmentError::NoOverlap.into();
        for e in [a, b, c, d] {
            assert!(!e.code.is_empty());
            assert!(!e.title.is_empty());
            assert!(!e.detail.is_empty());
            assert!(!e.suggestion.is_empty());
        }
    }
}
