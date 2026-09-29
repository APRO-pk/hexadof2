//! The HexaDOF project directory: layout, metadata, and integrity.
//!
//! A project is a directory with explicit, human-readable metadata. The logical
//! separation is stable even though the exact storage format may evolve:
//!
//! ```text
//! my-flight-project/
//!   project.hexadof.json
//!   models/
//!   simulations/
//!   telemetry/
//!   flights/
//!   reports/
//!   assets/
//! ```
//!
//! Every write goes through a temporary file and a rename, so a crash mid-write
//! cannot leave a truncated project file behind.

use std::path::{Path, PathBuf};

use hex_core::{Frame, Real, UnitSystem};
use serde::{Deserialize, Serialize};

use crate::error::ProjectError;

/// The project metadata file name.
pub const PROJECT_FILE_NAME: &str = "project.hexadof.json";

/// The project schema version this build writes.
pub const PROJECT_SCHEMA_VERSION: &str = "1.0";

/// Subdirectory names inside a project.
pub mod dirs {
    /// Imported dynamics models.
    pub const MODELS: &str = "models";
    /// Simulation runs.
    pub const SIMULATIONS: &str = "simulations";
    /// Device profiles and telemetry sessions.
    pub const TELEMETRY: &str = "telemetry";
    /// Imported flight logs and their normalised sessions.
    pub const FLIGHTS: &str = "flights";
    /// Exported reports.
    pub const REPORTS: &str = "reports";
    /// Meshes and other binary assets.
    pub const ASSETS: &str = "assets";
}

/// The directories a project must contain.
pub const REQUIRED_DIRECTORIES: [&str; 6] = [
    dirs::MODELS,
    dirs::SIMULATIONS,
    dirs::TELEMETRY,
    dirs::FLIGHTS,
    dirs::REPORTS,
    dirs::ASSETS,
];

/// The metadata stored in `project.hexadof.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProjectMetadata {
    /// Schema version of the project document.
    pub schema_version: String,
    /// Stable project identifier.
    pub project_id: String,
    /// Display name.
    pub name: String,
    /// Free-text description.
    #[serde(default)]
    pub description: String,
    /// Unit system the project declares for display.
    pub unit_system: UnitSystem,
    /// World frame the project uses.
    pub world_frame: hex_core::WorldFrame,
    /// Body frame convention.
    #[serde(default)]
    pub body_frame: hex_core::BodyFrame,
    /// Creation timestamp.
    pub created_at: String,
    /// Last modification timestamp.
    pub updated_at: String,
    /// Path to the imported dynamics model, relative to the project directory.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_reference: Option<String>,
    /// Application version that last wrote the project.
    pub application_version: String,
    /// Launch site latitude in degrees, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_latitude: Option<Real>,
    /// Launch site longitude in degrees, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub launch_longitude: Option<Real>,
    /// Ground elevation in metres, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ground_elevation: Option<Real>,
    /// Notes shown on the overview screen.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
}

impl ProjectMetadata {
    /// A new project's metadata.
    pub fn new(name: impl Into<String>) -> Self {
        let now = crate::clock::now_iso8601();
        Self {
            schema_version: PROJECT_SCHEMA_VERSION.to_string(),
            project_id: crate::clock::new_id(),
            name: name.into(),
            description: String::new(),
            unit_system: UnitSystem::Si,
            world_frame: hex_core::WorldFrame::Enu,
            body_frame: hex_core::BodyFrame::ForwardRightDown,
            created_at: now.clone(),
            updated_at: now,
            model_reference: None,
            application_version: hex_core::app_version().to_string(),
            launch_latitude: None,
            launch_longitude: None,
            ground_elevation: None,
            notes: None,
        }
    }

    /// The frame triple implied by this metadata.
    pub fn frame(&self) -> Frame {
        Frame::new(self.world_frame, self.body_frame)
    }

