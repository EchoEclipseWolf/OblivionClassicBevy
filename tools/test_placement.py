"""Regression checks for the one-port-per-function gate and the owner classes' home table."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class PlacementTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.tool = self.root / "tools/placement.py"
        self.tool.parent.mkdir()
        for name in ("placement.py", "rust_source.py"):
            shutil.copyfile(Path(__file__).with_name(name), self.tool.with_name(name))
        self.write("tools/port_ledger/ida_functions.csv", """entry,name,size
00401000,Actor_GetHeight,32
00402000,TESPackage_IsCombatType,16
00403000,sub_403000,16
00404000,NiNode_DetachChild,64
00405000,HUDMainMenu_Update,16
""")
        self.write("tools/port_ledger/ida_split_chunks.csv", "entry,start,end_exclusive\n00404000,00409000,00409010\n")
        self.actor = "crates/oblivion_sim/src/tes/actors/actor.rs"
        self.package = "crates/oblivion_sim/src/tes/ai/ai_package.rs"
        self.write(self.actor, """
            // IDA: 00401000 Actor_GetHeight(actor)
            pub fn actor_height() -> u32 { 1 }
        """)
        self.write(self.package, """
            // IDA: 00402000 TESPackage_IsCombatType(package)
            pub fn is_combat_type() -> bool { false }
        """)
        self.baseline({"ports": {}, "bodies": []})
        self.rules = [
            {"classes": "^Actor$", "homes": ["crates/oblivion_sim/src/tes/actors/"], "why": "actors"},
            {"classes": "^TESPackage$", "homes": ["crates/oblivion_sim/src/tes/ai/"], "why": "packages"},
            {"classes": "^NiNode$", "homes": ["*"], "why": "anywhere, for the chunk test"},
            {"classes": "^(?!Menu$)\\w+Menu$", "homes": ["crates/oblivion_sim/src/tes/menus/{snake}.rs"], "why": "menus"},
        ]
        self.homes({})

    def write(self, path, source):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source)

    def baseline(self, value):
        self.write("tools/placement_baseline.json", json.dumps(value))

    def homes(self, exceptions):
        self.write("tools/placement_homes.json", json.dumps({"rules": self.rules, "exceptions": exceptions}))

    def run_tool(self, *arguments):
        return subprocess.run([sys.executable, str(self.tool), *arguments], capture_output=True, text=True)

    def test_the_baseline_passes(self):
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("0 new, 0 stale", result.stdout)

    def test_a_function_outside_its_class_home_fails(self):
        self.write(self.actor, """
            // IDA: 00401000 Actor_GetHeight(actor)
            pub fn actor_height() -> u32 { 1 }
            /// IDA: 00402008, the type test inside TESPackage_IsCombatType.
            fn package_type() -> u32 { 2 }
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("package_type: TESPackage_IsCombatType belongs to TESPackage, whose home is "
                      "crates/oblivion_sim/src/tes/ai/", result.stdout)
        self.assertIn("is already ported at crates/oblivion_sim/src/tes/ai/ai_package.rs:3: is_combat_type", result.stdout)

    def test_a_file_that_holds_a_class_cannot_gain_more_of_it_outside_the_home(self):
        menu = "crates/oblivion_sim/src/tes/menus/stats_menu.rs"
        first = """
            // IDA: 00401000 Actor_GetHeight, the first misfile.
            fn height() -> u32 { 1 }
        """
        self.write(menu, first)
        self.baseline({"ports": {"00401000": {"ida": "Actor_GetHeight", "reason": "forwarder",
                                              "members": [f"{self.actor}::actor_height", f"{menu}::height"]}},
                       "bodies": []})
        self.homes({f"{menu}::height": "reviewed: the menu's forwarder"})
        self.assertEqual(self.run_tool("check").returncode, 0)
        self.write(menu, first + """
            // IDA: 00401010, the clamp inside Actor_GetHeight.
            fn clamp_height() -> u32 { 2 }
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("stats_menu.rs:6: clamp_height: Actor_GetHeight belongs to Actor", result.stdout)

    def test_a_home_exception_that_is_gone_is_stale(self):
        self.homes({f"{self.package}::no_such_function": "reviewed once"})
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 0)
        self.assertIn("stale entry: home exception", result.stdout)

    def test_a_menu_is_filed_in_the_file_named_for_it(self):
        self.write("crates/oblivion_sim/src/tes/menus/hud_main_menu.rs", """
            // IDA: 00405000 HUDMainMenu_Update(menu)
            fn update() {}
        """)
        self.assertEqual(self.run_tool("check").returncode, 0)
        self.write("crates/oblivion_sim/src/tes/menus/mod.rs", """
            // IDA: 00405004, the fade inside HUDMainMenu_Update.
            fn fade() {}
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("whose home is crates/oblivion_sim/src/tes/menus/hud_main_menu.rs", result.stdout)

    def test_a_test_only_import_does_not_hide_the_next_function(self):
        self.write(self.actor, """
            #[cfg(test)]
            use std::fmt::Write;
            // IDA: 00402000 TESPackage_IsCombatType(package), misfiled
            fn package_type() -> u32 { 2 }
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("package_type: TESPackage_IsCombatType belongs to TESPackage", result.stdout)

    def test_a_reviewed_second_port_passes_and_a_third_fails(self):
        copy = "crates/oblivion_sim/src/tes/menus/stats_menu.rs"
        self.write(copy, """
            impl StatsMenu {
                // IDA: 00401000 Actor_GetHeight, the menu's forwarder.
                fn height(&self) -> u32 { 1 }
            }
        """)
        members = [f"{self.actor}::actor_height", f"{copy}::StatsMenu::height"]
        self.baseline({"ports": {"00401000": {"ida": "Actor_GetHeight", "members": members, "reason": "forwarder"}},
                       "bodies": []})
        self.homes({f"{copy}::StatsMenu::height": "method of StatsMenu: the menu's forwarder"})
        self.assertEqual(self.run_tool("check").returncode, 0)
        self.write(self.package, """
            // IDA: 00402000 TESPackage_IsCombatType(package)
            pub fn is_combat_type() -> bool { false }
            // IDA: 00401000 Actor_GetHeight
            fn local_height() -> u32 { 1 }
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ai_package.rs:5: local_height: 00401000 Actor_GetHeight is already ported at", result.stdout)

    def test_the_heading_name_serves_an_unnamed_export_and_tests_are_skipped(self):
        self.write(self.actor, """
            // IDA: 00401000 Actor_GetHeight(actor)
            pub fn actor_height() -> u32 { 1 }
            // IDA: 00403000 BoundItemEffect_Equip(effect)
            fn equip() {}
            #[cfg(test)]
            mod tests {
                // IDA: 00402000 TESPackage_IsCombatType
                fn fixture() {}
                #[test]
                fn replays() {}
            }
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("owner class BoundItemEffect (BoundItemEffect_Equip) has no home", result.stdout)
        self.assertNotIn("TESPackage", result.stdout.split("\n")[0])
        self.assertIn("1 new", result.stdout)

    def test_a_heading_above_a_multi_line_attribute_is_read(self):
        self.write(self.package, """
            // IDA: 00402000 TESPackage_IsCombatType(package)
            pub fn is_combat_type() -> bool { false }
            // IDA: 00401000 Actor_GetHeight
            #[allow(
                clippy::neg_cmp_op_on_partial_ord,
                reason = "the x87 compare"
            )]
            fn local_height() -> u32 { 1 }
            let_values![
                1
            ]
            fn unheaded() {}
        """)
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("ai_package.rs:9: local_height: 00401000 Actor_GetHeight is already ported at", result.stdout)
        self.assertNotIn("unheaded", result.stdout)

    def test_identical_bodies_fail_until_reviewed(self):
        body = "{ let mut total = 0u32; for i in 0..8u32 { total = total.wrapping_mul(31).wrapping_add(i); } " \
               "if total > 9 { total -= 9; } total }"
        self.write(self.actor, f"// IDA: N/A a host helper.\nfn mix() -> u32 {body}\n")
        self.write(self.package, f"// IDA: N/A a host helper.\nfn mix_again() -> u32 {body}\n")
        self.baseline({"ports": {}, "bodies": []})
        result = self.run_tool("check")
        self.assertEqual(result.returncode, 1)
        self.assertIn("identical bodies:", result.stdout)
        self.baseline({"ports": {}, "bodies": [[f"{self.actor}::mix", f"{self.package}::mix_again"]]})
        self.assertEqual(self.run_tool("check").returncode, 0)

    def test_where_finds_a_port_by_address_inside_a_chunk_and_by_name(self):
        self.write(self.actor, """
            // IDA: 00409004, NiNode_DetachChild's tail chunk.
            fn detach_tail() {}
        """)
        by_address = self.run_tool("where", "00404010")
        self.assertIn("00404000 NiNode_DetachChild", by_address.stdout)
        self.assertIn("actor.rs:3: detach_tail", by_address.stdout)
        by_name = self.run_tool("where", "IsCombatType")
        self.assertIn("ai_package.rs:3: is_combat_type", by_name.stdout)
        missing = self.run_tool("where", "00403000")
        self.assertIn("no production function is headed by it", missing.stdout)
        home = self.run_tool("where", "Actor_GetEyeHeight")
        self.assertIn("Actor's home: crates/oblivion_sim/src/tes/actors/", home.stdout)
        unknown = self.run_tool("where", "BoundItemEffect_Equip")
        self.assertIn("BoundItemEffect has no home rule yet", unknown.stdout)

    def test_owed_lists_only_the_entries_marked_owed(self):
        self.homes({"a.rs::x": "OWED: moves with its merge", "b.rs::q": "method of a type that stays"})
        self.baseline({"bodies": [], "ports": {
            "00401000": {"ida": "Actor_GetHeight", "members": ["a.rs::x", "b.rs::y"], "reason": "OWED: merge"},
            "00402000": {"ida": "TESPackage_IsCombatType", "members": ["a.rs::p", "b.rs::q"], "reason": "forwarder"},
        }})
        result = self.run_tool("report", "--owed")
        self.assertIn("00401000 Actor_GetHeight: OWED: merge", result.stdout)
        self.assertNotIn("00402000", result.stdout)
        self.assertIn("1 of 2 recorded cross-file ports are owed a fix", result.stdout)
        self.assertIn("a.rs::x: OWED: moves with its merge", result.stdout)
        self.assertIn("1 of 2 functions recorded outside their class's home are owed a move", result.stdout)

    def test_prune_drops_only_what_is_gone(self):
        self.baseline({"ports": {"00401000": {"ida": "Actor_GetHeight", "members": ["a.rs::x", "b.rs::y"],
                                              "reason": "old"}},
                       "bodies": [["a.rs::x", "b.rs::y"]]})
        stale = self.run_tool("check")
        self.assertIn("stale entry: 00401000: a.rs::x is gone", stale.stdout)
        self.assertEqual(self.run_tool("prune").returncode, 0)
        pruned = json.loads((self.root / "tools/placement_baseline.json").read_text())
        self.assertEqual(pruned, {"ports": {}, "bodies": []})


if __name__ == "__main__":
    unittest.main()
