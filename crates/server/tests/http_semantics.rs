//! HTTP-layer conformance semantics, tested against the real handler with
//! the committed fixture — no sockets, exact header assertions.

use hyper::{Request, StatusCode};
use iiif_core::info::Limits;
use iiif_server::app::{App, SourceRoot};
use iiif_sources::LocalRoot;
use std::path::Path;
use std::sync::Arc;
use tokio::sync::Semaphore;

fn fixture_root() -> SourceRoot {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures");
    SourceRoot::Local(LocalRoot::new(&root).expect("fixture dir exists"))
}

fn app() -> Arc<App> {
    Arc::new(App {
        root: fixture_root(),
        limits: Limits {
            max_width: 8192,
            max_height: 8192,
            max_area: 67_108_864,
        },
        public_base: Some("https://images.example.org".to_owned()),
        admission: Arc::new(Semaphore::new(8)),
        decode_permits: Arc::new(Semaphore::new(4)),
        workers: 4,
        queue_depth: 4,
        metrics: Arc::new(iiif_server::metrics::Metrics::default()),
    })
}

async fn get(app: &Arc<App>, path: &str) -> hyper::Response<http_body_util::Full<bytes::Bytes>> {
    let req = Request::get(path).body(()).unwrap();
    Arc::clone(app).handle(req).await
}

fn header<'r>(
    response: &'r hyper::Response<http_body_util::Full<bytes::Bytes>>,
    name: &str,
) -> &'r str {
    response
        .headers()
        .get(name)
        .unwrap_or_else(|| panic!("{name} header missing"))
        .to_str()
        .unwrap()
}

#[tokio::test]
async fn base_uri_redirects_to_info() {
    let app = app();
    let response = get(&app, "/iiif/3/rgb_pyramid.tif").await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        header(&response, "location"),
        "/iiif/3/rgb_pyramid.tif/info.json"
    );
    assert_eq!(header(&response, "access-control-allow-origin"), "*");
}

#[tokio::test]
async fn info_json_semantics() {
    let app = app();
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/info.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "application/json");
    assert_eq!(header(&response, "vary"), "Accept");
    assert!(header(&response, "link").contains("level2.json>;rel=\"profile\""));

    // JSON-LD negotiation flips the media type.
    let req = Request::get("/iiif/3/rgb_pyramid.tif/info.json")
        .header("accept", "application/ld+json")
        .body(())
        .unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert!(header(&response, "content-type").starts_with("application/ld+json;profile="));
}

#[tokio::test]
async fn info_json_uses_public_base() {
    use http_body_util::BodyExt;
    let app = app();
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/info.json").await;
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        json["id"],
        "https://images.example.org/iiif/3/rgb_pyramid.tif"
    );
    assert_eq!(json["profile"], "level2");
}

#[tokio::test]
async fn image_carries_canonical_link() {
    let app = app();
    let response = get(
        &app,
        "/iiif/3/rgb_pyramid.tif/0,0,512,512/256,/0/default.jpg",
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "image/jpeg");
    let link = header(&response, "link");
    assert!(
        link.contains(
            "<https://images.example.org/iiif/3/rgb_pyramid.tif/0,0,512,512/256,256/0/default.jpg>;rel=\"canonical\""
        ),
        "unexpected link: {link}"
    );
    assert!(link.contains("rel=\"profile\""));
}

#[tokio::test]
async fn head_returns_headers_without_body() {
    use http_body_util::BodyExt;
    let app = app();
    let req = Request::head("/iiif/3/rgb_pyramid.tif/info.json")
        .body(())
        .unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "application/json");
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty());
}

#[tokio::test]
async fn options_preflight() {
    let app = app();
    let req = Request::options("/iiif/3/x").body(()).unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert_eq!(response.status(), StatusCode::NO_CONTENT);
    assert_eq!(header(&response, "access-control-allow-origin"), "*");
    assert!(header(&response, "access-control-allow-methods").contains("GET"));
}