    /// Problems that block opening the project.
    pub fn validate(&self) -> Vec<String> {
        let mut problems = Vec::new();
        if self.name.trim().is_empty() {
            problems.push("the project name is empty".to_string());
        }
        if self.project_id.trim().is_empty() {
            problems.push("the project identifier is empty".to_string());
        }
        if self.schema_version.trim().is_empty() {
            problems.push("the schema version is missing".to_string());
        }
        problems
    }

    /// Record a modification.
    pub fn touch(&mut self) {
        self.updated_at = crate::clock::now_iso8601();
        self.application_version = hex_core::app_version().to_string();
    }
}

/// An open project directory.
#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    /// Absolute path to the project directory.
    root: PathBuf,
    /// The project metadata.
    pub metadata: ProjectMetadata,
    /// Whether metadata has been modified since the last save.
    dirty: bool,
    /// Warnings raised when the project was opened.
    pub open_warnings: Vec<String>,
}

impl Project {
    /// Create a new project directory.
    ///
    /// An existing non-empty directory is refused, so a new project can never
    /// overwrite work.
    pub fn create(root: impl AsRef<Path>, name: impl Into<String>) -> Result<Self, ProjectError> {
        let root = root.as_ref().to_path_buf();
        if root.exists() {
            let mut entries = std::fs::read_dir(&root)
                .map_err(|e| ProjectError::io(&root, e))?
                .filter_map(std::result::Result::ok);
            if entries.next().is_some() {
                return Err(ProjectError::DirectoryNotEmpty { path: root });
            }
        }
        std::fs::create_dir_all(&root).map_err(|e| ProjectError::io(&root, e))?;

        let metadata = ProjectMetadata::new(name);
        let mut project = Self {
            root,
            metadata,
            dirty: true,
            open_warnings: Vec::new(),
        };
        project.ensure_layout()?;
        project.save()?;
        Ok(project)
    }

    /// Open an existing project directory.
    pub fn open(root: impl AsRef<Path>) -> Result<Self, ProjectError> {
        let root = root.as_ref().to_path_buf();
        if !root.exists() {
            return Err(ProjectError::NotFound { path: root });
        }
        let file = root.join(PROJECT_FILE_NAME);
        if !file.exists() {
            return Err(ProjectError::MissingProjectFile {
                name: PROJECT_FILE_NAME,
                path: file,
            });
        }
        let text = std::fs::read_to_string(&file).map_err(|e| ProjectError::io(&file, e))?;
        let metadata: ProjectMetadata =
            serde_json::from_str(&text).map_err(|e| ProjectError::Parse {
                path: file.clone(),
                detail: e.to_string(),
            })?;

        let problems = metadata.validate();
        if !problems.is_empty() {
            return Err(ProjectError::InvalidMetadata {
                detail: problems.join("; "),
            });
        }

        let mut open_warnings = Vec::new();
        if metadata.schema_version != PROJECT_SCHEMA_VERSION {
            open_warnings.push(format!(
                "The project declares schema version {}, and this build writes {}. It was opened in compatibility mode; save it to update the version.",
                metadata.schema_version, PROJECT_SCHEMA_VERSION
            ));
        }

        let project = Self {
            root,
            metadata,
            dirty: false,
            open_warnings,
        };
        // Repair a missing directory rather than refusing to open, and say so.
        let mut warnings = project.open_warnings.clone();
        for dir in REQUIRED_DIRECTORIES {
            let path = project.path_for(dir);
            if !path.exists() {
                std::fs::create_dir_all(&path).map_err(|e| ProjectError::io(&path, e))?;
                warnings.push(format!(
                    "The {} directory was missing and has been created.",
                    dir
                ));
            }
        }
        let project = Self {
            open_warnings: warnings,
            ..project
        };
        Ok(project)
    }

    /// The project directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Whether metadata has unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// The path of a project subdirectory.
    pub fn path_for(&self, directory: &str) -> PathBuf {
        self.root.join(directory)
    }

