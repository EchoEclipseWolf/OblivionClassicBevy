//! IDA: N/A the accounting of a `.rows` file. Every line is either read by a check or named in the caller's deferred table with the slice that will read it. A later slice removes its lines from the table; when the table is empty, every line has been read.

use std::cell::RefCell;

use oblivion_core::fixtures;

/// IDA: N/A a deferred group of rows: the row kind, the field after it (a form type or a list offset, or "" for none), and the slice that reads them.
pub type Deferred = (&'static str, &'static [&'static str], &'static str);

pub struct RowBook {
    pub name: String,
    pub lines: Vec<String>,
    read: RefCell<Vec<bool>>,
}

impl Drop for RowBook {
    fn drop(&mut self) {
        fixtures::ledger_read(&self.name, &self.read.borrow());
    }
}

impl RowBook {
    #[must_use]
    pub fn load(name: &str) -> Self {
        let lines = fixtures::rows(name);
        let read = RefCell::new(vec![false; lines.len()]);
        Self {
            name: name.to_owned(),
            lines,
            read,
        }
    }

    /// IDA: N/A every line of a kind and second field, as its fields, marked read.
    pub fn take(&self, kind: &str, field: &str) -> Vec<Vec<&str>> {
        let mut found = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts[0] == kind && parts[1] == field {
                self.read.borrow_mut()[index] = true;
                found.push(parts);
            }
        }
        found
    }

    /// IDA: N/A the one line of a kind and second field.
    pub fn take_one(&self, kind: &str, field: &str) -> Vec<&str> {
        let mut rows = self.take(kind, field);
        assert_eq!(rows.len(), 1, "{kind} {field}");
        rows.remove(0)
    }

    /// IDA: N/A the one line of a kind whose second field is a value.
    pub fn take_one_kind(&self, kind: &str) -> Vec<&str> {
        let index = self
            .lines
            .iter()
            .position(|line| line.split(' ').next() == Some(kind))
            .unwrap_or_else(|| panic!("no {kind} row"));
        self.read.borrow_mut()[index] = true;
        self.lines[index].split(' ').collect()
    }

    /// IDA: N/A every line of a kind, as its fields, marked read.
    pub fn take_kind(&self, kind: &str) -> Vec<Vec<&str>> {
        let mut found = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            let parts: Vec<&str> = line.split(' ').collect();
            if parts[0] == kind {
                self.read.borrow_mut()[index] = true;
                found.push(parts);
            }
        }
        found
    }

    /// IDA: N/A a DATA row's bytes, marked read: the first 0x40 the run dumped behind a pointer at `offset`.
    pub fn data(&self, form_type: &str, id: &str, offset: u32) -> Option<Vec<u8>> {
        let prefix = format!("DATA {form_type} {id} {offset:X} ");
        let index = self
            .lines
            .iter()
            .position(|line| line.starts_with(&prefix))?;
        self.read.borrow_mut()[index] = true;
        Some(hex_bytes(&self.lines[index][prefix.len()..]))
    }

    /// IDA: N/A marks one sample's remaining DATA rows read, and returns their offsets: the run dumps behind any word that looked like a pointer, including unset bytes, and those rows have nothing to compare.
    pub fn take_other_data(&self, form_type: &str, id: &str) -> Vec<u32> {
        let prefix = format!("DATA {form_type} {id} ");
        let mut read = self.read.borrow_mut();
        let mut found = Vec::new();
        for (index, line) in self.lines.iter().enumerate() {
            if line.starts_with(&prefix) && !read[index] {
                read[index] = true;
                let offset = line[prefix.len()..].split(' ').next().unwrap();
                found.push(u32::from_str_radix(offset, 16).unwrap());
                fixtures::ledger_relaxed(
                    &self.name,
                    &format!("DATA {form_type} {id} {offset}"),
                    1,
                    "the run dumped behind a word that only looked like a pointer: no pointer cell in the image, nothing to compare",
                );
            }
        }
        found
    }

    /// IDA: N/A every line is read or deferred, and no deferred line was read, so the table can only shrink.
    pub fn assert_accounted(&self, deferred: &[Deferred]) {
        let read = self.read.borrow();
        let mut unread = Vec::new();
        let mut early = Vec::new();
        let mut counts = vec![0_usize; deferred.len()];
        for (index, line) in self.lines.iter().enumerate() {
            let mut parts = line.split(' ');
            let kind = parts.next().unwrap();
            let field = parts.next().unwrap_or("");
            let group = deferred.iter().position(|(group_kind, fields, _)| {
                *group_kind == kind && (fields.is_empty() || fields.contains(&field))
            });
            match (read[index], group) {
                (true, Some(_)) => early.push(line.chars().take(40).collect::<String>()),
                (false, None) => unread.push(line.chars().take(40).collect::<String>()),
                (false, Some(group)) => counts[group] += 1,
                (true, None) => {}
            }
        }
        for (group, count) in deferred.iter().zip(&counts) {
            fixtures::ledger_deferred(
                &self.name,
                &format!("{} {:?}", group.0, group.1),
                *count,
                group.2,
            );
        }
        assert!(
            unread.is_empty(),
            "{} rows unread and not deferred: {:?}",
            unread.len(),
            &unread[..unread.len().min(20)]
        );
        assert!(
            early.is_empty(),
            "{} rows read but still deferred: remove them from the table: {:?}",
            early.len(),
            &early[..early.len().min(20)]
        );
        for (group, count) in deferred.iter().zip(&counts) {
            println!(
                "deferred to {}: {} {:?}: {count} rows",
                group.2, group.0, group.1
            );
        }
        let total = read.iter().filter(|&&line| line).count();
        println!(
            "rows read: {total} of {}, deferred: {}",
            self.lines.len(),
            counts.iter().sum::<usize>()
        );
    }
}

/// IDA: N/A a hex string's bytes.
#[must_use]
pub fn hex_bytes(text: &str) -> Vec<u8> {
    (0..text.len() / 2)
        .map(|at| u8::from_str_radix(&text[at * 2..at * 2 + 2], 16).unwrap())
        .collect()
}

/// IDA: N/A a comma list of hex IDs.
#[must_use]
pub fn hex_ids(text: &str) -> Vec<u32> {
    text.split(',')
        .map(|id| u32::from_str_radix(id, 16).unwrap())
        .collect()
}
