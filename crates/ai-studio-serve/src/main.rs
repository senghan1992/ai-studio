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
mod auth;
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
    /// When non-empty, every `/api` request must carry one of these tokens.
    auth: auth::AuthConfig,
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
         \x20 --token <문자열>     API 접속 토큰 (환경변수 AI_STUDIO_TOKEN). 지정하면 모든\n\
         \x20                      /api 요청에 Authorization: Bearer 또는 ?token= 이 필요합니다\n\
         \x20 --tokens <파일>      이름 있는 토큰 목록 — 한 줄에 `이름:역할:토큰`,\n\
         \x20                      역할은 read/write(읽기/쓰기), `#`은 주석\n\
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
        auth: std::env::var("AI_STUDIO_TOKEN")
            .ok()
            .filter(|t| !t.is_empty())
            .map(auth::AuthConfig::single)
            .unwrap_or_default(),
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
            "--token" => {
                let token = next("--token")?;
                if !token.is_empty() {
                    options.auth.add_single(token);
                }
            }
            "--tokens" => {
                let path = next("--tokens")?;
                let text = std::fs::read_to_string(&path)
                    .map_err(|e| format!("토큰 파일을 읽을 수 없습니다 ({path}): {e}"))?;
                options.auth = auth::AuthConfig::parse_file(&text)
                    .map_err(|e| format!("토큰 파일 ({path}): {e}"))?;
            }
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
                .unwrap_or_else(|_| "ai_studio_serve=info,tower_http=info".into()),
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

    // Axum's default 2MB body limit would refuse any real Office file, and an
    // imported deck's base64 is a third larger again than the file itself.
    let mut api_routes =
        api::routes().layer(axum::extract::DefaultBodyLimit::max(192 * 1024 * 1024));
    // The tokens guard the data, not the app shell: static files stay open,
    // every /api route (health included) answers 401 without a token.
    if !options.auth.is_empty() {
        let config = Arc::new(options.auth.clone());
        api_routes = api_routes.layer(axum::middleware::from_fn(move |request, next| {
            auth::require_token(config.clone(), request, next)
        }));
    }
    let mut app = Router::new().nest("/api", api_routes);

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

    // One line per request — method, path, status, latency — so a pilot can be
    // audited and a failing import found in the log rather than reproduced.
    let trace = TraceLayer::new_for_http()
        .make_span_with(tower_http::trace::DefaultMakeSpan::new().level(tracing::Level::INFO))
        .on_response(tower_http::trace::DefaultOnResponse::new().level(tracing::Level::INFO));
    let app = app
        .layer(trace)
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
    println!(
        "인증        {}",
        if options.auth.is_empty() {
            "없음".to_string()
        } else {
            options.auth.summary()
        }
    );
    if options.host == IpAddr::V4(Ipv4Addr::UNSPECIFIED) && options.auth.is_empty() {
        println!(
            "경고: 0.0.0.0에 토큰 없이 바인딩했습니다 — 네트워크의 누구나 문서를 읽고 쓸 수 \
             있습니다. --token 또는 AI_STUDIO_TOKEN을 지정하세요."
        );
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