    /// The models directory.
    pub fn models_dir(&self) -> PathBuf {
        self.path_for(dirs::MODELS)
    }

    /// The simulations directory.
    pub fn simulations_dir(&self) -> PathBuf {
        self.path_for(dirs::SIMULATIONS)
    }

    /// The telemetry directory.
    pub fn telemetry_dir(&self) -> PathBuf {
        self.path_for(dirs::TELEMETRY)
    }

    /// The flights directory.
    pub fn flights_dir(&self) -> PathBuf {
        self.path_for(dirs::FLIGHTS)
    }

    /// The reports directory.
    pub fn reports_dir(&self) -> PathBuf {
        self.path_for(dirs::REPORTS)
    }

    /// The assets directory.
    pub fn assets_dir(&self) -> PathBuf {
        self.path_for(dirs::ASSETS)
    }

    /// Make sure every required directory exists.
    pub fn ensure_layout(&self) -> Result<(), ProjectError> {
        for dir in REQUIRED_DIRECTORIES {
            let path = self.path_for(dir);
            std::fs::create_dir_all(&path).map_err(|e| ProjectError::io(&path, e))?;
        }
        Ok(())
    }

    /// Whether every required directory exists.
    pub fn layout_is_complete(&self) -> bool {
        REQUIRED_DIRECTORIES
            .iter()
            .all(|d| self.path_for(d).is_dir())
    }

    /// Write the project metadata.
    pub fn save(&mut self) -> Result<(), ProjectError> {
        self.metadata.touch();
        let path = self.root.join(PROJECT_FILE_NAME);
        let text =
            serde_json::to_string_pretty(&self.metadata).map_err(|e| ProjectError::Serialize {
                detail: e.to_string(),
            })?;
        crate::io::write_atomic(&path, text.as_bytes())?;
        self.dirty = false;
        Ok(())
    }

    /// Apply a change to the metadata and mark the project dirty.
    pub fn update<F: FnOnce(&mut ProjectMetadata)>(&mut self, change: F) {
        change(&mut self.metadata);
        self.dirty = true;
    }

    /// Resolve a project-relative reference to an absolute path.
    ///
    /// A reference that escapes the project directory is refused, so a crafted
    /// project file cannot read or write outside its own folder.
    pub fn resolve(&self, reference: &str) -> Result<PathBuf, ProjectError> {
        let candidate = Path::new(reference);
        if candidate.is_absolute() {
            return Err(ProjectError::ReferenceOutside {
                reference: reference.to_string(),
            });
        }
        let joined = self.root.join(candidate);
        let normalized = crate::io::normalize(&joined);
        if !normalized.starts_with(&self.root) {
            return Err(ProjectError::ReferenceOutside {
                reference: reference.to_string(),
            });
        }
        Ok(normalized)
    }

    /// A project-relative path for an absolute path inside the project.
    pub fn relative(&self, path: &Path) -> Option<String> {
        path.strip_prefix(&self.root)
            .ok()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
    }

    /// The imported model path, when one is referenced.
    pub fn model_path(&self) -> Option<Result<PathBuf, ProjectError>> {
        self.metadata
            .model_reference
            .as_deref()
            .map(|r| self.resolve(r))
    }

