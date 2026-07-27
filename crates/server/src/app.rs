//! The HTTP application: routing (the IIIF grammar *is* the router),
//! spec-mandated response semantics, CORS, content negotiation, and
//! backpressure.
//!
//! Pure request→response logic; `main.rs` owns sockets and runtime.

use std::{
    fmt,
    hash::{Hash, Hasher},
    sync::Arc,
    time::Instant,
};

use bytes::Bytes;
use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode,
    header::{
        ACCEPT, ALLOW, CACHE_CONTROL, CONTENT_TYPE, ETAG, HeaderValue, IF_NONE_MATCH, LINK,
        LOCATION, RETRY_AFTER, VARY,
    },
};
use iiif_core::{
    codec::{CodecError, open_master},
    encode::EncodeError,
    eval::{EvalError, evaluate},
    grammar::{ImageRequest, ParseError},
    ident::Identifier,
    info::{Info, Limits},
    pipeline::{self, PipelineError},
    source::SourceError,
};
use iiif_sources::{LocalFile, LocalRoot, ObjectRoot};
use tokio::sync::Semaphore;

use crate::metrics::{Family, Metrics};

/// JSON-LD media type with the required profile parameter.
const LD_JSON: &str = "application/ld+json;profile=\"http://iiif.io/api/image/3/context.json\"";
const LD_JSON_V2: &str = "application/ld+json;profile=\"http://iiif.io/api/image/2/context.json\"";

/// M5 cache posture: strong validator (`ETag`) plus a modest freshness
/// window — the CDN/proxy in front owns long-lived caching policy; our
/// job is correct revalidation semantics.
const CACHE_CONTROL_VALUE: &str = "public, max-age=3600";

/// The M5 `ETag` definition: hash of (source identity, source version
/// [mtime+size], canonical request URI, binary version). Cheap, correct,
/// no state. Two `DefaultHasher` passes with domain separation give 128
/// bits against accidental collision; `DefaultHasher::new()` is
/// deterministic across runs of the same binary, and the binary version
/// is part of the input.
fn etag_for(identifier: &str, source_version: (u64, u64), canonical: &str) -> String {
    let mut halves = [0u64; 2];
    for (domain, half) in halves.iter_mut().enumerate() {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        domain.hash(&mut hasher);
        identifier.hash(&mut hasher);
        source_version.hash(&mut hasher);
        canonical.hash(&mut hasher);
        env!("CARGO_PKG_VERSION").hash(&mut hasher);
        *half = hasher.finish();
    }
    format!("\"{:016x}{:016x}\"", halves[0], halves[1])
}

/// True when an `If-None-Match` header matches this `ETag` (or is `*`).
fn if_none_match_hits<B>(req: &Request<B>, etag: &str) -> bool {
    req.headers()
        .get(IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| {
            value == "*"
                || value
                    .split(',')
                    .any(|candidate| candidate.trim().trim_start_matches("W/") == etag)
        })
}

/// Finish a response builder whose inputs are statically valid. The builder
/// only errors on malformed status codes or header values; every call site
/// passes constants or already-validated strings, so the fallback exists
/// purely to keep the server panic-free if that invariant ever breaks.
fn built(res: hyper::http::Result<Response<Full<Bytes>>>) -> Response<Full<Bytes>> {
    res.unwrap_or_else(|_| {
        let mut fallback = Response::new(Full::new(Bytes::new()));
        *fallback.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        fallback
    })
}

fn not_modified(etag: &str) -> Response<Full<Bytes>> {
    built(
        Response::builder()
            .status(StatusCode::NOT_MODIFIED)
            .header(ETAG, etag)
            .header(CACHE_CONTROL, CACHE_CONTROL_VALUE)
            .body(Full::new(Bytes::new())),
    )
}

/// The compliance-level profile documents, sent as a Link header on every
/// image and info.json response (optional feature `profileLinkHeader`).
const PROFILE_LINK: &str = "<http://iiif.io/api/image/3/level2.json>;rel=\"profile\"";
const PROFILE_LINK_V2: &str = "<http://iiif.io/api/image/2/level2.json>;rel=\"profile\"";

/// Which API family a request addresses. Both mount over the same engine
/// (design spec: the v2.1 endpoint is a translation layer, M3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Version {
    V2,
    V3,
}

