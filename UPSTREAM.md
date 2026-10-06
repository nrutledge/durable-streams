# Source provenance

This public HyperSpaces fork starts from the official `durable-streams` 0.1.5
crate, published by Electric. It is **not** the unrelated
`durable-streams-server` crate.

- Archive: https://static.crates.io/crates/durable-streams/durable-streams-0.1.5.crate
- SHA-256: `472721e61ca191520c5e2b9bf8859aa8f9aa85599027f9103addbf695724b8ad`
- Upstream: https://github.com/electric-sql/electric
- VCS base: `88793e76595d69be300731b9b25c58538923a53b`
- Path: `packages/durable-streams-rust`

The published crate records a dirty VCS tree: the archive hash, not the VCS
commit alone, identifies the release. `Cargo.toml.orig` is restored as the
editable manifest. The initial commit preserves the published source and lock.

Our transport additions share the existing handlers and one Store. No data
format, WAL, offset or write admission changes are made. HTTP/1 stays on 4437;
the private read-only prior-knowledge HTTP/2 listener is enabled with
`--h2-port 4438`. HyperSpaces pins the CI-published image by digest.

On shutdown the h2 listener stops accepting and sends GOAWAY. Existing
responses share the HTTP/1 listener's 25-second drain deadline before remaining
readers are aborted; committers stop after both drains. File-read failures abort
the affected h2 response or h1 SSE body, including Linux reactor reads, rather
than emitting a successful end. The real transport regression covers these
paths with flow-controlled catch-up, an idle reader and a truncated root file.
