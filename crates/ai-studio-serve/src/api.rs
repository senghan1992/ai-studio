//! The HTTP surface. Every handler is a thin wrapper over `ai_core::Studio`.

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;

use ai_core::{
    CreateRequest, Error, ImportRequest, ProjectPayload, RecalcRequest, RenameRequest, Studio,
    UploadAssetRequest,
};

pub type Shared = Arc<Studio>;

/// An error rendered the way the web client already expects: `{ "error": "…" }`.
pub struct ApiError(Error);

impl From<Error> for ApiError {
    fn from(e: Error) -> Self {
        ApiError(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status =
            StatusCode::from_u16(self.0.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            tracing::error!("{}", self.0);
        }
        (
            status,
            Json(serde_json::json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}

type ApiResult<T> = std::result::Result<T, ApiError>;

pub fn routes() -> Router<Shared> {
    Router::new()
        .route("/health", get(health))
        .route("/projects", get(list_projects).post(create_project))
        .route("/import", post(import_file))
        .route(
            "/projects/{folder}",
            get(get_project)
                .put(save_project)
                .patch(rename_project)
                .delete(delete_project),
        )
        .route("/projects/{folder}/files", get(project_files))
        .route("/projects/{folder}/file", get(read_file))
        .route("/projects/{folder}/digest", get(digest))
        .route("/projects/{folder}/preview", post(preview))
        .route("/projects/{folder}/export/{ext}", get(export))
        .route(
            "/projects/{folder}/assets",
            get(list_assets).post(upload_asset),
        )
        .route("/projects/{folder}/asset", get(read_asset))
        .route("/recalc", post(recalc))
}

async fn health(State(studio): State<Shared>) -> impl IntoResponse {
    Json(studio.health())
}

async fn list_projects(State(studio): State<Shared>) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.list_projects()?))
}

async fn create_project(
    State(studio): State<Shared>,
    Json(request): Json<CreateRequest>,
) -> ApiResult<impl IntoResponse> {
    Ok((StatusCode::CREATED, Json(studio.create_project(request)?)))
}

/// Convert an Office file into a project. The upload is base64 in JSON rather
/// than multipart so the desktop and browser paths send the identical body.
async fn import_file(
    State(studio): State<Shared>,
    Json(request): Json<ImportRequest>,
) -> ApiResult<impl IntoResponse> {
    Ok((StatusCode::CREATED, Json(studio.import(request)?)))
}

async fn get_project(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.get_project(&folder)?))
}

async fn save_project(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
    Json(payload): Json<ProjectPayload>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.save_project(&folder, payload)?))
}

async fn rename_project(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
    Json(request): Json<RenameRequest>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.rename_project(&folder, &request.title)?))
}

async fn delete_project(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
) -> ApiResult<impl IntoResponse> {
    studio.delete_project(&folder)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn project_files(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.project_files(&folder)?))
}

#[derive(Deserialize)]
pub struct PathQuery {
    pub path: String,
}

async fn read_file(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.read_file(&folder, &query.path)?))
}

async fn digest(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
) -> ApiResult<impl IntoResponse> {
    let text = studio.digest(&folder)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/markdown; charset=utf-8"),
    );
    Ok((headers, text))
}

async fn preview(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
    Json(payload): Json<ProjectPayload>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.preview(&folder, payload)?))
}

async fn export(
    State(studio): State<Shared>,
    Path((folder, ext)): Path<(String, String)>,
) -> ApiResult<impl IntoResponse> {
    let body = studio.export(&folder, &ext)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(body.mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    // RFC 5987 so a Korean filename survives the header.
    headers.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename*=UTF-8''{}",
            percent_encode(&body.filename)
        ))
        .unwrap_or(HeaderValue::from_static("attachment")),
    );
    Ok((headers, body.bytes))
}

async fn list_assets(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
) -> ApiResult<impl IntoResponse> {
    Ok(Json(studio.list_assets(&folder)?))
}

async fn upload_asset(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
    Json(request): Json<UploadAssetRequest>,
) -> ApiResult<impl IntoResponse> {
    Ok((
        StatusCode::CREATED,
        Json(studio.upload_asset(&folder, request)?),
    ))
}

async fn read_asset(
    State(studio): State<Shared>,
    Path(folder): Path<String>,
    Query(query): Query<PathQuery>,
) -> ApiResult<impl IntoResponse> {
    let (bytes, mime) = studio.read_asset(&folder, &query.path)?;
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime)
            .unwrap_or(HeaderValue::from_static("application/octet-stream")),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    Ok((headers, bytes))
}

async fn recalc(
    State(studio): State<Shared>,
    Json(request): Json<RecalcRequest>,
) -> impl IntoResponse {
    Json(studio.recalc(request))
}

/// Percent-encode for a `filename*=UTF-8''…` header value.
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filenames_survive_the_content_disposition_header() {
        assert_eq!(percent_encode("budget.xlsx"), "budget.xlsx");
        assert_eq!(percent_encode("예산.xlsx"), "%EC%98%88%EC%82%B0.xlsx");
        assert_eq!(percent_encode("a b.csv"), "a%20b.csv");
    }
}
