"""Regression cases for source versus executable attribution in the port ledger."""

from pathlib import Path
from tempfile import TemporaryDirectory
import unittest

import generate as ledger


class SiteLedgerTests(unittest.TestCase):
    def setUp(self):
        self.functions = ledger.Functions({
            "00401000": ("Actor_Update", 12),
            "00402000": ("Scene_Draw", 12),
        }, {
            "00401000": [(0x401000, 0x401008), (0x403000, 0x403004)],
            "00402000": [(0x402000, 0x402008), (0x403000, 0x403004)],
        })

    def scan(self, text):
        with TemporaryDirectory() as folder:
            path = Path(folder) / "sample.rs"
            path.write_text(text)
            return ledger.scan(ledger.audit.Source(path, "sample.rs"), self.functions)[0]

    def test_shared_tail_and_gap(self):
        self.assertEqual(self.functions.owners("00403001"), ["00401000", "00402000"])
        self.assertEqual(self.functions.owners("00401009"), [])
        self.assertEqual(self.functions.bytes(["00401000", "00401000", "00402000"]), 20)

    def test_definition_and_secondary_address_across_attribute(self):
        rows = self.scan("""/// IDA: 00401000 Actor_Update: calls 00402000.
/// Completion: 100% of the reached arms.
#[expect(
    dead_code,
    reason = "address in a string 00402000"
)]
pub fn actor_update() {
    count_face(world, 0x0040_2000);
    let _ = GameError::unclassified(0, "unknown site");
    let _ = GameError::from_legacy_message(message);
}""")
        definition, = (r for r in rows if r.kind == "definition")
        self.assertEqual((definition.address, definition.assessment), ("00401000", "complete within stated scope"))
        citation, = (r for r in rows if r.kind == "citation")
        self.assertEqual(citation.address, "00402000")
        count, = (r for r in rows if r.kind == "counted call")
        self.assertEqual((count.address, count.enclosing_entry), ("00402000", "00401000"))
        errors = [r for r in rows if r.kind == "error unclassified"]
        self.assertEqual([(r.address, r.enclosing_entry) for r in errors], [("", "00401000")] * 2)

    def test_non_definitions_never_grant_credit(self):
        rows = self.scan("""/// IDA: N/A uses 00401000.
fn host() {}
/// IDA: 00401003..00401006
fn actor_update() {}
/// IDA: 00402000 Scene_Draw
fn unrelated() {}
/// IDA: 00B02010 some global
fn draw() {}
trait Service {
    /// IDA: 00402000 Scene_Draw
    fn draw(&self) {}
}""")
        self.assertFalse(any(r.kind == "definition" for r in rows))
        self.assertEqual({r.kind for r in rows}, {"host helper", "citation", "instruction site",
                                                   "ambiguous definition", "data reference", "trait contract"})

    def test_unassessed_partial_and_bare_percent(self):
        self.assertEqual(ledger.assessment("All reached cases passed", 0), "unassessed")
        self.assertEqual(ledger.assessment("Completion: 80% of the list.", 0), "partial")
        self.assertEqual(ledger.assessment("Completion: 100% of the arm. TODO: [ ] rest", 1), "partial")
        self.assertEqual(ledger.assessment("Completion: 100%.", 0), "100% annotation, scope unspecified")

    def test_counter_alternatives_and_loop(self):
        rows = self.scan("""fn run() {
    count_idle_call(world, 0x0040_1000);
    for address in [0x0040_1000, 0x0040_2000] {
        count(world, address);
    }
    count(world, if changed { 0x0040_1000 } else { 0x0040_2000 });
    values.iter().count();
}""")
        self.assertEqual([r.address for r in rows if r.kind == "counted call"],
                         ["00401000", "00401000", "00402000", "00401000", "00402000"])

    def test_array_signature_char_literal_and_trait_signature(self):
        rows = self.scan("""/// IDA: 00401000 Actor_Update
fn actor_update(data: [u8; 3]) -> [u8; 3] {
    let ch = '}';
    data
}
trait Service {
    /// IDA: 00402000 Scene_Draw
    fn draw(&self);
}""")
        self.assertEqual([(r.name, r.kind) for r in rows], [("actor_update", "definition"), ("draw", "trait contract")])

    def test_check_is_read_only_for_both_current_and_stale_output(self):
        with TemporaryDirectory() as folder:
            path = Path(folder)
            ledger.sync_reports(path, {"ledger.csv": "first"}, check=False)
            report = path / "ledger.csv"
            stamp = report.stat().st_mtime_ns
            ledger.sync_reports(path, {"ledger.csv": "first"}, check=True)
            with self.assertRaisesRegex(ValueError, "Out of date"):
                ledger.sync_reports(path, {"ledger.csv": "second"}, check=True)
            self.assertEqual((report.read_text(), report.stat().st_mtime_ns), ("first", stamp))

    def test_duplicate_review_rejects_new_and_stale(self):
        with TemporaryDirectory() as folder:
            path = Path(folder) / "allow.txt"
            path.write_text("00401000 documented reason\n")
            self.assertIn("00401000", ledger.reviewed_duplicates({"00401000": []}, path))
            for groups in ({}, {"00402000": []}):
                with self.assertRaises(ValueError):
                    ledger.reviewed_duplicates(groups, path)


if __name__ == "__main__":
    unittest.main()
