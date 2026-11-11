"""Regression checks for the production-module graph and its reviewed edge identities."""
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest


class LayeringTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.tool = self.root / "tools/layering.py"
        self.tool.parent.mkdir()
        shutil.copyfile(Path(__file__).with_name("layering.py"), self.tool)
        shutil.copyfile(Path(__file__).with_name("rust_source.py"), self.tool.with_name("rust_source.py"))
        self.source = "crates/oblivion_sim/src/gamebryo/low.rs"
        self.write("crates/oblivion_core/src/lib.rs", "")
        self.write("crates/oblivion_pathing/src/lib.rs", "")
        self.write("crates/oblivion_sim/src/lib.rs", "pub mod gamebryo; pub mod tes;")
        self.write("crates/oblivion_sim/src/gamebryo/mod.rs", "pub mod low;")
        self.write("crates/oblivion_sim/src/tes/mod.rs", """
            pub mod actors;
            pub use actors::{first as old_first, second as old_second};
            pub use actors::second::Value as PublicValue;
        """)
        self.write("crates/oblivion_sim/src/tes/actors/mod.rs", "pub mod first; pub mod second;")
        for module in ("first", "second"):
            self.write(f"crates/oblivion_sim/src/tes/actors/{module}.rs", "pub struct Value;")

    def write(self, path, source):
        path = self.root / path
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(source)

    def graph(self, source):
        self.write(self.source, source)
        raw = subprocess.check_output([sys.executable, self.tool, "graph", "--upward"], text=True)
        return json.loads(raw)

    def test_aliases_braces_inline_modules_and_qualified_paths(self):
        graph = self.graph("""
            use crate::{tes::{self as facade, old_first::{Value as FromFirst}}};
            mod nested {
                fn read() { let _ = super::facade::PublicValue; }
            }
            fn other() { use crate::tes as local; let _ = local::old_first::Value; }
        """)
        self.assertEqual(graph[self.source], [
            "oblivion_sim::tes::actors::first", "oblivion_sim::tes::actors::second",
        ])

    def test_cfg_excludes_only_the_attributed_item(self):
        graph = self.graph('''
            #[cfg(test)] #[path = "absent.rs"] mod nonstandard_test_name;
            #[cfg(all(unix, test))] pub(crate) use crate::{tes::{old_first}, tes::old_first::Value};
            #[cfg(test)] pub fn helper() { let _ = crate::tes::old_first::Value; }
            /* /* nested */ use crate::tes::old_first; */
            const TEXT: &str = r##"crate::tes::old_first::Value"##;
            #[cfg(not(test))] fn production() { let _ = crate::tes::old_second::Value; }
        ''')
        self.assertEqual(graph[self.source], ["oblivion_sim::tes::actors::second"])

    def test_glob_through_reexports(self):
        self.write("crates/oblivion_sim/src/tes/mod.rs", """
            mod actors;
            mod facade { pub use super::actors::second::Value; }
            pub use facade::*;
        """)
        graph = self.graph("use crate::tes::*; fn read() { let _ = Value; }")
        self.assertEqual(graph[self.source], ["oblivion_sim::tes::actors::second"])

    def test_folders_inside_tes_share_a_rank_and_winmain_sits_above_them(self):
        self.write(self.source, "")
        self.write("crates/oblivion_sim/src/tes/mod.rs", "pub mod actors; pub mod menus; pub mod winmain;")
        self.write("crates/oblivion_sim/src/tes/menus/mod.rs", "pub mod stats; pub struct Shown;")
        self.write("crates/oblivion_sim/src/tes/winmain/mod.rs", "pub struct Driver;")
        self.write("crates/oblivion_sim/src/tes/menus/stats.rs", "use crate::tes::winmain::Driver;")
        actors = "crates/oblivion_sim/src/tes/actors/first.rs"
        self.write(actors, "pub struct Value; use crate::tes::menus::Shown;")
        raw = subprocess.check_output([sys.executable, self.tool, "graph", "--upward"], text=True)
        graph = json.loads(raw)
        self.assertNotIn(actors, graph)  # tes/actors -> tes/menus climbs no rank
        self.assertEqual(graph["crates/oblivion_sim/src/tes/menus/stats.rs"], ["oblivion_sim::tes::winmain"])

    def test_substitution_within_one_layer_pair_fails(self):
        baseline = self.graph("use crate::tes::old_first::Value;")
        self.write("tools/layering_baseline.json", json.dumps(baseline))
        subprocess.run([sys.executable, self.tool, "check"], check=True, capture_output=True)
        self.write(self.source, "\nuse crate::tes::old_second::Value;")
        check = subprocess.run([sys.executable, self.tool, "check"], capture_output=True, text=True)
        self.assertEqual(check.returncode, 1)
        self.assertIn(self.source + ":2: new upward dependency on oblivion_sim::tes::actors::second", check.stdout)
        self.assertIn("1 added, 1 removed", check.stdout)


if __name__ == "__main__":
    unittest.main()
