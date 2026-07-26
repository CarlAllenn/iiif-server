//! Pure library for the IIIF Image API engine: URL grammar, identifier
//! rules, the byte-range source seam, info.json, and the image pipeline.
//!
//! The grammar layer does no I/O. Nothing in this crate touches the network
//! or filesystem; sources are abstracted behind [`source::ByteRangeSource`].

pub mod codec;
pub mod encode;
pub mod eval;
pub mod grammar;
pub mod ident;
pub mod image;
pub mod info;
pub mod pipeline;
pub mod source;
pub mod v2;
