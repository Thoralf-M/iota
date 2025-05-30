// Copyright (c) The Diem Core Contributors
// Copyright (c) The Move Contributors
// Modifications Copyright (c) 2024 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//! A helper for generating structured code.
//!
//! TODO(wrwg): this should be moved somewhere else. It is currently contained
//! here   so its on the bottom of the dependency relation, and there is no
//! `utility` crate   where it could belong to.

use std::{cell::RefCell, collections::BTreeMap};

use codespan::{ByteIndex, ByteOffset, RawIndex, RawOffset};

use crate::model::Loc;

struct CodeWriterData {
    /// A function to be called on each emitted string. If the function does not
    /// change anything, it returns None.
    emit_hook: Box<dyn Fn(&str) -> Option<String>>,

    /// The generated output string.
    output: String,

    /// Current active indentation.
    indent: usize,

    /// Current active location.
    current_location: Loc,

    /// A sparse mapping from byte index in written output to location in source
    /// file. Any index not in this map is approximated by the next smaller
    /// index on lookup.
    output_location_map: BTreeMap<ByteIndex, Loc>,

    /// A map from label indices to the current position in output they are
    /// pointing to.
    label_map: BTreeMap<ByteIndex, ByteIndex>,
}

/// A helper to emit code. Supports indentation and maintains source to target
/// location information.
pub struct CodeWriter(RefCell<CodeWriterData>);

/// A label which can be created at the code writers current output position to
/// later insert code at this position.
#[derive(Debug, Clone, Copy)]
pub struct CodeWriterLabel(ByteIndex);

impl CodeWriter {
    /// Creates new code writer, with the given default location.
    pub fn new(loc: Loc) -> CodeWriter {
        let zero = ByteIndex(0);
        let mut output_location_map = BTreeMap::new();
        output_location_map.insert(zero, loc.clone());
        Self(RefCell::new(CodeWriterData {
            emit_hook: Box::new(|_| None),
            output: String::new(),
            indent: 0,
            current_location: loc,
            output_location_map,
            label_map: Default::default(),
        }))
    }

    pub fn set_emit_hook<F>(&self, f: F)
    where
        F: Fn(&str) -> Option<String>,
        F: 'static,
    {
        let mut data = self.0.borrow_mut();
        data.emit_hook = Box::new(f)
    }

    /// Creates a label at which code can be inserted later.
    pub fn create_label(&self) -> CodeWriterLabel {
        let mut data = self.0.borrow_mut();
        let index = ByteIndex(data.output.len() as RawIndex);
        data.label_map.insert(index, index);
        CodeWriterLabel(index)
    }

    /// Inserts code at the previously created label.
    pub fn insert_at_label(&self, label: CodeWriterLabel, s: &str) {
        let mut data = self.0.borrow_mut();
        let index = *data.label_map.get(&label.0).expect("label undefined");
        let shift = ByteOffset(s.len() as RawOffset);
        // Shift indices after index.
        data.label_map = std::mem::take(&mut data.label_map)
            .into_iter()
            .map(|(i, j)| if j > index { (i, j + shift) } else { (i, j) })
            .collect();
        data.output_location_map = std::mem::take(&mut data.output_location_map)
            .into_iter()
            .map(|(i, loc)| {
                if i > index {
                    (i + shift, loc)
                } else {
                    (i, loc)
                }
            })
            .collect();
        // Insert text.
        data.output.insert_str(index.0 as usize, s);
    }

    /// Calls a function to process the code written so far. This is embedded
    /// into a function so we ensure correct scoping of borrowed RefCell
    /// content.
    pub fn process_result<T, F: FnMut(&str) -> T>(&self, mut f: F) -> T {
        // Ensure that result is terminated by newline without spaces.
        // This assumes that we already trimmed all individual lines.
        let data = self.0.borrow();
        let output = data.output.as_str();
        let mut j = output.trim_end().len();
        if j < output.len() && output[j..].starts_with('\n') {
            j += 1;
        }
        f(&output[0..j])
    }

