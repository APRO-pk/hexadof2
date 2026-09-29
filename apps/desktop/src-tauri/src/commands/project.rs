//! Project and settings commands.

use hex_project::{
    delete_flight_session, delete_run, delete_telemetry_session, discover_projects, read_metadata,
    ProjectInventory, ProjectMetadata, Settings,
};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::{missing, CommandError};
use crate::events;
use crate::state::{emit_notice, AppState};

/// What the frontend needs to render the project status.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectView {
    /// Project name.
    pub name: String,
    /// Project description.
    pub description: String,
    /// Absolute project directory.
    pub root: String,
    /// Project identifier.
    pub project_id: String,
    /// Schema version on disk.
    pub schema_version: String,
    /// ISO 8601 creation time.
    pub created_at: String,
    /// ISO 8601 last modification time.
    pub updated_at: String,
    /// Project-relative model reference, when one is set.
    pub model_reference: Option<String>,
    /// Unit system label.
    pub unit_system: String,
    /// World frame label.
    pub world_frame: String,
    /// Body frame label.
    pub body_frame: String,
    /// Whether metadata has unsaved changes.
    pub dirty: bool,
    /// Warnings raised when the project was opened.
    pub warnings: Vec<String>,
    /// Artifact counts.
    pub inventory: ProjectInventory,
    /// Whether every required directory exists.
    pub layout_complete: bool,
}

/// A project found on disk without opening it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DiscoveredProject {
    /// Absolute directory.
    pub root: String,
    /// Project name from the metadata.
    pub name: String,
    /// ISO 8601 last modification time.
    pub updated_at: String,
    /// Whether the metadata could be read.
    pub readable: bool,
    /// Why it could not be read, when it could not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

impl ProjectView {
    fn from_project(project: &hex_project::Project) -> Self {
        Self {
            name: project.metadata.name.clone(),
            description: project.metadata.description.clone(),
            root: project.root().to_string_lossy().to_string(),
            project_id: project.metadata.project_id.clone(),
            schema_version: project.metadata.schema_version.clone(),
            created_at: project.metadata.created_at.clone(),
            updated_at: project.metadata.updated_at.clone(),
            model_reference: project.metadata.model_reference.clone(),
            unit_system: project.metadata.unit_system.label().to_string(),
            world_frame: project.metadata.world_frame.label().to_string(),
            body_frame: project.metadata.body_frame.label().to_string(),
            dirty: project.is_dirty(),
            warnings: project.open_warnings.clone(),
            inventory: project.inventory(),
            layout_complete: project.layout_is_complete(),
        }
    }
}

/// Create a project in a new directory.
#[tauri::command]
pub fn project_create(
    app: AppHandle,
    state: State<'_, AppState>,
    directory: String,
    name: String,
) -> Result<ProjectView, CommandError> {
    if directory.trim().is_empty() {
        return Err(crate::error::invalid(
            "A project directory is required.",
            "Choose an empty folder for the project.",
        ));
    }
    if name.trim().is_empty() {
        return Err(crate::error::invalid(
            "A project name is required.",
            "Type a name for the project.",
        ));
    }
    let project = hex_project::Project::create(&directory, &name)?;
    let view = ProjectView::from_project(&project);

    state.with(|s| {
        s.project = Some(project);
        s.models.clear();
        s.flight = None;
        s.flight_name = None;
        s.last_run = None;
        s.comparison = None;
        // Creating a project is a deliberate act, so it becomes the most recent.
        s.settings.remember_project(&view.root);
        let _ = s.settings.save_default();
    })?;

    emit_notice(
        &app,
        events::NoticeEvent::info("Project created", format!("{} at {}", view.name, view.root)),
    );
    Ok(view)
}

/// Open an existing project directory.
#[tauri::command]
pub fn project_open(
    app: AppHandle,
    state: State<'_, AppState>,
    directory: String,
) -> Result<ProjectView, CommandError> {
    let project = hex_project::Project::open(&directory)?;
    let view = ProjectView::from_project(&project);

    state.with(|s| {
        s.project = Some(project);
        s.models.clear();
        s.flight = None;
        s.flight_name = None;
        s.last_run = None;
        s.comparison = None;
        s.settings.remember_project(&view.root);
        let _ = s.settings.save_default();
        Ok::<(), CommandError>(())
    })??;

    for warning in &view.warnings {
        emit_notice(
            &app,
            events::NoticeEvent::warning("Project opened with a note", warning.clone()),
        );
    }
    emit_notice(
        &app,
        events::NoticeEvent::info("Project opened", view.name.to_string()),
    );
    Ok(view)
}

