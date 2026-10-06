<!-- Added by HyperSpaces (2026): Require attribution for all future fork edits. Original: durable-streams 0.1.5, Apache-2.0. -->
# Fork attribution rules

This fork is based on the imported Durable Streams 0.1.5 release at
`d286a4b2ebfd040908ae9f07849582707774a125`. Keep upstream attribution permanently.

- Keep the received `LICENSE` text and all upstream copyright, patent,
  trademark and attribution notices intact. Do not remove upstream attribution.
- Every file changed from that import except `LICENSE` must carry a prominent
  first-line notice:
  `Modified by HyperSpaces (YEAR): <one-line summary>. Original: durable-streams 0.1.5, Apache-2.0.`
  Use `Added by HyperSpaces` for files absent from the import. Update the year
  and summary when editing again; retain the original provenance.
- Use the file's comment syntax: `//` for Rust, JavaScript and TypeScript, `#` for Python, shell, TOML, YAML and
  Docker inputs, and `<!-- ... -->` for Markdown. Put the notice immediately
  after a necessary shebang or Docker syntax directive. Plain-text `NOTICE` uses
  a notice preamble. Keep `LICENSE` exactly as published, without adding a
  notice or other text inside it; its attribution belongs in `NOTICE` and
  `UPSTREAM.md`. The notice gate exempts `LICENSE`, but the inventory includes it.
  Reapply the notice if Cargo regenerates `Cargo.lock`.
- For JSON and any format without comments, preserve its data format and use
  an adjacent `<filename>.NOTICE` rather than injecting comments or metadata.
  Its first line is the sidecar's own Added/Modified notice; its second line is
  the source file's Added/Modified notice, naming that source file in the summary.
  Inventory both files. CI also parses changed JSON to reject invalid payloads.
  Extend the gate with the proper comment syntax for a new commentable format.
- Every change updates the file inventory in `UPSTREAM.md`, including these
  rules, CI, generated lock files and compliance scripts. List each added or
  modified path once, with its change kind and a one-line description. Remove
  an entry when a file is deleted or restored exactly to the import.
- If upstream adds a `NOTICE` file, carry it over intact. Include its relevant
  notices in source and image distributions alongside `LICENSE` and update the
  Docker recipe and inventory in the same change. Do not imply that attribution
  notices change the Apache license terms.
- Runtime images must include the full license and `UPSTREAM.md` provenance
  and `NOTICE` under `/usr/share/doc/durable-streams/`. Coordinate every new deployment image
  digest with consumers that pin it; never replace an immutable pin silently.

After committing, run `python3 scripts/check_attribution.py` and
`python3 scripts/check_attribution_test.py`. CI checks the committed tree against
that fixed import, with full Git history, before tests or image publication.
