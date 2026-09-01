//! AI Studio, as a desktop app.
//!
//! The window renders the same web UI a browser would, but there is no server
//! and no port: the UI calls `ai-core` directly through Tauri commands, and the
//! whole document core — formula engine, save format, `AI.md`, Office exports —
//! is compiled into this binary. Nothing needs installing alongside it.
#![cfg_attr(
    all(not(debug_assertions), target_os = "windows"),
    windows_subsystem = "windows"
)]

use std::path::PathBuf;

use tauri::http;
use tauri::{Manager, UriSchemeContext, UriSchemeResponder};

mod commands;

/// Where documents live by default.
///
/// The platform's documents folder, so a new user's files are somewhere they can
/// find them rather than buried in an application-support directory. Overridable
/// with `AI_STUDIO_WORKSPACE` for a portable install or a shared drive.
fn default_workspace() -> PathBuf {
    if let Some(dir) = std::env::var_os("AI_STUDIO_WORKSPACE") {
        return PathBuf::from(dir);
    }
    directories::UserDirs::new()
        .and_then(|dirs| dirs.document_dir().map(|d| d.join("AI Studio")))
        .unwrap_or_else(|| {
            directories::ProjectDirs::from("dev", "aistudio", "AI Studio")
                .map(|p| p.data_dir().join("workspace"))
                .unwrap_or_else(|| PathBuf::from("workspace"))
        })
}

fn main() {
    let workspace = default_workspace();
    let studio = match ai_core::Studio::open(&workspace) {
        Ok(studio) => studio,
        Err(e) => {
            eprintln!("작업 폴더를 열 수 없습니다 ({}): {e}", workspace.display());
            std::process::exit(1);
        }
    };

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        // Images inside a project are served through our own scheme rather than a
        // filesystem scope, so the containment check is the same one the HTTP
        // route uses and there is only one place to get it right.
        .register_asynchronous_uri_scheme_protocol("aistudio", serve_asset)
        .manage(studio)
        .invoke_handler(tauri::generate_handler![
            commands::health,
            commands::list_projects,
            commands::create_project,
            commands::get_project,
            commands::save_project,
            commands::rename_project,
            commands::delete_project,
            commands::project_files,
            commands::read_file,
            commands::digest,
            commands::list_assets,
            commands::upload_asset,
            commands::recalc,
            commands::workspace_path,
            commands::export_project,
            commands::reveal_project,
        ])
        .setup(|app| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title("AI Studio");
            }
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("AI Studio를 시작할 수 없습니다");
}

/// `aistudio://localhost/<folder>/assets/<name>` -> the image bytes.
fn serve_asset(
    context: UriSchemeContext<'_, tauri::Wry>,
    request: http::Request<Vec<u8>>,
    responder: UriSchemeResponder,
) {
    let studio = context.app_handle().state::<ai_core::Studio>();
    let path = request.uri().path().trim_start_matches('/').to_string();

    let reply = |status: u16, mime: &str, body: Vec<u8>| {
        http::Response::builder()
            .status(status)
            .header(http::header::CONTENT_TYPE, mime)
            .header(http::header::CACHE_CONTROL, "no-cache")
            .body(body)
            .expect("a valid response")
    };

    // The first segment is the project folder; the rest is the path within it.
    let Some((folder, inner)) = path.split_once('/') else {
        responder.respond(reply(
            400,
            "text/plain; charset=utf-8",
            "잘못된 자산 경로".as_bytes().to_vec(),
        ));
        return;
    };
    let folder = percent_decode(folder);
    let inner = percent_decode(inner);

    match studio.read_asset(&folder, &inner) {
        Ok((bytes, mime)) => responder.respond(reply(200, &mime, bytes)),
        Err(e) => {
            let status = e.status();
            responder.respond(reply(
                status,
                "text/plain; charset=utf-8",
                e.to_string().into_bytes(),
            ))
        }
    }
}

/// Decode the percent-escapes the frontend put in the URL.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

#[cfg(test)]
mod tests {
    use super::percent_decode;

    #[test]
    fn percent_escapes_round_trip() {
        assert_eq!(percent_decode("plain.png"), "plain.png");
        assert_eq!(percent_decode("%EC%98%88%EC%82%B0.aigrid"), "예산.aigrid");
        assert_eq!(percent_decode("a%20b"), "a b");
        // A malformed escape is left as written rather than dropped.
        assert_eq!(percent_decode("100%zz"), "100%zz");
    }
}