/// Write the project metadata.
#[tauri::command]
pub fn project_save(state: State<'_, AppState>) -> Result<ProjectView, CommandError> {
    state.with(|s| {
        let project = s.project.as_mut().ok_or_else(|| {
            missing(
                "project.none_open",
                "No project is open",
                "There is nothing to save.",
                "Create or open a project first.",
            )
        })?;
        project.save()?;
        Ok(ProjectView::from_project(project))
    })?
}

/// Read the current project status without changing anything.
#[tauri::command]
pub fn project_status(state: State<'_, AppState>) -> Result<Option<ProjectView>, CommandError> {
    state.with(|s| s.project.as_ref().map(ProjectView::from_project))
}

/// Update the project metadata.
#[tauri::command]
pub fn project_update(
    state: State<'_, AppState>,
    name: Option<String>,
    description: Option<String>,
    save: Option<bool>,
) -> Result<ProjectView, CommandError> {
    state.with(|s| {
        let project = s.project.as_mut().ok_or_else(|| {
            missing(
                "project.none_open",
                "No project is open",
                "There is nothing to update.",
                "Create or open a project first.",
            )
        })?;
        project.update(|m| {
            if let Some(n) = &name {
                m.name = n.clone();
            }
            if let Some(d) = &description {
                m.description = d.clone();
            }
        });
        if save.unwrap_or(true) {
            project.save()?;
        }
        Ok(ProjectView::from_project(project))
    })?
}

/// Find projects under a directory.
#[tauri::command]
pub fn project_discover(
    directory: String,
    max_depth: Option<usize>,
) -> Result<Vec<DiscoveredProject>, CommandError> {
    let roots = discover_projects(&directory, max_depth.unwrap_or(3));
    let mut found = Vec::with_capacity(roots.len());
    for root in roots {
        let root_string = root.to_string_lossy().to_string();
        match read_metadata(&root) {
            Ok(metadata) => found.push(DiscoveredProject {
                root: root_string,
                name: metadata.name,
                updated_at: metadata.updated_at,
                readable: true,
                problem: None,
            }),
            Err(e) => found.push(DiscoveredProject {
                root: root_string,
                name: "(unreadable project)".to_string(),
                updated_at: String::new(),
                readable: false,
                problem: Some(e.user_message()),
            }),
        }
    }
    Ok(found)
}

/// Report the recent projects that still exist.
#[tauri::command]
pub fn project_recent(state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    state.with(|s| {
        s.settings
            .general
            .existing_recent_projects()
            .into_iter()
            .map(|p| p.to_string())
            .collect()
    })
}

/// Forget a recent project.
#[tauri::command]
pub fn project_forget_recent(
    state: State<'_, AppState>,
    directory: String,
) -> Result<Vec<String>, CommandError> {
    state.with(|s| {
        s.settings.general.forget_project(&directory);
        let _ = s.settings.save_default();
        s.settings.general.recent_projects.to_vec()
    })
}

