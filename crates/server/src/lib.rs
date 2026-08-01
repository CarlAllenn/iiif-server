//! Library surface of the server crate: the HTTP application, exposed so
//! integration tests exercise exact response semantics without sockets.

pub mod app;
pub mod metrics;

/// Release version, from the crate manifest — the single source. Reported by
/// `--version` and by the `iiif_build_info` metric.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Commit the binary was built from, or `unknown`.
///
/// Injected at build time by the release pipeline (`IIIF_BUILD_REVISION`).
/// Absent in ordinary `cargo build`s, where there is no meaningful answer —
/// a working tree is not a revision.
pub const REVISION: &str = match option_env!("IIIF_BUILD_REVISION") {
    Some(revision) => revision,
    None => "unknown",
};