impl Version {
    const fn prefix(self) -> &'static str {
        match self {
            Self::V2 => "iiif/2",
            Self::V3 => "iiif/3",
        }
    }

    const fn profile_link(self) -> &'static str {
        match self {
            Self::V2 => PROFILE_LINK_V2,
            Self::V3 => PROFILE_LINK,
        }
    }

    const fn ld_json(self) -> &'static str {
        match self {
            Self::V2 => LD_JSON_V2,
            Self::V3 => LD_JSON,
        }
    }
}

/// Where masters come from: a local directory or an S3-compatible
/// object store — the prefix→root map whose default size is one.
#[derive(Debug)]
pub enum SourceRoot {
    /// Masters under a local directory.
    Local(LocalRoot),
    /// Masters in an S3-compatible object store.
    Object(ObjectRoot),
}

/// One resolved master, ready for the sync decoder bridge.
enum Resolved {
    Local(LocalFile),
    Object(Bytes, (u64, u64)),
}

impl SourceRoot {
    async fn resolve(&self, id: &Identifier) -> Result<Resolved, SourceError> {
        match self {
            Self::Local(root) => root.resolve(id).map(Resolved::Local),
            Self::Object(root) => root
                .resolve(id)
                .await
                .map(|(bytes, version)| Resolved::Object(bytes, version)),
        }
    }
}

impl Resolved {
    const fn source_version(&self) -> (u64, u64) {
        match self {
            Self::Local(file) => file.source_version(),
            Self::Object(_, version) => *version,
        }
    }

    fn into_reader(self) -> std::io::Result<SourceReader> {
        Ok(match self {
            Self::Local(file) => SourceReader::File(file.into_std_file()?),
            Self::Object(bytes, _) => SourceReader::Memory(std::io::Cursor::new(bytes)),
        })
    }
}

/// The sync `Read + Seek` bridge the codecs consume.
enum SourceReader {
    File(std::fs::File),
    Memory(std::io::Cursor<Bytes>),
}

impl std::io::Read for SourceReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Self::File(file) => std::io::Read::read(file, buf),
            Self::Memory(cursor) => std::io::Read::read(cursor, buf),
        }
    }
}

impl std::io::Seek for SourceReader {
    fn seek(&mut self, pos: std::io::SeekFrom) -> std::io::Result<u64> {
        match self {
            Self::File(file) => std::io::Seek::seek(file, pos),
            Self::Memory(cursor) => std::io::Seek::seek(cursor, pos),
        }
    }
}

/// Shared server state: source root, limits, and the bounded decode pool.
pub struct App {
    /// Where masters are resolved from.
    pub root: SourceRoot,
    /// Deployment size limits, published in info.json and enforced.
    pub limits: Limits,
    /// Public `scheme://authority/prefix` used to build `id` values,
    /// derived from the request Host header when absent.
    pub public_base: Option<String>,
    /// Admission permits = workers + queue depth: a failed try-acquire
    /// means the queue is full → 503 with Retry-After.
    pub admission: Arc<Semaphore>,
    /// Execution permits = workers: bounds concurrent pixel work; waiting
    /// here is bounded because admission already capped the waiters.
    pub decode_permits: Arc<Semaphore>,
    /// Worker-pool sizing, kept for the queue-depth gauges.
    pub workers: usize,
    /// Admission queue length beyond the worker pool.
    pub queue_depth: usize,
    /// The frozen metric set (design spec, Observability).
    pub metrics: Arc<Metrics>,
}

impl fmt::Debug for App {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("App")
            .field("root", &self.root)
            .field("limits", &self.limits)
            .field("public_base", &self.public_base)
            .field("workers", &self.workers)
            .field("queue_depth", &self.queue_depth)
            .finish_non_exhaustive()
    }
}

impl App {
    /// Route and answer one request. Infallible at the HTTP layer: every
    /// failure becomes a spec-mandated status.
    ///
    /// # Panics
    ///
    /// Only if `hyper`'s response builder rejects statically valid
    /// header/status combinations — structurally impossible.
    pub async fn handle<B: Send + Sync>(self: Arc<Self>, req: Request<B>) -> Response<Full<Bytes>> {
        let started = Instant::now();
        let family = match Route::of(req.uri().path()) {
            Route::InfoJson { .. } => Family::Info,
            Route::Image { .. } => Family::Image,
            _ => Family::Other,
        };
        let response = self.handle_inner(req).await;
        self.metrics
            .observe(family, response.status().as_u16(), started.elapsed());
        response
    }

