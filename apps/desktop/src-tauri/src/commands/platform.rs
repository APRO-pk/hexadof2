//! APRO Works platform import.
//!
//! Lets HexaDOF pull a vehicle's mass properties straight from the orchestration store
//! instead of making the user export a file from the CAD application and find it again.
//!
//! Everything here is a thin adapter over [`hex_bridge`], which owns the payload
//! interpretation, the unit boundary and the axis mapping. The commands only deal with
//! connection state and session state.
//!
//! The store conversation is *pull*: the hub notifies, this side decides when to read.
//! Nothing is pushed into the application behind the user's back.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use crate::error::CommandError;
use crate::events;
use crate::state::{emit_notice, AppState};

/// Whether HexaDOF can reach an APRO Works store, and why not when it cannot.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformStatus {
    /// False when HexaDOF was started on its own rather than by the hub. Not an error.
    pub connected: bool,
    /// Endpoint from the hub's discovery file, when there is one.
    pub endpoint: Option<String>,
    pub app_slug: String,
    /// The artifact type this app reads.
    pub consumes: String,
    /// A sentence to show the user verbatim.
    pub detail: String,
}

/// One published vehicle that could be imported.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformModel {
    /// The artifact instance, which is the producer's stable vehicle identity.
    pub instance: String,
    /// The publisher's label for the newest revision, when it set one.
    pub label: Option<String>,
    pub revision_number: u32,
    /// Total mass of the newest revision, kilograms. Shown so the user can tell two
    /// similarly named designs apart before importing.
    pub mass_kg: f64,
    /// Whether the payload carried its own reference geometry.
    pub has_reference_geometry: bool,
    /// Whether the newest revision declares SI. A payload that does not is listed but
    /// cannot be imported.
    pub declares_si: bool,
}

/// What an import produced, in the same shape the file-import dialog already consumes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlatformImportResult {
    pub model: crate::commands::model::ModelView,
    pub usable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub document_json: Option<String>,
    /// The artifact instance the model came from.
    pub instance: String,
    pub revision_number: u32,
    /// True when the publisher's own reference geometry was used.
    pub reference_from_publisher: bool,
}

/// Connect using the launch handshake, or explain why there is nothing to connect to.
fn platform_client() -> Result<hex_bridge::apro_client::HttpStoreClient, CommandError> {
    match hex_bridge::apro_client::HttpStoreClient::from_launch_environment() {
        Ok(Some(client)) => Ok(client),
        Ok(None) => Err(CommandError::warning(
            "platform.not_connected",
            "HexaDOF is not connected to APRO Works",
            "This window was started on its own, so there is no store to read from.",
            "Launch HexaDOF from the APRO Works hub to import models from the platform.",
        )),
        Err(err) => Err(CommandError::new(
            "platform.unreachable",
            "The APRO Works store could not be reached",
            format!("The launch handshake failed: {err}"),
            "Relaunch HexaDOF from the hub, or check that the hub is still running.",
        )
        .with_technical(format!("{err:?}"))),
    }
}

/// Report the connection state. Never fails, so the interface can always render it.
#[tauri::command]
pub fn platform_status() -> Result<PlatformStatus, CommandError> {
    use hex_bridge::apro_client::{read_discovery, HttpStoreClient};

    let endpoint = read_discovery()
        .ok()
        .flatten()
        .map(|discovery| discovery.endpoint);

    let (connected, detail) = match HttpStoreClient::from_launch_environment() {
        Ok(Some(_)) => (
            true,
            "Connected to APRO Works. Published vehicles can be imported directly."
                .to_string(),
        ),
        Ok(None) => (
            false,
            "Standalone. Launch HexaDOF from APRO Works to import models from the \
             platform."
                .to_string(),
        ),
        Err(err) => (false, format!("Could not reach the APRO Works store: {err}")),
    };

    Ok(PlatformStatus {
        connected,
        endpoint,
        app_slug: hex_bridge::APP_SLUG.to_string(),
        consumes: hex_bridge::apro_contracts::MASS_PROPERTIES_TYPE.to_string(),
        detail,
    })
}

/// List the vehicles published to the platform.
#[tauri::command]
pub fn platform_list_models() -> Result<Vec<PlatformModel>, CommandError> {
    use hex_bridge::apro_client::{AproStoreClient, ArtifactFilter};

    let client = platform_client()?;

    let artifacts = client
        .list_artifacts(&ArtifactFilter {
            // The filter takes the raw type id, not a parsed one.
            type_id: Some(hex_bridge::apro_contracts::MASS_PROPERTIES_TYPE.to_string()),
            ..Default::default()
        })
        .map_err(|err| platform_error("list", hex_bridge::BridgeError::Client(err)))?;

    let mut models = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        // A design that has been declared but never published has no revision to read.
        let Some(revision_number) = artifact.current_revision_number else {
            continue;
        };

        let published = hex_bridge::pull_mass_properties(&client, &artifact.instance)
            .map_err(|err| platform_error("read", err))?;

        // A declared-but-never-published artifact, or a payload that failed validation,
        // still appears in the list so the user can see it exists — but marked as not
        // importable rather than silently omitted.
        let (mass_kg, has_reference_geometry, declares_si) = match &published {
            Some(published) => (
                published.payload.mass_kg,
                published.payload.reference_geometry.is_some(),
                published.payload.units == hex_bridge::apro_contracts::SI_MARKER,
            ),
            None => (0.0, false, false),
        };

        models.push(PlatformModel {
            instance: artifact.instance,
            label: artifact.label,
            revision_number,
            mass_kg,
            has_reference_geometry,
            declares_si,
        });
    }

    models.sort_by(|a, b| a.instance.cmp(&b.instance));
    Ok(models)
}

