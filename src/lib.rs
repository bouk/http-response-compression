//! HTTP response compression middleware for Tower.
//!
//! This crate provides a Tower layer that automatically compresses HTTP response
//! bodies using Zstd, Brotli, Gzip, or Deflate based on the client's `Accept-Encoding` header.
//!
//! # Example
//!
//! ```ignore
//! use http_response_compression::CompressionLayer;
//! use tower::ServiceBuilder;
//!
//! let service = ServiceBuilder::new()
//!     .layer(CompressionLayer::new())
//!     .service(my_service);
//! ```
//!
//! # Compression Rules
//!
//! The middleware will **not** compress responses when:
//! - No supported `Accept-Encoding` is present in the request
//! - `Content-Encoding` header is already set
//! - `Content-Range` header is present (range responses)
//! - `Content-Type` starts with `image/` (except `image/svg+xml`)
//! - `Content-Type` starts with `application/grpc` (except `application/grpc-web`)
//! - `Content-Length` is below the minimum size threshold (default: 860 bytes)
//!
//! The middleware will **always flush** after each chunk when:
//! - `X-Accel-Buffering: no` header is present
//! - `Content-Type` is `text/event-stream`
//! - `Content-Type` starts with `application/grpc-web`
//!
//! # Response Modifications
//!
//! When compression is applied:
//! - `Content-Encoding` header is set to the codec used
//! - `Content-Length` header is removed (compressed size is unknown)
//! - `Accept-Ranges` header is removed
//! - `Vary` header includes `Accept-Encoding`

#![deny(missing_docs)]

#[cfg(not(any(
    feature = "zstd",
    feature = "brotli",
    feature = "gzip",
    feature = "deflate"
)))]
compile_error!("At least one compression codec feature must be enabled");

mod body;
mod codec;
mod future;
mod layer;
mod service;

pub use body::CompressionBody;
pub use future::ResponseFuture;
pub use layer::CompressionLayer;
pub use service::CompressionService;
