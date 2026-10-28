//! Scenepacks: a character's shots, filtered and picked by hand, exported in
//! one go. The pack is made from the analysis each time (as_project::pack),
//! so the grid, the plan and the export can't disagree.

use as_api::{
    ExportPlan, ExportSummary, PackChoices, PackExportRequest, PackNaming, PackPicks, PackPreview, PackSpec, PackState,
};
use as_core::{CharacterId, ProjectId};
use as_export::builtin_presets;
use as_media::Caps;
use tauri::{AppHandle, State};
use tauri_specta::Event;

use crate::error::{AppError, AppResult, ErrorCode};
use crate::events::{ExportChanged, PackChanged};
use crate::paths;
use crate::state::{AppState, allow_asset_tree, blocking};
use crate::work;

/// A character's pack settings as last used (the defaults the first time).
#[tauri::command]
#[specta::specta]
pub async fn pack_get(
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
) -> AppResult<PackState> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.pack_state(character_id)?)).await
}

/// Keep a character's pack settings for next time.
#[tauri::command]
#[specta::specta]
pub async fn pack_save(
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
    pack: PackState,
) -> AppResult<()> {
    let lib = state.library.clone();
    blocking(move || Ok(lib.project(project_id)?.save_pack_state(character_id, &pack)?)).await
}

/// Every shot of the character, the ones the pack takes marked, in its order.
#[tauri::command]
#[specta::specta]
pub async fn pack_preview(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
    spec: PackSpec,
) -> AppResult<PackPreview> {
    let lib = state.library.clone();
    let (preview, faces) = blocking(move || {
        let p = lib.project(project_id)?;
        Ok((p.pack_preview(character_id, &spec)?, p.faces_dir()))
    })
    .await?;
    // The grid shows the shots' sprites and faces.
    allow_asset_tree(&app, &faces);
    Ok(preview)
}

/// Pick, drop or hand back to the filters shots of a pack (a click's
/// `picks`, in order), and say what that made of the pack `spec` makes: its
/// choices alone, as picks change nothing else. No `pack:changed`: the grid
/// that picked takes the answer.
#[tauri::command]
#[specta::specta]
pub async fn pack_override(
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
    picks: Vec<PackPicks>,
    spec: PackSpec,
) -> AppResult<PackChoices> {
    let lib = state.library.clone();
    blocking(move || {
        let p = lib.project(project_id)?;
        for pick in &picks {
            p.set_pack_overrides(character_id, &pick.shots, pick.include)?;
        }
        Ok(p.pack_choices(character_id, &spec)?)
    })
    .await
}

/// The names the pack's files would get and its size with each preset.
#[tauri::command]
#[specta::specta]
pub async fn pack_plan(
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
    spec: PackSpec,
    naming: PackNaming,
) -> AppResult<ExportPlan> {
    let (lib, tools) = (state.library.clone(), state.tools().ok());
    blocking(move || {
        // Sized for the encoders an export would use (found once, in about a second).
        let none = Caps::default();
        let caps = tools.as_deref().map_or(&none, |t| t.caps());
        Ok(lib.project(project_id)?.pack_plan(character_id, &spec, &naming, &builtin_presets(), caps)?)
    })
    .await
}

/// Export a character's pack: exactly what the preview with `req.stamp`
/// showed (refused if the pack changed since). Progress arrives as `job:*`
/// events, results as `export:changed`.
#[tauri::command]
#[specta::specta]
pub async fn pack_export(
    app: AppHandle,
    state: State<'_, AppState>,
    project_id: ProjectId,
    character_id: CharacterId,
    req: PackExportRequest,
) -> AppResult<ExportSummary> {
    let preset = builtin_presets()
        .into_iter()
        .find(|p| p.id == req.preset_id)
        .ok_or_else(|| AppError::new(ErrorCode::Invalid, "Unknown export preset."))?;
    // The folder picked for it (the character's own goes inside, by name).
    if let Some(dir) = req.out_dir.as_deref().map(str::trim).filter(|d| !d.is_empty()) {
        paths::check(dir)?;
    }
    let lib = state.library.clone();
    let (project, summary) = blocking(move || {
        let project = lib.project(project_id)?;
        let default_dir = lib.default_export_dir(&lib.summary(project_id)?)?;
        let (request, out_dir) = project.pack_export_request(character_id, &req, &default_dir)?;
        let summary = project.create_export(&request, preset, &out_dir)?;
        Ok((project, summary))
    })
    .await?;
    work::queue_export(&app, &project, &summary);
    let _ = ExportChanged { project_id, export_id: summary.id }.emit(&app);
    // The settings it was made with are kept.
    let _ = PackChanged { project_id, character_id }.emit(&app);
    Ok(summary)
}
