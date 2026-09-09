//! Which of the compiler's two source dialects a document is written in.
//!
//! `.laplace` is a project file: the usual Stan program structure, with
//! `library { }` and `pkg::func` on top. `.laplacelib` is the library
//! dialect (`laplace::parser::laplacelib`): the same expression, statement
//! and import syntax, but only function definitions -- bare at top level or
//! inside an optional `functions { }` wrapper -- plus an optional
//! `library { }` block. The model-shaped blocks are rejected outright.
//!
//! Almost everything the server does is dialect-independent, because
//! `library { }` and `pkg::func` mean the same thing in both. The three
//! places that must care are marked by callers of [`Dialect::is_library`]:
//! which blocks are legal, which block keywords to complete, and how to
//! assemble a Stan program for `stanc` to check.

use std::path::Path;

use tower_lsp::lsp_types::Url;

use laplace::parser::laplacelib::LAPLACELIB_EXTENSION;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    /// A `.laplace` project file.
    Laplace,
    /// A `.laplacelib` library file.
    Library,
}

impl Dialect {
    pub fn for_path(path: &Path) -> Dialect {
        match path.extension().and_then(|e| e.to_str()) {
            Some(LAPLACELIB_EXTENSION) => Dialect::Library,
            _ => Dialect::Laplace,
        }
    }

    /// The dialect of the document at `uri`. A URI that isn't a file path
    /// (an untitled buffer, say) is treated as `.laplace`: it is the
    /// stricter of the two, so nothing is silently let through.
    pub fn for_uri(uri: &Url) -> Dialect {
        uri.to_file_path()
            .map(|p| Dialect::for_path(&p))
            .unwrap_or(Dialect::Laplace)
    }

    pub fn is_library(self) -> bool {
        self == Dialect::Library
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialect_comes_from_the_file_extension() {
        assert_eq!(Dialect::for_path(Path::new("/m/model.laplace")), Dialect::Laplace);
        assert_eq!(Dialect::for_path(Path::new("/m/stats.laplacelib")), Dialect::Library);
    }

    #[test]
    fn an_unknown_or_missing_extension_is_treated_as_the_project_dialect() {
        assert_eq!(Dialect::for_path(Path::new("/m/notes.txt")), Dialect::Laplace);
        assert_eq!(Dialect::for_path(Path::new("/m/README")), Dialect::Laplace);
    }
}
