//! Tauri commands.
//!
//! Every one is a one-line wrapper over `ai_core::Studio`, and the names match
//! the HTTP routes in `ai-studio-serve`. The desktop app holds no document logic
//! of its own.

use std::path::PathBuf;

use tauri::State;

use ai_core::{
    AssetEntry, AssetList, CreateRequest, FileBody, FileList, Health, ProjectList, ProjectPayload,
    RecalcRequest, RecalcResponse, Studio, UploadAssetRequest,
};

/// A command error, rendered as a plain message the UI already knows how to show.
pub struct CommandError(String);

impl From<ai_core::Error> for CommandError {
    fn from(e: ai_core::Error) -> Self {
        CommandError(e.to_string())
    }
}

impl serde::Serialize for CommandError {
    fn serialize<S: serde::Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        // The web client reads `{ error }` from HTTP; over IPC a rejected promise
        // carries the message directly, so a bare string is what it expects.
        s.serialize_str(&self.0)
    }
}

type Result<T> = std::result::Result<T, CommandError>;

#[tauri::command]
pub fn health(studio: State<'_, Studio>) -> Health {
    studio.health()
}

#[tauri::command]
pub fn list_projects(studio: State<'_, Studio>) -> Result<ProjectList> {
    Ok(studio.list_projects()?)
}

#[tauri::command]
pub fn create_project(studio: State<'_, Studio>, request: CreateRequest) -> Result<ProjectPayload> {
    Ok(studio.create_project(request)?)
}

#[tauri::command]
pub fn get_project(studio: State<'_, Studio>, folder: String) -> Result<ProjectPayload> {
    Ok(studio.get_project(&folder)?)
}

#[tauri::command]
pub fn save_project(
    studio: State<'_, Studio>,
    folder: String,
    payload: ProjectPayload,
) -> Result<ProjectPayload> {
    Ok(studio.save_project(&folder, payload)?)
}

#[tauri::command]
pub fn rename_project(
    studio: State<'_, Studio>,
    folder: String,
    title: String,
) -> Result<ProjectPayload> {
    Ok(studio.rename_project(&folder, &title)?)
}

#[tauri::command]
pub fn delete_project(studio: State<'_, Studio>, folder: String) -> Result<()> {
    Ok(studio.delete_project(&folder)?)
}

#[tauri::command]
pub fn project_files(studio: State<'_, Studio>, folder: String) -> Result<FileList> {
    Ok(studio.project_files(&folder)?)
}

#[tauri::command]
pub fn read_file(studio: State<'_, Studio>, folder: String, path: String) -> Result<FileBody> {
    Ok(studio.read_file(&folder, &path)?)
}

#[tauri::command]
pub fn digest(studio: State<'_, Studio>, folder: String) -> Result<String> {
    Ok(studio.digest(&folder)?)
}

#[tauri::command]
pub fn list_assets(studio: State<'_, Studio>, folder: String) -> Result<AssetList> {
    Ok(studio.list_assets(&folder)?)
}

#[tauri::command]
pub fn upload_asset(
    studio: State<'_, Studio>,
    folder: String,
    request: UploadAssetRequest,
) -> Result<AssetEntry> {
    Ok(studio.upload_asset(&folder, request)?)
}

#[tauri::command]
pub fn recalc(studio: State<'_, Studio>, request: RecalcRequest) -> RecalcResponse {
    studio.recalc(request)
}

/// Where the workspace lives, for the launcher's "폴더 열기" button.
#[tauri::command]
pub fn workspace_path(studio: State<'_, Studio>) -> String {
    studio.workspace().to_string_lossy().into_owned()
}

/// Render an export and write it wherever the user chooses.
///
/// Returns the path written, or `None` if the save dialog was dismissed — which
/// is not an error and should not surface as one.
#[tauri::command]
pub async fn export_project(
    app: tauri::AppHandle,
    studio: State<'_, Studio>,
    folder: String,
    ext: String,
) -> Result<Option<String>> {
    use tauri_plugin_dialog::DialogExt;

    let body = studio.export(&folder, &ext)?;
    let ext_owned = ext.clone();

    let (tx, rx) = std::sync::mpsc::channel::<Option<PathBuf>>();
    app.dialog()
        .file()
        .set_file_name(&body.filename)
        .add_filter(format!(".{ext_owned}"), &[ext_owned.as_str()])
        .save_file(move |path| {
            let _ = tx.send(path.and_then(|p| p.into_path().ok()));
        });

    // The dialog callback fires on the main thread; this command is async so the
    // wait does not block it.
    let Ok(Some(target)) = rx.recv() else {
        return Ok(None);
    };
    std::fs::write(&target, &body.bytes).map_err(ai_core::Error::from)?;
    Ok(Some(target.to_string_lossy().into_owned()))
}

/// Reveal a project folder in the OS file manager.
#[tauri::command]
pub fn reveal_project(
    app: tauri::AppHandle,
    studio: State<'_, Studio>,
    folder: String,
) -> Result<()> {
    use tauri_plugin_opener::OpenerExt;

    let dir = ai_format::project::existing_project_dir(studio.workspace(), &folder)
        .map_err(ai_core::Error::from)?;
    let _ = app.opener().reveal_item_in_dir(dir);
    Ok(())
}
