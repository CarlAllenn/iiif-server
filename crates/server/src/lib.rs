//! Library surface of the server crate: the HTTP application, exposed so
//! integration tests exercise exact response semantics without sockets.

pub mod app;
pub mod metrics;
