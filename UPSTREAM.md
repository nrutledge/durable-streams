<!-- Modified by HyperSpaces (2026): Record source provenance and the complete fork change inventory. Original: durable-streams 0.1.5, Apache-2.0. -->
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

## Apache-2.0 attribution

The complete Apache-2.0 license is in [LICENSE](LICENSE). The imported crate did
not include that file; the fork supplies it exactly as published, with no
addition notice or other text inside the legal document. Fork attribution
is in [NOTICE](NOTICE). Existing upstream source and documentation attribution is
retained. The imported source contains no `NOTICE` file; our added notice
records fork attribution. If upstream adds one, retain its relevant notices
in all distributions too. Runtime images package `LICENSE`, `NOTICE` and this
provenance under `/usr/share/doc/durable-streams/`.

Every added or modified file relative to import
`d286a4b2ebfd040908ae9f07849582707774a125` is listed below. Each except the unmodified legal document `LICENSE` also
bears a first-line change notice (after a required shebang or Docker syntax directive).
The inventory includes generated files and checks itself; deleted files are
not distributed and are removed from the list. CI rejects missing notices,
missing or incorrect entries, and stale entries before image publication.
[AGENTS.md](AGENTS.md) owns the rules for future edits.

### Fork file inventory

| File | Change | Description |
| --- | --- | --- |
| `.dockerignore` | Added | Exclude Git metadata and build output from the image context. |
| `.github/workflows/image.yml` | Added | Test the fork, enforce attribution, and publish immutable GHCR images. |
| `AGENTS.md` | Added | Require notices, inventory updates and preserved upstream attribution. |
| `CHANGELOG.md` | Modified | Record the native h2 fork and preserved storage behavior. |
| `Cargo.lock` | Modified | Lock the h2 transport dependencies. |
| `Cargo.toml` | Modified | Add the h2 and HTTP transport dependencies. |
| `Dockerfile` | Modified | Build the fork with both listeners and package its license, notices and provenance. |
| `LICENSE` | Added | Distribute the complete original Apache-2.0 license text unchanged; attribution is in NOTICE. |
| `NOTICE` | Added | Record fork changes and retain upstream attribution separately from the license. |
| `README.md` | Modified | Document the maintained fork while retaining the upstream README. |
| `UPSTREAM.md` | Modified | Record source provenance and the complete fork change inventory. |
| `scripts/check_attribution.py` | Added | Check notices and the inventory against the fixed imported Git tree. |
| `scripts/check_attribution_test.py` | Added | Verify the attribution gate using real Git additions, edits and deletions. |
| `src/api.rs` | Modified | Allow streaming sources to propagate file-read errors. |
| `src/engine_h2.rs` | Added | Serve bounded read-only h2c through the existing handlers and Store. |
| `src/engine_raw.rs` | Modified | Abort HTTP/1 streaming bodies on source errors. |
| `src/handlers.rs` | Modified | Preserve SSE file errors and verify damaged-history behavior. |
| `src/main.rs` | Modified | Wire the optional h2 listener and coordinated shutdown drain. |
| `src/sse_reactor.rs` | Modified | Abort Linux reactor SSE responses on file-read failures. |
