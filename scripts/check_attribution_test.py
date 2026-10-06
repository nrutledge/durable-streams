# Added by HyperSpaces (2026): Exercise the attribution gate against real Git histories. Original: durable-streams 0.1.5, Apache-2.0.
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

CHECKER = Path(__file__).with_name("check_attribution.py").resolve()


class AttributionTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.repo = Path(self.temp.name)
        self.git("init", "-q")
        self.write("Cargo.toml", '# Upstream copyright remains here.\n[package]\nname = "durable-streams"\n')
        self.write("UPSTREAM.md", "# Original source provenance\n")
        self.write("package.json", '{"name":"durable-streams","version":"0.1.5"}\n')
        self.commit()
        self.base = self.git("rev-parse", "HEAD").strip()
        self.write("LICENSE", CHECKER.parents[1].joinpath("LICENSE").read_text())
        self.write("Cargo.toml", '# Modified by HyperSpaces (2026): Add h2 dependencies. Original: durable-streams 0.1.5, Apache-2.0.\n# Upstream copyright remains here.\n[package]\nname = "durable-streams"\n')
        self.write("src/engine_h2.rs", '// Added by HyperSpaces (2026): Add the h2 transport. Original: durable-streams 0.1.5, Apache-2.0.\n')
        self.write("UPSTREAM.md", '<!-- Modified by HyperSpaces (2026): Track fork changes. Original: durable-streams 0.1.5, Apache-2.0. -->\n# Original source provenance\n\n| File | Change | Description |\n| --- | --- | --- |\n| `Cargo.toml` | Modified | Add h2 dependencies. |\n| `LICENSE` | Added | Preserve the received license exactly. |\n| `UPSTREAM.md` | Modified | Track fork changes. |\n| `src/engine_h2.rs` | Added | Add the h2 transport. |\n')

    def git(self, *args):
        return subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True,
                              text=True, env={**os.environ, "GIT_AUTHOR_NAME": "Test",
                              "GIT_AUTHOR_EMAIL": "test@example.invalid", "GIT_COMMITTER_NAME": "Test",
                              "GIT_COMMITTER_EMAIL": "test@example.invalid"}).stdout

    def write(self, path, content):
        target = self.repo / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(content)

    def commit(self):
        self.git("add", ".")
        self.git("commit", "-qm", "Fixture change")

    def run_check(self):
        self.commit()
        return subprocess.run(["python3", str(CHECKER), "--repo", str(self.repo), "--base", self.base],
                              capture_output=True, text=True)

    def test_complete_inventory_and_notices_pass(self):
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("4 changed files", result.stdout)

    def test_license_keeps_exact_text_without_a_notice(self):
        license_text = CHECKER.parents[1].joinpath("LICENSE").read_text()
        self.assertTrue(license_text.lstrip().startswith("Apache License"))
        self.write("LICENSE", license_text)
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.repo / "LICENSE").read_text(), license_text)

    def test_altered_license_body_fails(self):
        p = self.repo / "LICENSE"
        p.write_text(p.read_text().split("   9. Accepting Warranty", 1)[0])
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("LICENSE: must match the exact received", result.stderr)

    def test_javascript_typescript_and_shebang_notices_pass(self):
        files = {
            "npm/assemble.mjs": "// Added by HyperSpaces (2026): Assemble fork packages. Original: durable-streams 0.1.5, Apache-2.0.\nconsole.log('durable-streams');\n",
            "npm/bin/launcher.cjs": "#!/usr/bin/env node\n// Added by HyperSpaces (2026): Launch fork packages. Original: durable-streams 0.1.5, Apache-2.0.\nconsole.log('durable-streams');\n",
            "conformance/conformance.test.ts": "// Added by HyperSpaces (2026): Cover fork conformance. Original: durable-streams 0.1.5, Apache-2.0.\nconst version: string = '0.1.5';\n",
            "bench.sh": "#!/bin/sh\n# Added by HyperSpaces (2026): Benchmark fork reads. Original: durable-streams 0.1.5, Apache-2.0.\nexit 0\n",
        }
        p = self.repo / "UPSTREAM.md"
        for path, content in files.items():
            self.write(path, content)
            p.write_text(p.read_text() + f'| `{path}` | Added | Exercise the format notice. |\n')
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        for path in ("npm/assemble.mjs", "npm/bin/launcher.cjs"):
            parsed = subprocess.run(["node", "--check", str(self.repo / path)], capture_output=True, text=True)
            self.assertEqual(parsed.returncode, 0, parsed.stderr)

    def add_json_change(self, with_notice=True):
        content = '{"name":"durable-streams","version":"0.1.6"}\n'
        self.write("package.json", content)
        p = self.repo / "UPSTREAM.md"
        p.write_text(p.read_text() + '| `package.json` | Modified | Update the fork version. |\n')
        if with_notice:
            self.write("package.json.NOTICE", "Added by HyperSpaces (2026): Carry the package.json change notice. Original: durable-streams 0.1.5, Apache-2.0.\nModified by HyperSpaces (2026): Update package.json version for the fork. Original: durable-streams 0.1.5, Apache-2.0.\n")
            p.write_text(p.read_text() + '| `package.json.NOTICE` | Added | Carry the JSON change notice. |\n')
        return content

    def test_json_uses_adjacent_notice_without_inline_metadata(self):
        expected = self.add_json_change()
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual((self.repo / "package.json").read_text(), expected)

    def test_json_without_adjacent_notice_fails(self):
        self.add_json_change(with_notice=False)
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("package.json: requires adjacent package.json.NOTICE", result.stderr)

    def test_json_with_invalid_payload_fails_even_with_notice(self):
        self.add_json_change()
        self.write("package.json", "# Not a valid JSON comment.\n{}\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("package.json: invalid source", result.stderr)

    def test_added_file_without_notice_fails(self):
        self.write("src/engine_h2.rs", "// Native h2 transport.\n")
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("src/engine_h2.rs: prominent Added notice", result.stderr)

    def test_missing_inventory_entry_fails(self):
        p = self.repo / "UPSTREAM.md"
        p.write_text(p.read_text().replace('| `src/engine_h2.rs` | Added | Add the h2 transport. |\n', ''))
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("src/engine_h2.rs: missing or incorrect Added entry", result.stderr)

    def test_wrong_kind_or_buried_notice_fails(self):
        for prefix in ("# Added", "# Old header.\n# Modified"):
            with self.subTest(prefix=prefix):
                self.write("Cargo.toml", prefix + ' by HyperSpaces (2026): Add h2 dependencies. Original: durable-streams 0.1.5, Apache-2.0.\n')
                result = self.run_check()
                self.assertEqual(result.returncode, 1)
                self.assertIn("Cargo.toml: prominent Modified notice", result.stderr)

    def test_deleted_file_is_removed_from_inventory(self):
        self.commit()
        (self.repo / "src/engine_h2.rs").unlink()
        result = self.run_check()
        self.assertEqual(result.returncode, 1)
        self.assertIn("src/engine_h2.rs: stale UPSTREAM.md entry", result.stderr)
        p = self.repo / "UPSTREAM.md"
        p.write_text(p.read_text().replace('| `src/engine_h2.rs` | Added | Add the h2 transport. |\n', ''))
        result = self.run_check()
        self.assertEqual(result.returncode, 0, result.stderr)


if __name__ == "__main__":
    unittest.main()
