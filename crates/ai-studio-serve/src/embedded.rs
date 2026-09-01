//! The web UI, baked into the binary.
//!
//! Built with `--features embed-ui` after `npm run build`, so a single
//! downloaded file serves both the API and the interface.

use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::Embed;

#[derive(Embed)]
#[folder = "../../apps/web/dist"]
#[allow(clippy::upper_case_acronyms)]
struct Assets;

pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };

    match Assets::get(path).or_else(|| Assets::get("index.html")) {
        None => (StatusCode::NOT_FOUND, "웹 UI가 빌드되지 않았습니다").into_response(),
        Some(file) => {
            let mime = mime_guess::from_path(path).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref())],
                file.data.into_owned(),
            )
                .into_response()
        }
    }
}