    /// Extracts the output as a string. Leaves the writers data empty.
    pub fn extract_result(&self) -> String {
        let mut s = std::mem::take(&mut self.0.borrow_mut().output);
        // Eliminate any empty lines at end, but keep the lest EOL
        s.truncate(s.trim_end().len());
        if !s.ends_with('\n') {
            s.push('\n');
        }
        s
    }

    /// Sets the current location. This location will be associated with all
    /// subsequently written code so we can map back from the generated code
    /// to this location. If current loc is already the passed one, nothing
    /// will be updated, so it is ok to call this method repeatedly with the
    /// same value.
    pub fn set_location(&self, loc: &Loc) {
        let mut data = self.0.borrow_mut();
        let code_at = ByteIndex(data.output.len() as u32);
        if &data.current_location != loc {
            data.output_location_map.insert(code_at, loc.clone());
            data.current_location = loc.clone();
        }
    }

    /// Indents any subsequently written output. The current line of output and
    /// any subsequent ones will be indented. Note this works after the last
    /// output was `\n` but the line is still empty.
    pub fn indent(&self) {
        let mut data = self.0.borrow_mut();
        data.indent += 4;
    }

    /// Undo previously done indentation.
    pub fn unindent(&self) {
        let mut data = self.0.borrow_mut();
        assert!(data.indent >= 4);
        data.indent -= 4;
    }

    /// Emit some code with indentation
    pub fn with_indent<F>(&self, mut f: F)
    where
        F: FnMut(),
    {
        self.indent();
        f();
        self.unindent();
    }

    /// Emit a string. The string will be broken down into lines to apply
    /// current indentation.
    pub fn emit(&self, s: &str) {
        let rewritten = (*self.0.borrow().emit_hook)(s);
        let s = if let Some(r) = &rewritten {
            r.as_str()
        } else {
            s
        };
        let mut first = true;
        // str::lines ignores trailing newline, so deal with this ad-hoc
        let end_newl = s.ends_with('\n');
        for l in s.lines() {
            if first {
                first = false
            } else {
                Self::trim_trailing_whitespace(&mut self.0.borrow_mut().output);
                self.0.borrow_mut().output.push('\n');
            }
            self.emit_str(l)
        }
        if end_newl {
            Self::trim_trailing_whitespace(&mut self.0.borrow_mut().output);
            self.0.borrow_mut().output.push('\n');
        }
    }

    fn trim_trailing_whitespace(s: &mut String) {
        s.truncate(s.trim_end_matches(' ').len());
    }

    /// Emits a string and then terminates the line.
    pub fn emit_line(&self, s: &str) {
        self.emit(s.trim_end_matches(' '));
        self.emit("\n");
    }

    /// Helper for emitting a string for a single line.
    fn emit_str(&self, s: &str) {
        let mut data = self.0.borrow_mut();
        // If we are looking at the beginning of a new line, emit indent now.
        if data.indent > 0 && (data.output.is_empty() || data.output.ends_with('\n')) {
            let n = data.indent;
            data.output.push_str(&" ".repeat(n));
        }
        data.output.push_str(s);
    }
}

/// Macro to emit a simple or formatted string.
#[macro_export]
macro_rules! emit {
    ($target:expr, $s:expr) => (
       $target.emit($s)
    );
    ($target:expr, $s:expr, $($arg:expr),+ $(,)?) => (
       $target.emit(&format!($s, $($arg),+))
    )
}

/// Macro to emit a simple or formatted string followed by a new line.
#[macro_export]
macro_rules! emitln {
    ($target:expr) => (
       $target.emit_line("")
    );
    ($target:expr, $s:expr) => (
       $target.emit_line($s)
    );
    ($target:expr, $s:expr, $($arg:expr),+ $(,)?) => (
       $target.emit_line(&format!($s, $($arg),+))
    )
}