/// Pull a published vehicle and import it as a dynamics model.
///
/// The model id is the platform instance, so re-importing an updated design replaces the
/// same model rather than accumulating near-duplicates under an ever-changing name.
#[tauri::command]
pub fn platform_import_model(
    app: AppHandle,
    state: State<'_, AppState>,
    instance: String,
    name: Option<String>,
) -> Result<PlatformImportResult, CommandError> {
    if instance.trim().is_empty() {
        return Err(CommandError::warning(
            "platform.no_selection",
            "No vehicle was selected",
            "Choose a published vehicle to import.",
            "Pick one from the list, then import it.",
        ));
    }

    let client = platform_client()?;
    let display_name = name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&instance)
        .to_string();

    // Pull once. The revision number and the reference-geometry provenance both come from
    // the same payload the model is built from, so they cannot describe a different
    // revision than the one imported.
    let published = hex_bridge::pull_mass_properties(&client, &instance)
        .map_err(|err| platform_error("read", err))?
        .ok_or_else(|| {
            CommandError::new(
                "platform.not_published",
                "That design has not been published yet",
                format!("No revision of {instance} is on the store."),
                "Publish it from the CAD application, then refresh this list.",
            )
        })?;

    let reference_from_publisher = published.payload.reference_geometry.is_some();
    let reference = published.payload.reference_geometry.clone();

    let document =
        hex_bridge::to_model_document(&published.payload, &instance, &display_name, reference)
            .map_err(|err| platform_error("convert", err))?;
    let json = document
        .to_json_pretty()
        .map_err(|err| platform_error("serialise", hex_bridge::BridgeError::Model(err)))?;
    let imported = hex_model::import_json(&json, &hex_model::ImportOptions::si())
        .map_err(|err| platform_error("import", hex_bridge::BridgeError::Model(err)))?;

    let view = crate::commands::model::ModelView::from_imported(&imported);
    let usable = view.valid;

    state.with(|s| {
        s.models.push(imported);
        Ok::<(), CommandError>(())
    })??;

    if view.valid {
        emit_notice(
            &app,
            events::NoticeEvent::info(
                "Model imported from APRO Works",
                format!(
                    "{} v{} ({}).",
                    view.name, view.model_version, view.validation_summary
                ),
            ),
        );
    } else {
        emit_notice(
            &app,
            events::NoticeEvent::error(
                "Imported with blocking errors",
                format!("{} has {} blocking error(s).", view.name, view.error_count),
            ),
        );
    }

    Ok(PlatformImportResult {
        model: view,
        usable,
        document_json: Some(json),
        instance,
        revision_number: published.revision_number,
        reference_from_publisher,
    })
}

/// Turn a bridge failure into something the interface can explain.
fn platform_error(stage: &str, err: hex_bridge::BridgeError) -> CommandError {
    use hex_bridge::BridgeError;

    let code = match &err {
        BridgeError::NotPublished { .. } => "platform.not_published",
        BridgeError::MissingReferenceGeometry => "platform.no_reference_geometry",
        BridgeError::Contract(_) => "platform.payload_invalid",
        BridgeError::Client(_) => "platform.unreachable",
        BridgeError::NotUtf8 => "platform.payload_invalid",
        BridgeError::Model(_) => "platform.model_invalid",
        BridgeError::TypeId(_) | BridgeError::Json(_) => "platform.internal",
    };

    let (title, suggestion) = match &err {
        BridgeError::NotPublished { .. } => (
            "That design has not been published yet",
            "Publish it from the CAD application, then refresh this list.",
        ),
        BridgeError::MissingReferenceGeometry => (
            "The published model does not say what its reference area is",
            "Reference area is a convention, not something a CAD model determines, so \
             the publisher has to state it. Republish from the CAD application with \
             reference geometry, or enter one manually.",
        ),
        BridgeError::Contract(_) => (
            "The published mass properties failed validation",
            "The payload may be in the wrong units. Re-publish it from the CAD \
             application, which converts to SI at the publish boundary.",
        ),
        BridgeError::Model(_) => (
            "The imported model did not pass HexaDOF's own validation",
            "Check the model in the Models panel for the specific errors.",
        ),
        _ => (
            "The model could not be imported from the platform",
            "Check the connection in the platform panel, then try again.",
        ),
    };

    CommandError::new(
        code,
        title,
        format!("The {stage} step failed: {err}"),
        suggestion,
    )
    .with_technical(format!("{err:?}"))
}