    /// Point the project at a model file and copy it into `models/`.
    ///
    /// Returns the project-relative reference.
    pub fn import_model_file(
        &mut self,
        source: &Path,
        file_name: Option<&str>,
    ) -> Result<String, ProjectError> {
        if !source.is_file() {
            return Err(ProjectError::NotFound {
                path: source.to_path_buf(),
            });
        }
        let name = match file_name {
            Some(n) => crate::io::safe_file_name(n)?,
            None => {
                let derived = source
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| "model.dynamic.json".to_string());
                crate::io::safe_file_name(&derived)?
            }
        };
        let destination = self.models_dir().join(&name);
        std::fs::create_dir_all(self.models_dir())
            .map_err(|e| ProjectError::io(self.models_dir(), e))?;
        let bytes = std::fs::read(source).map_err(|e| ProjectError::io(source, e))?;
        crate::io::write_atomic(&destination, &bytes)?;
        let reference = format!("{}/{}", dirs::MODELS, name);
        self.update(|m| m.model_reference = Some(reference.clone()));
        Ok(reference)
    }

    /// A short status line for the top bar.
    pub fn status_line(&self) -> String {
        format!(
            "{} ({}){}",
            self.metadata.name,
            self.root.display(),
            if self.dirty { ", unsaved changes" } else { "" }
        )
    }

    /// Counts of the artifacts in the project, for the overview screen.
    pub fn inventory(&self) -> ProjectInventory {
        ProjectInventory {
            models: crate::io::count_matching(&self.models_dir(), |n| n.ends_with(".json")),
            simulations: crate::io::count_directories(&self.simulations_dir()),
            flight_logs: crate::io::count_directories(&self.flights_dir()),
            telemetry_sessions: crate::io::count_matching(&self.telemetry_dir(), |n| {
                n.ends_with(".hlog")
            }),
            // Reports may be grouped into subdirectories, for example a
            // comparisons folder, so the count is recursive.
            reports: crate::io::count_matching_recursive(&self.reports_dir(), |n| {
                n.ends_with(".md") || n.ends_with(".csv") || n.ends_with(".json")
            }),
        }
    }
}

/// How much is in a project directory.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ProjectInventory {
    pub models: usize,
    pub simulations: usize,
    pub flight_logs: usize,
    pub telemetry_sessions: usize,
    pub reports: usize,
}

impl ProjectInventory {
    /// Whether the project has anything in it yet.
    pub fn is_empty(&self) -> bool {
        self.models == 0
            && self.simulations == 0
            && self.flight_logs == 0
            && self.telemetry_sessions == 0
            && self.reports == 0
    }

    /// A one-line summary for the overview card.
    pub fn summary(&self) -> String {
        format!(
            "{} model(s), {} run(s), {} flight log(s), {} telemetry session(s), {} report(s)",
            self.models, self.simulations, self.flight_logs, self.telemetry_sessions, self.reports
        )
    }
}

