# http-response-compression

A Tower middleware layer for compressing HTTP response bodies.

## Features

- Supports Gzip, Brotli, and Zstd compression
- Automatic codec selection based on `Accept-Encoding` header
- Configurable minimum body size threshold (default: 860 bytes)
- Streaming support with flush control for SSE and gRPC-web
- Preserves trailers through compression

## Usage

```rust
use http_response_compression::CompressionLayer;
use tower::ServiceBuilder;

let service = ServiceBuilder::new()
    .layer(CompressionLayer::new())
    .service(my_service);
```

With custom minimum size:

```rust
let service = ServiceBuilder::new()
    .layer(CompressionLayer::new().min_size(1024))
    .service(my_service);
```

## Compression Rules

The middleware will **not** compress responses when:

- No supported `Accept-Encoding` is present in the request
- `Content-Encoding` header is already set
- `Content-Range` header is present (range responses)
- `Content-Type` is `image/*` (except `image/svg+xml`)
- `Content-Type` is `application/grpc` (except `application/grpc-web`)
- `Content-Length` is below the minimum size threshold

The middleware will **always flush** after each chunk when:

- `X-Accel-Buffering: no` header is present
- `Content-Type` is `text/event-stream`
- `Content-Type` is `application/grpc-web`

## Response Modifications

When compression is applied:

- `Content-Encoding` header is set to the codec used
- `Content-Length` header is removed (compressed size is unknown)
- `Accept-Ranges` header is removed
- `Vary` header includes `Accept-Encoding`

## License

MIT