/// Delete a saved run.
#[tauri::command]
pub fn run_delete(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<usize, CommandError> {
    let project = state.require_project()?;
    delete_run(&project, &name)?;
    let remaining = hex_project::list_runs(&project).len();
    let cleared = state.with(|s| {
        let matches = s
            .last_run
            .as_ref()
            .and_then(|r| r.artifact_name.as_deref())
            .map(|n| n == name)
            .unwrap_or(false);
        if matches {
            s.last_run = None;
        }
        matches
    })?;
    if cleared {
        emit_notice(
            &app,
            events::NoticeEvent::info("Run deleted", format!("{} was removed.", name)),
        );
    }
    Ok(remaining)
}

/// Delete a telemetry session.
#[tauri::command]
pub fn telemetry_session_delete(
    state: State<'_, AppState>,
    name: String,
) -> Result<(), CommandError> {
    let project = state.require_project()?;
    delete_telemetry_session(&project, &name)?;
    Ok(())
}

/// Delete a flight session.
#[tauri::command]
pub fn flight_session_delete(state: State<'_, AppState>, name: String) -> Result<(), CommandError> {
    let project = state.require_project()?;
    delete_flight_session(&project, &name)?;
    state.with(|s| {
        if s.flight_name.as_deref() == Some(name.as_str()) {
            s.flight = None;
            s.flight_name = None;
        }
    })
}

/// Load the application settings.
#[tauri::command]
pub fn settings_get(state: State<'_, AppState>) -> Result<Settings, CommandError> {
    state.with(|s| s.settings.clone())
}

/// Warnings raised while loading settings.
#[tauri::command]
pub fn settings_warnings(state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    state.with(|s| s.settings_warnings.clone())
}

/// Replace the application settings and write them to disk.
#[tauri::command]
pub fn settings_set(
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<Vec<String>, CommandError> {
    let problems = settings.validate();
    if !problems.is_empty() {
        return Err(crate::error::invalid(
            problems.join("; "),
            "Correct the highlighted settings and save again.",
        ));
    }
    settings.save_default()?;
    state.with(|s| {
        s.settings = settings;
        s.settings_warnings.clear();
        s.settings.validate()
    })
}

/// Restore the default settings.
#[tauri::command]
pub fn settings_reset(state: State<'_, AppState>) -> Result<Settings, CommandError> {
    let defaults = Settings::default();
    defaults.save_default()?;
    state.with(|s| {
        s.settings = defaults.clone();
        s.settings_warnings.clear();
    })?;
    Ok(defaults)
}

/// The settings file path, shown in the diagnostics panel.
#[tauri::command]
pub fn settings_path() -> String {
    Settings::default_path().to_string_lossy().to_string()
}

/// The application and environment description for the diagnostics panel.
#[tauri::command]
pub fn app_diagnostics(state: State<'_, AppState>) -> Result<Vec<String>, CommandError> {
    state.with(|s| {
        let mut lines = vec![
            format!("HexaDOF {}", hex_core::app_version()),
            format!("Settings: {}", Settings::default_path().display()),
            hex_dynamics::capability_summary(),
            hex_telemetry::capability_summary(),
            hex_analysis::capability_summary(),
            hex_project::capability_summary(),
        ];
        lines.extend(hex_project::describe_environment(s.project.as_ref()));
        lines.push(format!("Event channels: {}", events::all_channels().len()));
        if let Some(worker) = &s.telemetry {
            lines.push(format!("Telemetry port: {}", worker.port));
        }
        if !s.settings_warnings.is_empty() {
            for w in &s.settings_warnings {
                lines.push(format!("Settings warning: {}", w));
            }
        }
        lines
    })
}

/// The project metadata schema version this build writes.
#[tauri::command]
pub fn project_schema_version() -> String {
    hex_project::PROJECT_SCHEMA_VERSION.to_string()
}

/// A complete metadata document for reference, shown in the project panel.
#[tauri::command]
pub fn project_metadata_template(name: String) -> Result<ProjectMetadata, CommandError> {
    Ok(ProjectMetadata::new(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-commands-{}-{}-{}",
                label,
                std::process::id(),
                hex_project::new_id()
            ));
            std::fs::create_dir_all(&base).expect("temp dir");
            Self(base)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn project_view_reports_everything_the_ui_needs() {
        let temp = TempDir::new("view");
        let project = hex_project::Project::create(temp.path().join("p"), "View test").unwrap();
        let view = ProjectView::from_project(&project);
        assert_eq!(view.name, "View test");
        assert_eq!(view.schema_version, hex_project::PROJECT_SCHEMA_VERSION);
        assert!(view.layout_complete);
        assert!(view.inventory.is_empty());
        assert!(!view.dirty);
        assert!(view.world_frame.contains("ENU"));
        assert!(view.body_frame.contains("forward"));
        assert!(view.root.ends_with("p"));
        assert_eq!(view.unit_system, "SI (m, kg, s, rad)");
    }

    #[test]
    fn discover_reports_unreadable_projects_rather_than_hiding_them() {
        let temp = TempDir::new("discover");
        hex_project::Project::create(temp.path().join("good"), "Good").unwrap();
        // A directory with a project file that cannot be parsed.
        let broken = temp.path().join("broken");
        std::fs::create_dir_all(&broken).unwrap();
        std::fs::write(broken.join(hex_project::PROJECT_FILE_NAME), b"{not json").unwrap();

        let found = project_discover(temp.path().to_string_lossy().to_string(), Some(3)).unwrap();
        assert_eq!(found.len(), 2);
        let good = found.iter().find(|p| p.root.ends_with("good")).unwrap();
        assert!(good.readable);
        assert_eq!(good.name, "Good");
        let bad = found.iter().find(|p| p.root.ends_with("broken")).unwrap();
        assert!(!bad.readable);
        assert!(bad.problem.is_some());
    }

    #[test]
    fn discover_of_an_empty_directory_finds_nothing() {
        let temp = TempDir::new("empty");
        let found = project_discover(temp.path().to_string_lossy().to_string(), None).unwrap();
        assert!(found.is_empty());
    }

    #[test]
    fn settings_commands_validate_before_saving() {
        let mut settings = Settings::default();
        settings.telemetry.packet_timeout_seconds = -1.0;
        assert!(!settings.validate().is_empty());
        // The command rejects it, so the stored settings are untouched.
        let error = crate::error::invalid(settings.validate().join("; "), "fix it");
        assert!(error.detail.contains("timeout"));
    }

    #[test]
    fn metadata_template_is_a_valid_document() {
        let metadata = project_metadata_template("Template".to_string()).unwrap();
        assert_eq!(metadata.name, "Template");
        assert!(metadata.validate().is_empty());
        assert_eq!(metadata.schema_version, hex_project::PROJECT_SCHEMA_VERSION);
    }
}