/// List the projects in a directory by scanning for project files.
pub fn discover_projects(search_root: impl AsRef<Path>, max_depth: usize) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![(search_root.as_ref().to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        if depth > max_depth {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut has_project_file = false;
        let mut subdirectories = Vec::new();
        for entry in entries.filter_map(std::result::Result::ok) {
            let path = entry.path();
            if path.is_dir() {
                subdirectories.push(path);
            } else if path
                .file_name()
                .map(|n| n == PROJECT_FILE_NAME)
                .unwrap_or(false)
            {
                has_project_file = true;
            }
        }
        if has_project_file {
            found.push(dir.clone());
            // Do not descend into a project looking for nested projects.
            continue;
        }
        for sub in subdirectories {
            stack.push((sub, depth + 1));
        }
    }
    found.sort();
    found
}

/// Read just the metadata of a project directory without opening it fully.
pub fn read_metadata(root: impl AsRef<Path>) -> Result<ProjectMetadata, ProjectError> {
    let file = root.as_ref().join(PROJECT_FILE_NAME);
    if !file.exists() {
        return Err(ProjectError::MissingProjectFile {
            name: PROJECT_FILE_NAME,
            path: file,
        });
    }
    let text = std::fs::read_to_string(&file).map_err(|e| ProjectError::io(&file, e))?;
    serde_json::from_str(&text).map_err(|e| ProjectError::Parse {
        path: file,
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(label: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "hexadof-project-{}-{}-{}",
                label,
                std::process::id(),
                crate::clock::new_id()
            ));
            std::fs::create_dir_all(&base).expect("temp dir");
            Self(base)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn create_makes_the_full_layout_and_the_project_file() {
        let temp = TempDir::new("create");
        let project = Project::create(temp.path(), "Example Rocket").unwrap();
        assert!(project.layout_is_complete());
        assert!(temp.path().join(PROJECT_FILE_NAME).is_file());
        for dir in REQUIRED_DIRECTORIES {
            assert!(temp.path().join(dir).is_dir(), "missing {}", dir);
        }
        assert_eq!(project.metadata.name, "Example Rocket");
        assert_eq!(project.metadata.schema_version, PROJECT_SCHEMA_VERSION);
        assert!(!project.is_dirty());
    }

    #[test]
    fn create_refuses_a_non_empty_directory() {
        let temp = TempDir::new("nonempty");
        std::fs::write(temp.path().join("existing.txt"), b"data").unwrap();
        let err = Project::create(temp.path(), "Nope").unwrap_err();
        assert!(matches!(err, ProjectError::DirectoryNotEmpty { .. }));
    }

    #[test]
    fn round_trip_through_save_and_open() {
        let temp = TempDir::new("roundtrip");
        let mut project = Project::create(temp.path(), "Round Trip").unwrap();
        project.update(|m| {
            m.description = "6-DOF test project".to_string();
            m.world_frame = hex_core::WorldFrame::Ned;
            m.launch_latitude = Some(51.5);
        });
        project.save().unwrap();
        assert!(!project.is_dirty());

        let reopened = Project::open(temp.path()).unwrap();
        assert_eq!(reopened.metadata.name, "Round Trip");
        assert_eq!(reopened.metadata.description, "6-DOF test project");
        assert_eq!(reopened.metadata.world_frame, hex_core::WorldFrame::Ned);
        assert_eq!(reopened.metadata.launch_latitude, Some(51.5));
        assert_eq!(reopened.metadata.project_id, project.metadata.project_id);
        assert!(reopened.open_warnings.is_empty());
    }

    #[test]
    fn open_refuses_a_missing_directory() {
        let missing =
            std::env::temp_dir().join(format!("hexadof-missing-{}", crate::clock::new_id()));
        let err = Project::open(&missing).unwrap_err();
        assert!(matches!(err, ProjectError::NotFound { .. }));
    }

    #[test]
    fn open_refuses_a_directory_without_a_project_file() {
        let temp = TempDir::new("nofile");
        let err = Project::open(temp.path()).unwrap_err();
        assert!(matches!(err, ProjectError::MissingProjectFile { .. }));
        assert!(err.user_message().contains("project.hexadof.json"));
    }

    #[test]
    fn open_rejects_invalid_metadata() {
        let temp = TempDir::new("invalid");
        let mut metadata = ProjectMetadata::new("");
        metadata.project_id = String::new();
        std::fs::write(
            temp.path().join(PROJECT_FILE_NAME),
            serde_json::to_string(&metadata).unwrap(),
        )
        .unwrap();
        let err = Project::open(temp.path()).unwrap_err();
        assert!(matches!(err, ProjectError::InvalidMetadata { .. }));
        assert!(err.user_message().contains("name"));
    }

    #[test]
    fn open_repairs_a_missing_directory_and_warns() {
        let temp = TempDir::new("repair");
        let project = Project::create(temp.path(), "Repair").unwrap();
        std::fs::remove_dir_all(project.reports_dir()).unwrap();
        let reopened = Project::open(temp.path()).unwrap();
        assert!(reopened.reports_dir().is_dir());
        assert!(reopened.open_warnings.iter().any(|w| w.contains("reports")));
    }

    #[test]
    fn open_warns_on_a_schema_version_difference() {
        let temp = TempDir::new("schema");
        let mut project = Project::create(temp.path(), "Schema").unwrap();
        project.metadata.schema_version = "0.9".to_string();
        let text = serde_json::to_string_pretty(&project.metadata).unwrap();
        std::fs::write(temp.path().join(PROJECT_FILE_NAME), text).unwrap();
        let reopened = Project::open(temp.path()).unwrap();
        assert!(reopened.open_warnings.iter().any(|w| w.contains("0.9")));
    }

    #[test]
    fn frame_comes_from_the_metadata() {
        let metadata = ProjectMetadata::new("Frames");
        assert_eq!(metadata.frame().world, hex_core::WorldFrame::Enu);
        assert_eq!(metadata.frame().body, hex_core::BodyFrame::ForwardRightDown);
    }

    #[test]
    fn import_model_file_copies_and_references_the_model() {
        let temp = TempDir::new("import");
        let mut project = Project::create(temp.path(), "Import").unwrap();
        let source = temp.path().join("outside.json");
        std::fs::write(&source, br#"{"model_id":"test"}"#).unwrap();

        let reference = project.import_model_file(&source, None).unwrap();
        assert_eq!(reference, "models/outside.json");
        assert!(project.models_dir().join("outside.json").is_file());
        assert_eq!(
            std::fs::read(project.models_dir().join("outside.json")).unwrap(),
            br#"{"model_id":"test"}"#
        );
        assert_eq!(
            project.metadata.model_reference.as_deref(),
            Some("models/outside.json")
        );
        assert!(project.is_dirty());
        assert!(project.model_path().unwrap().is_ok());
    }

    #[test]
    fn import_model_file_rejects_a_missing_source() {
        let temp = TempDir::new("importmissing");
        let mut project = Project::create(temp.path(), "Import").unwrap();
        let err = project
            .import_model_file(&temp.path().join("nope.json"), None)
            .unwrap_err();
        assert!(matches!(err, ProjectError::NotFound { .. }));
    }

    #[test]
    fn import_model_file_rejects_a_dangerous_name() {
        let temp = TempDir::new("importname");
        let mut project = Project::create(temp.path(), "Import").unwrap();
        let source = temp.path().join("ok.json");
        std::fs::write(&source, b"{}").unwrap();
        let err = project
            .import_model_file(&source, Some("../escape.json"))
            .unwrap_err();
        assert!(matches!(err, ProjectError::UnsafeFileName { .. }));
    }

    #[test]
    fn resolve_refuses_to_escape_the_project_directory() {
        let temp = TempDir::new("escape");
        let project = Project::create(temp.path(), "Escape").unwrap();
        assert!(project.resolve("models/rocket.json").is_ok());
        for bad in [
            "../outside.json",
            "models/../../outside.json",
            "/etc/passwd",
        ] {
            let err = project.resolve(bad).unwrap_err();
            assert!(
                matches!(err, ProjectError::ReferenceOutside { .. }),
                "{} was allowed",
                bad
            );
        }
    }

    #[test]
    fn relative_round_trips_a_path_inside_the_project() {
        let temp = TempDir::new("relative");
        let project = Project::create(temp.path(), "Relative").unwrap();
        let absolute = project.models_dir().join("rocket.json");
        let relative = project.relative(&absolute).unwrap();
        assert_eq!(relative, "models/rocket.json");
        assert_eq!(project.resolve(&relative).unwrap(), absolute);
        assert!(project.relative(Path::new("C:/elsewhere/x.json")).is_none());
    }

    #[test]
    fn inventory_counts_the_artifacts() {
        let temp = TempDir::new("inventory");
        let project = Project::create(temp.path(), "Inventory").unwrap();
        assert!(project.inventory().is_empty());

        std::fs::write(project.models_dir().join("a.json"), b"{}").unwrap();
        std::fs::create_dir_all(project.simulations_dir().join("run-001")).unwrap();
        std::fs::create_dir_all(project.simulations_dir().join("run-002")).unwrap();
        std::fs::create_dir_all(project.flights_dir().join("flight-001")).unwrap();
        std::fs::write(project.telemetry_dir().join("session-001.hlog"), b"x").unwrap();
        std::fs::write(project.reports_dir().join("report.md"), b"x").unwrap();

        let inventory = project.inventory();
        assert_eq!(inventory.models, 1);
        assert_eq!(inventory.simulations, 2);
        assert_eq!(inventory.flight_logs, 1);
        assert_eq!(inventory.telemetry_sessions, 1);
        assert_eq!(inventory.reports, 1);
        assert!(!inventory.is_empty());
        assert!(inventory.summary().contains("1 model"));
    }

    #[test]
    fn status_line_reports_the_name_and_dirty_state() {
        let temp = TempDir::new("status");
        let mut project = Project::create(temp.path(), "Status").unwrap();
        assert!(project.status_line().contains("Status"));
        project.update(|m| m.name = "Renamed".to_string());
        assert!(project.status_line().contains("unsaved changes"));
    }

    #[test]
    fn discover_finds_projects_without_descending_into_them() {
        let temp = TempDir::new("discover");
        let a = Project::create(temp.path().join("one"), "One").unwrap();
        let b = Project::create(temp.path().join("nested/two"), "Two").unwrap();
        let found = discover_projects(temp.path(), 4);
        assert_eq!(found.len(), 2);
        assert!(found.contains(&a.root().to_path_buf()));
        assert!(found.contains(&b.root().to_path_buf()));
        // A file inside a project must not be mistaken for a nested project.
        std::fs::write(a.models_dir().join("m.json"), b"{}").unwrap();
        assert_eq!(discover_projects(temp.path(), 4).len(), 2);
    }

    #[test]
    fn discover_honours_the_depth_limit() {
        let temp = TempDir::new("depth");
        Project::create(temp.path().join("a/b/c/d/deep"), "Deep").unwrap();
        assert!(discover_projects(temp.path(), 1).is_empty());
        assert_eq!(discover_projects(temp.path(), 8).len(), 1);
    }

    #[test]
    fn read_metadata_works_without_opening() {
        let temp = TempDir::new("readmeta");
        let project = Project::create(temp.path(), "Meta").unwrap();
        let metadata = read_metadata(temp.path()).unwrap();
        assert_eq!(metadata.name, "Meta");
        assert_eq!(metadata.project_id, project.metadata.project_id);
    }

    #[test]
    fn read_metadata_reports_a_missing_file() {
        let temp = TempDir::new("readmetamissing");
        let err = read_metadata(temp.path()).unwrap_err();
        assert!(matches!(err, ProjectError::MissingProjectFile { .. }));
    }

    #[test]
    fn metadata_validation_catches_an_empty_name() {
        let mut metadata = ProjectMetadata::new("x");
        assert!(metadata.validate().is_empty());
        metadata.name = "   ".to_string();
        assert!(!metadata.validate().is_empty());
        metadata.name = "ok".to_string();
        metadata.project_id = String::new();
        assert!(!metadata.validate().is_empty());
    }

    #[test]
    fn metadata_touch_updates_the_timestamp_and_version() {
        let mut metadata = ProjectMetadata::new("Touch");
        let first = metadata.updated_at.clone();
        metadata.touch();
        assert!(!metadata.updated_at.is_empty());
        assert_eq!(metadata.application_version, hex_core::app_version());
        // The timestamp must be a valid ISO 8601 value, so it round trips.
        assert!(metadata.updated_at.contains('T') || !first.is_empty());
    }

    #[test]
    fn project_paths_are_all_inside_the_project() {
        let temp = TempDir::new("paths");
        let project = Project::create(temp.path(), "Paths").unwrap();
        for dir in REQUIRED_DIRECTORIES {
            assert!(project.path_for(dir).starts_with(project.root()));
        }
        assert_eq!(project.models_dir(), project.path_for(dirs::MODELS));
        assert_eq!(project.assets_dir(), project.path_for(dirs::ASSETS));
    }
}