    async fn handle_inner<B: Send + Sync>(
        self: &Arc<Self>,
        req: Request<B>,
    ) -> Response<Full<Bytes>> {
        let method = req.method().clone();
        if method == Method::OPTIONS {
            return preflight();
        }
        if !matches!(method, Method::GET | Method::HEAD) {
            return error(StatusCode::METHOD_NOT_ALLOWED, "only GET and HEAD");
        }
        let path = req.uri().path().to_owned();
        let mut response = match Route::of(&path) {
            Route::Health => built(
                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, "text/plain")
                    .body(Full::new(Bytes::from_static(b"ok\n"))),
            ),
            Route::Metrics => {
                let in_flight = self
                    .workers
                    .saturating_sub(self.decode_permits.available_permits());
                let admitted = (self.workers + self.queue_depth)
                    .saturating_sub(self.admission.available_permits());
                let queued = admitted.saturating_sub(in_flight);
                let body = self.metrics.render(in_flight as u64, queued as u64);
                built(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, "text/plain; version=0.0.4")
                        .body(Full::new(Bytes::from(body))),
                )
            },
            Route::BaseRedirect {
                version,
                identifier,
            } => Identifier::decode(identifier).map_or_else(
                |_| error(StatusCode::NOT_FOUND, "unknown identifier"),
                |id| {
                    built(
                        Response::builder()
                            .status(StatusCode::SEE_OTHER)
                            .header(
                                LOCATION,
                                format!("/{}/{}/info.json", version.prefix(), id.encoded()),
                            )
                            .body(Full::new(Bytes::new())),
                    )
                },
            ),
            Route::InfoJson {
                version,
                identifier,
            } => self.info_json(version, identifier, &req).await,
            Route::Image {
                version,
                identifier,
                rest,
            } => {
                let base = self.base_uri(&req);
                let if_none_match = req
                    .headers()
                    .get(IF_NONE_MATCH)
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_owned);
                self.image(version, identifier, rest, &base, if_none_match)
                    .await
            },
            Route::None => error(StatusCode::NOT_FOUND, "no such resource"),
        };
        add_cors(&mut response);
        if method == Method::HEAD {
            *response.body_mut() = Full::new(Bytes::new());
        }
        response
    }

    async fn info_json<B: Sync>(
        &self,
        version: Version,
        raw_id: &str,
        req: &Request<B>,
    ) -> Response<Full<Bytes>> {
        let Ok(id) = Identifier::decode(raw_id) else {
            return error(StatusCode::NOT_FOUND, "unknown identifier");
        };
        let source = match self.root.resolve(&id).await {
            Ok(source) => source,
            Err(e) => return source_error(&e),
        };
        // ETag first: a revalidation hit never opens the master at all.
        let etag = etag_for(
            id.as_path(),
            source.source_version(),
            &format!("{}/info.json", version.prefix()),
        );
        if if_none_match_hits(req, &etag) {
            return not_modified(&etag);
        }
        let opened = tokio::task::spawn_blocking(move || {
            let reader = source
                .into_reader()
                .map_err(|e| CodecError::Corrupt(format!("source handle: {e}")))?;
            open_master(reader).map(|master| master.describe())
        })
        .await;
        let description = match opened {
            Ok(Ok(description)) => description,
            Ok(Err(e)) => return codec_error(&e),
            Err(_) => return error(StatusCode::INTERNAL_SERVER_ERROR, "decode task failed"),
        };
        let base = self.base_uri(req);
        let document_id = format!("{base}/{}/{}", version.prefix(), id.encoded());
        let body = match version {
            Version::V3 => Info::new(document_id, &description, self.limits).to_json(),
            Version::V2 => iiif_core::v2::info_json(&document_id, &description, self.limits),
        };
        // Content negotiation (§5.2): ld+json when asked for, with Vary.
        let accept = req
            .headers()
            .get(ACCEPT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let content_type = if accept.contains("application/ld+json") {
            version.ld_json()
        } else {
            "application/json"
        };
        built(
            Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, content_type)
                .header(VARY, "Accept")
                .header(LINK, version.profile_link())
                .header(ETAG, etag)
                .header(CACHE_CONTROL, CACHE_CONTROL_VALUE)
                .body(Full::new(Bytes::from(body))),
        )
    }

    async fn image(
        &self,
        version: Version,
        raw_id: &str,
        rest: &str,
        base: &str,
        if_none_match: Option<String>,
    ) -> Response<Full<Bytes>> {
        let Ok(id) = Identifier::decode(raw_id) else {
            return error(StatusCode::NOT_FOUND, "unknown identifier");
        };
        let (request, v2_spelling) = match version {
            Version::V3 => match ImageRequest::parse(rest) {
                Ok(request) => (request, None),
                Err(e) => return parse_error(&e),
            },
            Version::V2 => match iiif_core::v2::parse_image_request(rest) {
                Ok(parsed) => (parsed.request, Some(parsed)),
                Err(e) => return parse_error(&e),
            },
        };
        let source = match self.root.resolve(&id).await {
            Ok(source) => source,
            Err(e) => return source_error(&e),
        };
        // Backpressure: admission bounds the queue (full → 503),
        // execution bounds concurrent decode work.
        let Ok(admission) = Arc::clone(&self.admission).try_acquire_owned() else {
            return overloaded();
        };
        let Ok(permit) = Arc::clone(&self.decode_permits).acquire_owned().await else {
            return error(StatusCode::INTERNAL_SERVER_ERROR, "pool closed");
        };
        let limits = self.limits;
        // Spare workers → let codecs use their own internal parallelism.
        let pool_idle = self.decode_permits.available_permits() > 0;
        let source_version = source.source_version();
        let identifier_path = id.as_path().to_owned();
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit; // held for the duration of the decode
            let _admission = admission;
            let reader = source.into_reader().map_err(|e| {
                ImageFailure::Codec(CodecError::Corrupt(format!("source handle: {e}")))
            })?;
            let mut master = open_master(reader)?;
            master.set_internal_parallelism(pool_idle);
            let (full_w, full_h) = master.dimensions();
            let plan = evaluate(&request, full_w, full_h, limits).map_err(ImageFailure::Eval)?;
            let canonical_path = v2_spelling.as_ref().map_or_else(
                || plan.canonical_path(),
                |v2| iiif_core::v2::canonical_path(&plan, v2),
            );
            // ETag is derived from the canonical URI, so every spelling of
            // the same request revalidates against the same tag — and a
            // hit skips all pixel work.
            let etag = etag_for(&identifier_path, source_version, &canonical_path);
            if let Some(candidates) = &if_none_match
                && (candidates == "*"
                    || candidates
                        .split(',')
                        .any(|c| c.trim().trim_start_matches("W/") == etag))
            {
                return Ok(ImageOutcome::NotModified { etag });
            }
            let bytes =
                pipeline::execute(master.as_mut(), &plan).map_err(ImageFailure::Pipeline)?;
            Ok::<_, ImageFailure>(ImageOutcome::Fresh {
                bytes,
                plan,
                canonical_path,
                etag,
            })
        })
        .await;
        match result {
            Ok(Ok(ImageOutcome::NotModified { etag })) => not_modified(&etag),
            Ok(Ok(ImageOutcome::Fresh {
                bytes,
                plan,
                canonical_path,
                etag,
            })) => {
                // Optional features `canonicalLinkHeader` + `profileLinkHeader`.
                let canonical = format!(
                    "<{base}/{}/{}/{canonical_path}>;rel=\"canonical\", {}",
                    version.prefix(),
                    id.encoded(),
                    version.profile_link()
                );
                built(
                    Response::builder()
                        .status(StatusCode::OK)
                        .header(CONTENT_TYPE, plan.format.media_type())
                        .header(LINK, canonical)
                        .header(ETAG, etag)
                        .header(CACHE_CONTROL, CACHE_CONTROL_VALUE)
                        .body(Full::new(Bytes::from(bytes))),
                )
            },
            Ok(Err(failure)) => failure.into_response(),
            Err(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "decode task failed"),
        }
    }

    fn base_uri<B>(&self, req: &Request<B>) -> String {
        if let Some(base) = &self.public_base {
            return base.clone();
        }
        let host = req
            .headers()
            .get(hyper::header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("localhost");
        format!("http://{host}")
    }
}

