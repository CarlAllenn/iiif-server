//! The HTTP application: routing (the IIIF grammar *is* the router),
//! spec-mandated response semantics, CORS, content negotiation, and
//! backpressure. Pure request→response logic; `main.rs` owns sockets and
//! runtime.

use bytes::Bytes;
use http_body_util::Full;
use hyper::header::{ACCEPT, ALLOW, CONTENT_TYPE, HeaderValue, LINK, LOCATION, RETRY_AFTER, VARY};
use hyper::{Method, Request, Response, StatusCode};
use iiif_core::codec::{CodecError, open_master};
use iiif_core::encode::EncodeError;
use iiif_core::eval::{EvalError, evaluate};
use iiif_core::grammar::{ImageRequest, ParseError};
use iiif_core::ident::Identifier;
use iiif_core::info::{Info, Limits};
use iiif_core::pipeline::{self, PipelineError};
use iiif_core::source::SourceError;
use iiif_sources::LocalRoot;
use std::sync::Arc;
use tokio::sync::Semaphore;

/// JSON-LD media type with the required profile parameter.
const LD_JSON: &str = "application/ld+json;profile=\"http://iiif.io/api/image/3/context.json\"";
const LD_JSON_V2: &str = "application/ld+json;profile=\"http://iiif.io/api/image/2/context.json\"";

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
    fn prefix(self) -> &'static str {
        match self {
            Self::V2 => "iiif/2",
            Self::V3 => "iiif/3",
        }
    }

    fn profile_link(self) -> &'static str {
        match self {
            Self::V2 => PROFILE_LINK_V2,
            Self::V3 => PROFILE_LINK,
        }
    }

    fn ld_json(self) -> &'static str {
        match self {
            Self::V2 => LD_JSON_V2,
            Self::V3 => LD_JSON,
        }
    }
}

/// Shared server state: source root, limits, and the bounded decode pool.
pub struct App {
    pub root: LocalRoot,
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
}

impl App {
    /// Route and answer one request. Infallible at the HTTP layer: every
    /// failure becomes a spec-mandated status.
    ///
    /// # Panics
    ///
    /// Only if `hyper`'s response builder rejects statically valid
    /// header/status combinations — structurally impossible.
    pub async fn handle<B>(self: Arc<Self>, req: Request<B>) -> Response<Full<Bytes>> {
        let method = req.method().clone();
        if method == Method::OPTIONS {
            return preflight();
        }
        if !matches!(method, Method::GET | Method::HEAD) {
            return error(StatusCode::METHOD_NOT_ALLOWED, "only GET and HEAD");
        }
        let path = req.uri().path().to_owned();
        let mut response = match Route::of(&path) {
            Route::Health => Response::builder()
                .status(StatusCode::OK)
                .header(CONTENT_TYPE, "text/plain")
                .body(Full::new(Bytes::from_static(b"ok\n")))
                .expect("static response"),
            Route::BaseRedirect {
                version,
                identifier,
            } => match Identifier::decode(identifier) {
                Ok(id) => Response::builder()
                    .status(StatusCode::SEE_OTHER)
                    .header(
                        LOCATION,
                        format!("/{}/{}/info.json", version.prefix(), id.encoded()),
                    )
                    .body(Full::new(Bytes::new()))
                    .expect("static response"),
                Err(_) => error(StatusCode::NOT_FOUND, "unknown identifier"),
            },
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
                self.image(version, identifier, rest, &base).await
            }
            Route::None => error(StatusCode::NOT_FOUND, "no such resource"),
        };
        add_cors(&mut response);
        if method == Method::HEAD {
            *response.body_mut() = Full::new(Bytes::new());
        }
        response
    }

    async fn info_json<B>(
        &self,
        version: Version,
        raw_id: &str,
        req: &Request<B>,
    ) -> Response<Full<Bytes>> {
        let Ok(id) = Identifier::decode(raw_id) else {
            return error(StatusCode::NOT_FOUND, "unknown identifier");
        };
        let source = match self.root.resolve(&id) {
            Ok(source) => source,
            Err(e) => return source_error(&e),
        };
        let opened = tokio::task::spawn_blocking(move || {
            let file = source
                .into_std_file()
                .map_err(|e| CodecError::Corrupt(format!("file handle: {e}")))?;
            open_master(file).map(|master| master.describe())
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
        Response::builder()
            .status(StatusCode::OK)
            .header(CONTENT_TYPE, content_type)
            .header(VARY, "Accept")
            .header(LINK, version.profile_link())
            .body(Full::new(Bytes::from(body)))
            .expect("valid response")
    }

    async fn image(
        &self,
        version: Version,
        raw_id: &str,
        rest: &str,
        base: &str,
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
        let source = match self.root.resolve(&id) {
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
        let result = tokio::task::spawn_blocking(move || {
            let _permit = permit; // held for the duration of the decode
            let _admission = admission;
            let file = source.into_std_file().map_err(|e| {
                ImageFailure::Codec(CodecError::Corrupt(format!("file handle: {e}")))
            })?;
            let mut master = open_master(file)?;
            let (full_w, full_h) = master.dimensions();
            let plan = evaluate(&request, full_w, full_h, limits).map_err(ImageFailure::Eval)?;
            let bytes =
                pipeline::execute(master.as_mut(), &plan).map_err(ImageFailure::Pipeline)?;
            Ok::<_, ImageFailure>((bytes, plan))
        })
        .await;
        match result {
            Ok(Ok((bytes, plan))) => {
                // Optional features `canonicalLinkHeader` + `profileLinkHeader`.
                let canonical_path = match v2_spelling {
                    Some(v2) => iiif_core::v2::canonical_path(&plan, &v2),
                    None => plan.canonical_path(),
                };
                let canonical = format!(
                    "<{base}/{}/{}/{canonical_path}>;rel=\"canonical\", {}",
                    version.prefix(),
                    id.encoded(),
                    version.profile_link()
                );
                Response::builder()
                    .status(StatusCode::OK)
                    .header(CONTENT_TYPE, plan.format.media_type())
                    .header(LINK, canonical)
                    .body(Full::new(Bytes::from(bytes)))
                    .expect("valid response")
            }
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
            Self::Pipeline(PipelineError::ArbitraryRotationUnimplemented) => error(
                StatusCode::NOT_IMPLEMENTED,
                "arbitrary rotation is not implemented yet",
            ),
            Self::Pipeline(PipelineError::Encode(EncodeError::UnsupportedFormat(f))) => error(
                StatusCode::BAD_REQUEST,
                &format!("format {f} is not supported by this build"),
            ),
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
    let mut response = Response::builder()
        .status(StatusCode::NO_CONTENT)
        .header(ALLOW, "GET, HEAD, OPTIONS")
        .header("access-control-allow-methods", "GET, HEAD, OPTIONS")
        .header("access-control-allow-headers", "Accept")
        .header("access-control-max-age", "86400")
        .body(Full::new(Bytes::new()))
        .expect("static response");
    add_cors(&mut response);
    response
}

fn error(status: StatusCode, message: &str) -> Response<Full<Bytes>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(Full::new(Bytes::from(format!("{message}\n"))))
        .expect("valid response")
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
