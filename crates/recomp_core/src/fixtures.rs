//! IDA: N/A test support: the rows and config imported by `tools/import_fixtures.py`.

use std::path::PathBuf;

/// IDA: N/A `<workspace>/fixtures`, or `OBLIVION_FIXTURES_ROOT`: `tools/side_by_side.py` points it at a copy with one row changed, to see which test notices.
#[must_use]
pub fn root() -> PathBuf {
    std::env::var_os("OBLIVION_FIXTURES_ROOT").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures"),
        PathBuf::from,
    )
}

/// IDA: N/A the original game data: `OBLIVION_DATA_DIR`, else the C++ repository's copy.
#[must_use]
pub fn data_dir() -> PathBuf {
    std::env::var_os("OBLIVION_DATA_DIR")
        .map_or_else(|| root().join("../../OblivionRecomp/Data"), PathBuf::from)
}

/// IDA: N/A the rows of `fixtures/rows/<name>.rows`, in file order.
#[must_use]
pub fn rows(name: &str) -> Vec<String> {
    ledger_line(&["LOAD", name]);
    let path = root().join("rows").join(format!("{name}.rows"));
    let bytes = std::fs::read(&path).unwrap_or_else(|error| panic!("{path:?}: {error}"));
    // Rows are Latin-1, as the importer wrote them.
    let text: String = bytes.iter().map(|&byte| char::from(byte)).collect();
    text.lines().map(str::to_owned).collect()
}

/// IDA: N/A one line of the ledger `tools/side_by_side.py` reads, when `OBLIVION_ROW_LEDGER` names a file: tab-separated, the test's name last. One append per line, so parallel tests don't tear it. With the variable unset this does nothing.
fn ledger_line(fields: &[&str]) {
    use std::io::Write;
    let Some(path) = std::env::var_os("OBLIVION_ROW_LEDGER") else {
        return;
    };
    let thread = std::thread::current();
    let mut line = fields.join("\t");
    line.push('\t');
    line.push_str(thread.name().unwrap_or("?"));
    line.push('\n');
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path);
    if let Ok(mut file) = file {
        let _ = file.write_all(line.as_bytes());
    }
}

/// IDA: N/A a row tracker's accounting, for the ledger: which lines of `fixtures/rows/<name>.rows` this test read, as ascending `first-last` ranges. A failed test reports nothing, so its rows don't count as read.
pub fn ledger_read(name: &str, read: &[bool]) {
    if std::thread::panicking() {
        return;
    }
    let mut ranges = Vec::new();
    let mut at = 0;
    while at < read.len() {
        if read[at] {
            let first = at;
            while at + 1 < read.len() && read[at + 1] {
                at += 1;
            }
            ranges.push(format!("{first}-{at}"));
        }
        at += 1;
    }
    ledger_line(&["READ", name, &read.len().to_string(), &ranges.join(",")]);
}

/// IDA: N/A a comparison set something aside, for the ledger: `what` of `fixtures/rows/<name>.rows`, how many, and why. An empty reason is a relaxation nobody recorded, and the tool lists it.
pub fn ledger_relaxed(name: &str, what: &str, count: usize, reason: &str) {
    ledger_line(&["RELAXED", name, what, &count.to_string(), reason]);
}

/// IDA: N/A a deferred group of a row tracker, for the ledger: `what`, how many, and the slice that reads them.
pub fn ledger_deferred(name: &str, what: &str, count: usize, slice: &str) {
    ledger_line(&["DEFERRED", name, what, &count.to_string(), slice]);
}