#[tokio::test]
async fn error_semantics() {
    let app = app();
    // Unknown identifier → 404.
    let response = get(&app, "/iiif/3/missing.tif/info.json").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // Raw slash in identifier → 404 (segment shape).
    let response = get(&app, "/iiif/3/a/b/full/max/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // Malformed size → 400.
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/nope/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    // Arbitrary rotation is implemented (canvas grows, corners filled).
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/max/45/default.jpg").await;
    assert_eq!(response.status(), StatusCode::OK);
    // The complete output table encodes — webp included (lossless).
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/max/0/default.webp").await;
    assert_eq!(response.status(), StatusCode::OK);
    // Traversal → 404.
    let response = get(&app, "/iiif/3/..%2Fsecret/full/max/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    // Wrong method → 405.
    let req = Request::post("/iiif/3/rgb_pyramid.tif/info.json")
        .body(())
        .unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn saturated_queue_returns_503_with_retry_after() {
    let app = Arc::new(App {
        root: fixture_root(),
        limits: Limits {
            max_width: 8192,
            max_height: 8192,
            max_area: 67_108_864,
        },
        public_base: None,
        // Zero admission permits: every image request is over capacity.
        admission: Arc::new(Semaphore::new(0)),
        decode_permits: Arc::new(Semaphore::new(1)),
        workers: 1,
        queue_depth: 0,
        metrics: Arc::new(iiif_server::metrics::Metrics::default()),
    });
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/max/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(header(&response, "retry-after"), "2");
}

#[tokio::test]
async fn v2_endpoint_semantics() {
    use http_body_util::BodyExt;
    let app = app();
    // v2 info.json: @id + profile array.
    let response = get(&app, "/iiif/2/rgb_pyramid.tif/info.json").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(header(&response, "link").contains("image/2/level2.json"));
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["@context"], "http://iiif.io/api/image/2/context.json");
    assert_eq!(
        json["@id"],
        "https://images.example.org/iiif/2/rgb_pyramid.tif"
    );
    assert_eq!(json["profile"][0], "http://iiif.io/api/image/2/level2.json");

    // v2 base redirect stays inside /iiif/2/.
    let response = get(&app, "/iiif/2/rgb_pyramid.tif").await;
    assert_eq!(
        header(&response, "location"),
        "/iiif/2/rgb_pyramid.tif/info.json"
    );

    // sizeAboveFull: upscaling without ^ is legal in v2 …
    let response = get(&app, "/iiif/2/rgb_pyramid.tif/full/1200,/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::OK);
    // … and the canonical link uses the v2 `w,` spelling.
    assert!(
        header(&response, "link").contains("/iiif/2/rgb_pyramid.tif/full/1200,/0/default.jpg>"),
        "got {}",
        header(&response, "link")
    );

    // `full` size and `^` behave per version: full is v2-only, ^ is v3-only.
    let response = get(&app, "/iiif/2/rgb_pyramid.tif/full/full/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::OK);
    let response = get(&app, "/iiif/2/rgb_pyramid.tif/full/^max/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/full/0/default.jpg").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/1200,/0/default.jpg").await;
    assert_eq!(
        response.status(),
        StatusCode::BAD_REQUEST,
        "v3 upscale still needs ^"
    );
}

#[tokio::test]
async fn etag_and_conditional_requests() {
    use http_body_util::BodyExt;
    let app = app();
    // info.json carries a strong ETag + Cache-Control.
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/info.json").await;
    let etag = header(&response, "etag").to_owned();
    assert!(etag.starts_with('"') && etag.ends_with('"'), "strong ETag");
    assert_eq!(header(&response, "cache-control"), "public, max-age=3600");

    // Revalidation → 304 with the same tag and no body.
    let req = Request::get("/iiif/3/rgb_pyramid.tif/info.json")
        .header("if-none-match", &etag)
        .body(())
        .unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
    assert_eq!(header(&response, "etag"), etag);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    assert!(body.is_empty());

    // Image ETags key on the CANONICAL request: two spellings of the same
    // pixels revalidate against one tag.
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/512,/0/default.jpg").await;
    let image_etag = header(&response, "etag").to_owned();
    let req = Request::get("/iiif/3/rgb_pyramid.tif/full/512,384/0/default.jpg")
        .header("if-none-match", &image_etag)
        .body(())
        .unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert_eq!(
        response.status(),
        StatusCode::NOT_MODIFIED,
        "canonical-equivalent spelling must revalidate"
    );

    // A different request has a different tag.
    let response = get(&app, "/iiif/3/rgb_pyramid.tif/full/256,/0/default.jpg").await;
    assert_ne!(header(&response, "etag"), image_etag);

    // Wildcard matches.
    let req = Request::get("/iiif/3/rgb_pyramid.tif/full/512,/0/default.jpg")
        .header("if-none-match", "*")
        .body(())
        .unwrap();
    let response = Arc::clone(&app).handle(req).await;
    assert_eq!(response.status(), StatusCode::NOT_MODIFIED);
}

#[tokio::test]
async fn metrics_render_the_frozen_set() {
    use http_body_util::BodyExt;
    let app = app();
    // Generate one of each family.
    let _ = get(&app, "/iiif/3/rgb_pyramid.tif/info.json").await;
    let _ = get(&app, "/iiif/3/rgb_pyramid.tif/full/64,/0/default.jpg").await;
    let response = get(&app, "/metrics").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let text = std::str::from_utf8(&body).unwrap();
    assert!(
        text.contains("iiif_requests_total{family=\"info\"} 1"),
        "{text}"
    );
    assert!(text.contains("iiif_requests_total{family=\"image\"} 1"));
    assert!(text.contains("iiif_responses_total{class=\"2xx\"} 2"));
    assert!(text.contains("iiif_request_duration_seconds_count 2"));
    assert!(text.contains("iiif_decode_in_flight 0"));
    assert!(text.contains("iiif_overload_total 0"));
}
