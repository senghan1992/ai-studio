//! The headless AI Studio server.
//!
//! Serves the same API the desktop app calls through Tauri commands, plus the
//! built web UI when there is one. Useful for a shared machine, a container, or
//! letting an agent drive documents without a GUI.

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use tower_http::trace::TraceLayer;

mod api;
#[cfg(feature = "embed-ui")]
mod embedded;

const DEFAULT_PORT: u16 = 5177;

struct Options {
    workspace: PathBuf,
    port: u16,
    host: IpAddr,
    /// A directory of built web assets to serve, when not embedding them.
    web_dist: Option<PathBuf>,
    open: bool,
}

fn usage() -> String {
    format!(
        "ai-studio-serve — AI Studio를 HTTP로 제공합니다\n\n\
         사용법: ai-studio-serve [옵션]\n\n\
         옵션:\n\
         \x20 --workspace <경로>   문서 폴더 (기본: ./workspace, 환경변수 AI_STUDIO_WORKSPACE)\n\
         \x20 --port <번호>        수신 포트 (기본: {DEFAULT_PORT}, 환경변수 PORT)\n\
         \x20 --host <주소>        수신 주소 (기본: 127.0.0.1). 0.0.0.0은 네트워크에 노출됩니다\n\
         \x20 --web <경로>         빌드된 웹 UI 폴더 (기본: apps/web/dist가 있으면 사용)\n\
         \x20 --open               시작 후 브라우저를 엽니다\n\
         \x20 --help               이 도움말\n"
    )
}

fn parse_args() -> Result<Options, String> {
    let mut options = Options {
        workspace: std::env::var_os("AI_STUDIO_WORKSPACE")
            .map(PathBuf::from)
            .unwrap_or_else(default_workspace),
        port: std::env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT),
        host: IpAddr::V4(Ipv4Addr::LOCALHOST),
        web_dist: None,
        open: false,
    };

    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        let mut next = |name: &str| {
            args.next()
                .ok_or_else(|| format!("{name} 뒤에 값이 필요합니다"))
        };
        match arg.as_str() {
            "--workspace" | "-w" => options.workspace = PathBuf::from(next("--workspace")?),
            "--port" | "-p" => {
                options.port = next("--port")?
                    .parse()
                    .map_err(|_| "포트는 숫자여야 합니다".to_string())?
            }
            "--host" => {
                options.host = next("--host")?
                    .parse()
                    .map_err(|_| "주소를 해석할 수 없습니다".to_string())?
            }
            "--web" => options.web_dist = Some(PathBuf::from(next("--web")?)),
            "--open" => options.open = true,
            "--help" | "-h" => {
                print!("{}", usage());
                std::process::exit(0);
            }
            other => return Err(format!("알 수 없는 옵션: {other}")),
        }
    }
    Ok(options)
}

fn default_workspace() -> PathBuf {
    // Next to the executable in a packaged install, or `./workspace` in a checkout.
    std::env::current_dir()
        .unwrap_or_default()
        .join("workspace")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "ai_studio_serve=info,tower_http=warn".into()),
        )
        .init();

    let options = match parse_args() {
        Ok(options) => options,
        Err(message) => {
            eprintln!("{message}\n\n{}", usage());
            std::process::exit(2);
        }
    };

    let studio = match ai_core::Studio::open(&options.workspace) {
        Ok(studio) => Arc::new(studio),
        Err(e) => {
            eprintln!("작업 폴더를 열 수 없습니다: {e}");
            std::process::exit(1);
        }
    };

    let mut app = Router::new().nest("/api", api::routes());

    // The UI, from a directory if one was given or found, else from the binary.
    let dist = options.web_dist.clone().or_else(find_web_dist);
    match dist {
        Some(dir) if dir.is_dir() => {
            tracing::info!("웹 UI: {}", dir.display());
            use tower_http::services::{ServeDir, ServeFile};
            let index = dir.join("index.html");
            // `/assets` is served without the SPA fallback: a stale hashed URL
            // should 404, not hand the browser an HTML page to parse as script.
            app = app
                .nest_service("/assets", ServeDir::new(dir.join("assets")))
                .fallback_service(ServeDir::new(&dir).fallback(ServeFile::new(index)));
        }
        _ => {
            #[cfg(feature = "embed-ui")]
            {
                tracing::info!("웹 UI: 바이너리에 내장됨");
                app = app.fallback(embedded::serve);
            }
            #[cfg(not(feature = "embed-ui"))]
            tracing::warn!(
                "빌드된 웹 UI가 없습니다. API만 제공합니다 (npm run build 후 --web 지정)"
            );
        }
    }

    let app = app
        .layer(TraceLayer::new_for_http())
        .layer(tower_http::cors::CorsLayer::permissive())
        .with_state(studio.clone());

    let addr = SocketAddr::new(options.host, options.port);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            eprintln!("{addr} 에서 수신할 수 없습니다: {e}");
            std::process::exit(1);
        }
    };

    let url = format!("http://{}:{}", display_host(options.host), options.port);
    println!("AI Studio  {url}");
    println!("작업 폴더   {}", studio.workspace().display());
    if options.host == IpAddr::V4(Ipv4Addr::UNSPECIFIED) {
        println!("경고: 0.0.0.0에 바인딩했습니다 — 네트워크의 누구나 문서를 읽고 쓸 수 있습니다.");
    }
    if options.open {
        open_browser(&url);
    }

    if let Err(e) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown())
        .await
    {
        eprintln!("서버가 종료되었습니다: {e}");
        std::process::exit(1);
    }
}

fn display_host(host: IpAddr) -> String {
    if host == IpAddr::V4(Ipv4Addr::UNSPECIFIED) {
        "localhost".to_string()
    } else {
        host.to_string()
    }
}

/// The built UI next to the binary, or in the repo during development.
fn find_web_dist() -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join("web"));
            candidates.push(dir.join("../web"));
        }
    }
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("apps/web/dist"));
    }
    candidates
        .into_iter()
        .find(|p| p.join("index.html").is_file())
}

fn open_browser(url: &str) {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "explorer"
    } else {
        "xdg-open"
    };
    let _ = std::process::Command::new(command).arg(url).spawn();
}

async fn shutdown() {
    let _ = tokio::signal::ctrl_c().await;
    println!("\n종료합니다.");
}
