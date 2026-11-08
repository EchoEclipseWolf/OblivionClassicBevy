# Original startup scaffold

`manifest.json` holds 47 bodies, 1,224 callsites and 1,605 blocks from the read-only IDA capture dated 2026-09-24, copied
from the C++ repository unchanged. It is a bounded expansion, not the transitive closure of startup. The 123 indirect
sites without one resolved reference stay unknown.

`dispositions.json` is copied from the C++ repository too, and **records the C++ port**. `implemented_behavior` there means
the C++ port has that body. It does not mean this repository does. What Rust has so far is
`crates/recomp_startup/src/scaffold.rs`'s `RUST_PORTED_THROUGH`, and the milestone table in `docs/migration-plan.md`.

```sh
python3 tools/startup_roadmap/generate.py
cargo fmt --all
```

This writes `crates/recomp_startup/src/scaffold_data.rs` and `summary.json`. The output is inert metadata: no original
address can be called, and a stub never reports success. Follow block successor edges; never run the address-sorted call
array as a job list.

`Startup-Call-Ledger.md` has one writer, this repository since the cutover on 2026-11-06. The generator writes the ledger
with `--notes-at-cutover <notes folder>`. Without it, the notes aren't touched.

`normalize.py` rebuilds the manifest from the preserved raw captures in the notes, if that is ever needed.
