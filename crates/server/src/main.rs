//! The server binary. hyper 1.x directly on tokio — no web framework; the
//! IIIF grammar in `iiif-core` *is* the router.
//!
//! Near-zero config: `iiif-server serve ./images` just works. The only
//! deployment-varying values are the numeric limits and pool sizing.

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "server binary's check subcommand is a CLI: stdout is output, stderr diagnostics"
)]

use hyper::service::service_fn;
use hyper_util::rt::TokioIo;
use iiif_core::info::Limits;
use iiif_server::app::{App, SourceRoot};

/// Bench-decided allocator (docs/spikes/alloc-bench.md): musl's malloc
/// contends badly under concurrent decode; mimalloc measured ~2×.
#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;
use std::{net::SocketAddr, path::Path, process::ExitCode, sync::Arc};

use iiif_sources::LocalRoot;
use tokio::{net::TcpListener, sync::Semaphore};
use tracing::{error, info};

/// Deployment knobs, all optional. Parsed by hand: seven flags do not
/// justify a dependency.
struct Config {
    root: String,
    bind: SocketAddr,
    max_width: u32,
    max_height: u32,
    max_area: u64,
    workers: usize,
    queue_depth: usize,
    public_base: Option<String>,
    endpoint: Option<String>,
}

const USAGE: &str = "usage: iiif-server serve <root> [--bind ADDR] [--max-width N] \
[--max-height N] [--max-area N] [--workers N] [--queue-depth N] [--public-base URL] \
[--endpoint URL]
    iiif-server check <file-or-directory>

<root> is a local directory or s3://bucket/prefix (credentials from the
environment; --endpoint for S3-compatible services). `check` inspects
masters offline and prints serving advice.";

/// `iiif-server check <path>`: offline master inspection — the operator
/// tool that turns serving-time surprises into setup-time advice.
fn run_check(path: &Path) -> ExitCode {
    let mut failures = 0u32;
    let mut checked = 0u32;
    let mut walk = vec![path.to_path_buf()];
    while let Some(entry) = walk.pop() {
        if entry.is_dir() {
            match std::fs::read_dir(&entry) {
                Ok(children) => {
                    for child in children.flatten() {
                        walk.push(child.path());
                    }
                },
                Err(e) => {
                    eprintln!("{}: unreadable directory: {e}", entry.display());
                    failures += 1;
                },
            }
            continue;
        }
        checked += 1;
        let opened = std::fs::File::open(&entry)
            .map_err(|e| format!("unreadable: {e}"))
            .and_then(|file| iiif_core::codec::open_master(file).map_err(|e| e.to_string()));
        match opened {
            Ok(master) => {
                let (w, h) = master.dimensions();
                let description = master.describe();
                let structure = if description.tiles.is_empty() {
                    "untiled".to_owned()
                } else {
                    format!(
                        "{}px tiles, scale factors {:?}",
                        description.tiles[0].width, description.tiles[0].scale_factors
                    )
                };
                println!("{}: OK — {w}×{h}, {structure}", entry.display());
                for advisory in master.advisories() {
                    println!("  advice: {advisory}");
                }
            },
            Err(message) => {
                println!("{}: REJECTED — {message}", entry.display());
                failures += 1;
            },
        }
    }
    println!("checked {checked} file(s), {failures} rejected");
    if failures == 0 {
        ExitCode::SUCCESS
    } else {
        ExitCode::FAILURE
    }
}

fn parse_args(args: &[String]) -> Result<Config, String> {
    let mut it = args.iter();
    match it.next().map(String::as_str) {
        Some("serve") => {},
        _ => return Err(USAGE.to_owned()),
    }
    let root = it.next().ok_or(USAGE)?.clone();
    let mut config = Config {
        root,
        bind: SocketAddr::from(([127, 0, 0, 1], 6363)),
        max_width: 8192,
        max_height: 8192,
        max_area: 33_554_432, // 32 megapixels
        workers: std::thread::available_parallelism().map_or(4, std::num::NonZero::get),
        queue_depth: 64,
        public_base: None,
        endpoint: None,
    };
    while let Some(flag) = it.next() {
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--bind" => config.bind = value.parse().map_err(|e| format!("--bind: {e}"))?,
            "--max-width" => {
                config.max_width = value.parse().map_err(|e| format!("--max-width: {e}"))?;
            },
            "--max-height" => {
                config.max_height = value.parse().map_err(|e| format!("--max-height: {e}"))?;
            },
            "--max-area" => {
                config.max_area = value.parse().map_err(|e| format!("--max-area: {e}"))?;
            },
            "--workers" => {
                config.workers = value.parse().map_err(|e| format!("--workers: {e}"))?;
            },
            "--queue-depth" => {
                config.queue_depth = value.parse().map_err(|e| format!("--queue-depth: {e}"))?;
            },
            "--public-base" => config.public_base = Some(value.clone()),
            "--endpoint" => config.endpoint = Some(value.clone()),
            other => return Err(format!("unknown flag {other}\n{USAGE}")),
        }
    }
    if config.workers == 0 {
        return Err("--workers must be at least 1".to_owned());
    }
    Ok(config)
}

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("check") {
        let Some(target) = args.get(1) else {
            eprintln!("usage: iiif-server check <file-or-directory>");
            return ExitCode::FAILURE;
        };
        return run_check(Path::new(target));
    }
    let config = match parse_args(&args) {
        Ok(config) => config,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::FAILURE;
        },
    };
    let runtime = match tokio::runtime::Runtime::new() {
        Ok(runtime) => runtime,
        Err(e) => {
            error!("runtime startup failed: {e}");
            return ExitCode::FAILURE;
        },
    };
    match runtime.block_on(serve(config)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            error!("{message}");
            ExitCode::FAILURE
        },
    }
}

async fn serve(config: Config) -> Result<(), String> {
    let root = if config.root.starts_with("s3://") {
        iiif_sources::init_tls();
        SourceRoot::Object(iiif_sources::ObjectRoot::new(
            &config.root,
            config.endpoint.as_deref(),
        )?)
    } else {
        SourceRoot::Local(
            LocalRoot::new(Path::new(&config.root))
                .map_err(|e| format!("source root {}: {e}", config.root))?,
        )
    };
    let app = Arc::new(App {
        root,
        limits: Limits {
            width: config.max_width,
            height: config.max_height,
            area: config.max_area,
        },
        public_base: config.public_base,
        admission: Arc::new(Semaphore::new(config.workers + config.queue_depth)),
        decode_permits: Arc::new(Semaphore::new(config.workers)),
        workers: config.workers,
        queue_depth: config.queue_depth,
        metrics: Arc::new(iiif_server::metrics::Metrics::default()),
    });
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|e| format!("bind {}: {e}", config.bind))?;
    info!(
        "serving {} on http://{} ({} workers, queue {})",
        config.root, config.bind, config.workers, config.queue_depth
    );
    loop {
        let (stream, _peer) = match listener.accept().await {
            Ok(accepted) => accepted,
            Err(e) => {
                error!("accept: {e}");
                continue;
            },
        };
        let app = Arc::clone(&app);
        tokio::spawn(async move {
            let service = service_fn(move |req| {
                let app = Arc::clone(&app);
                async move { Ok::<_, std::convert::Infallible>(app.handle(req).await) }
            });
            let connection = hyper::server::conn::http1::Builder::new()
                .serve_connection(TokioIo::new(stream), service);
            if let Err(e) = connection.await {
                // Client disconnects are routine; log at debug level only.
                tracing::debug!("connection ended: {e}");
            }
        });
    }
}