/// What the blocking image task produced.
enum ImageOutcome {
    /// Revalidation hit: no pixel work was done.
    NotModified { etag: String },
    Fresh {
        bytes: Vec<u8>,
        plan: iiif_core::eval::Plan,
        canonical_path: String,
        etag: String,
    },
}

/// Failures on the image path, unified for status mapping.
enum ImageFailure {
    Codec(CodecError),
    Eval(EvalError),
    Pipeline(PipelineError),
}

impl From<CodecError> for ImageFailure {
    fn from(e: CodecError) -> Self {
        Self::Codec(e)
    }
}

impl ImageFailure {
    fn into_response(self) -> Response<Full<Bytes>> {
        match self {
            Self::Eval(e) => error(StatusCode::BAD_REQUEST, &e.to_string()),
            Self::Codec(e) => codec_error(&e),
            Self::Pipeline(PipelineError::Encode(EncodeError::DimensionsBeyondFormat {
                ..
            })) => error(StatusCode::BAD_REQUEST, "output too large for this format"),
            Self::Pipeline(e) => error(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
        }
    }
}

/// The resource shapes under `/iiif/3/` and `/iiif/2/`.
enum Route<'p> {
    Health,
    Metrics,
    BaseRedirect {
        version: Version,
        identifier: &'p str,
    },
    InfoJson {
        version: Version,
        identifier: &'p str,
    },
    Image {
        version: Version,
        identifier: &'p str,
        rest: &'p str,
    },
    None,
}

