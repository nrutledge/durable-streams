<!-- Added by HyperSpaces (2026): Require attribution for all future fork edits. Original: durable-streams 0.1.5, Apache-2.0. -->
# Fork attribution rules

This fork is based on the imported Durable Streams 0.1.5 release at
`d286a4b2ebfd040908ae9f07849582707774a125`. Keep upstream attribution permanently.

- Keep the complete `LICENSE` text and all upstream copyright, patent,
  trademark and attribution notices intact. Do not remove upstream attribution.
- Every file changed from that import must carry a prominent first-line notice:
  `Modified by HyperSpaces (YEAR): <one-line summary>. Original: durable-streams 0.1.5, Apache-2.0.`
  Use `Added by HyperSpaces` for files absent from the import. Update the year
  and summary when editing again; retain the original provenance.
- Use the file's comment syntax: `//` for Rust, `#` for Python, TOML, YAML and
  Docker inputs, and `<!-- ... -->` for Markdown. Put the notice immediately
  after a necessary shebang or Docker syntax directive. Plain-text `LICENSE`
  and `NOTICE` use a separate notice preamble, leaving their original text intact.
  Reapply the notice if Cargo regenerates `Cargo.lock`.
- Every change updates the file inventory in `UPSTREAM.md`, including these
  rules, CI, generated lock files and compliance scripts. List each added or
  modified path once, with its change kind and a one-line description. Remove
  an entry when a file is deleted or restored exactly to the import.
- If upstream adds a `NOTICE` file, carry it over intact. Include its relevant
  notices in source and image distributions alongside `LICENSE` and update the
  Docker recipe and inventory in the same change. Do not imply that attribution
  notices change the Apache license terms.
- Runtime images must include the full license and `UPSTREAM.md` provenance
  under `/usr/share/doc/durable-streams/`. Coordinate every new deployment image
  digest with consumers that pin it; never replace an immutable pin silently.

After committing, run `python3 scripts/check_attribution.py` and
`python3 scripts/check_attribution_test.py`. CI checks the committed tree against
that fixed import, with full Git history, before tests or image publication.