impl<'p> Route<'p> {
    fn of(path: &'p str) -> Self {
        if path == "/healthz" {
            return Self::Health;
        }
        if path == "/metrics" {
            return Self::Metrics;
        }
        let (version, rest) = if let Some(rest) = path.strip_prefix("/iiif/3/") {
            (Version::V3, rest)
        } else if let Some(rest) = path.strip_prefix("/iiif/2/") {
            (Version::V2, rest)
        } else {
            return Self::None;
        };
        // Exact segment shapes only. An identifier containing a raw
        // (unescaped) slash changes the segment count and falls through
        // to 404, as the spec requires for unescaped special characters.
        let segments: Vec<&str> = rest.split('/').collect();
        match segments.as_slice() {
            [identifier] if !identifier.is_empty() => Self::BaseRedirect {
                version,
                identifier,
            },
            [identifier, "info.json"] => Self::InfoJson {
                version,
                identifier,
            },
            [identifier, _region, _size, _rotation, _file] => Self::Image {
                version,
                identifier,
                rest: &rest[identifier.len() + 1..],
            },
            _ => Self::None,
        }
    }
}

fn add_cors(response: &mut Response<Full<Bytes>>) {
    response
        .headers_mut()
        .insert("access-control-allow-origin", HeaderValue::from_static("*"));
}

fn preflight() -> Response<Full<Bytes>> {
    let mut response = built(
        Response::builder()
            .status(StatusCode::NO_CONTENT)
            .header(ALLOW, "GET, HEAD, OPTIONS")
            .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
            .header("access-control-allow-headers", "Accept")
            .header("access-control-max-age", "86400")
            .body(Full::new(Bytes::new())),
    );
    add_cors(&mut response);
    response
}

fn error(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    built(
        Response::builder()
            .status(status)
            .header(CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Full::new(Bytes::from(format!("{message}\n")))),
    )
}

fn overloaded() -> Response<Full<Bytes>> {
    let mut response = error(StatusCode::SERVICE_UNAVAILABLE, "decode pool saturated");
    response
        .headers_mut()
        .insert(RETRY_AFTER, HeaderValue::from_static("2"));
    response
}

fn parse_error(e: &ParseError) -> Response<Full<Bytes>> {
    error(StatusCode::BAD_REQUEST, &e.to_string())
}

fn source_error(e: &SourceError) -> Response<Full<Bytes>> {
    match e {
        SourceError::NotFound => error(StatusCode::NOT_FOUND, "unknown identifier"),
        _ => error(StatusCode::INTERNAL_SERVER_ERROR, "source read failed"),
    }
}

fn codec_error(e: &CodecError) -> Response<Full<Bytes>> {
    match e {
        // An operator-side master problem: the identifier exists but is
        // outside the supported matrix. 500-class (the client did nothing
        // wrong), message actionable.
        CodecError::Unsupported(msg) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("unsupported master: {msg}"),
        ),
        CodecError::Corrupt(msg) => error(
            StatusCode::INTERNAL_SERVER_ERROR,
            &format!("corrupt master: {msg}"),
        ),
        CodecError::Raster(_) => error(StatusCode::INTERNAL_SERVER_ERROR, "pipeline failure"),
    }
}
