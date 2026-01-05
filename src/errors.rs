// Copyright 2016-2020 the Tectonic Project
// Licensed under the MIT License.

//! Tectonic error types and support code.
//!
//! This module provides a unified error handling framework using `thiserror`
//! for structured error variants and `anyhow` for flexible error propagation.

#![allow(missing_docs)]

use std::{
    ffi,
    fmt::{self, Debug},
    io,
    io::Write,
    num,
    result::Result as StdResult,
    str,
};
use tectonic_errors::Error as NewError;
use zip::result::ZipError;

cfg_if::cfg_if! {
    if #[cfg(feature = "toml")] {
        pub type ReadError = toml::de::Error;
        pub type WriteError = toml::ser::Error;
    } else {
        use std::error;
        use std::fmt::Display;

        #[derive(Debug)]
        pub enum ReadError {}

        impl Display for ReadError {
            fn fmt(&self, _fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
                Ok(())
            }
        }

        impl error::Error for ReadError { }

        #[derive(Debug)]
        pub enum WriteError {}

        impl Display for WriteError {
            fn fmt(&self, _fmt: &mut fmt::Formatter<'_>) -> fmt::Result {
                Ok(())
            }
        }

        impl error::Error for WriteError { }
    }
}

/// Structured error types for Tectonic operations.
#[derive(Debug, thiserror::Error)]
pub enum TectonicError {
    /// The item is not the expected length.
    #[error("expected length {expected}; found {observed}")]
    BadLength {
        /// Expected length
        expected: usize,
        /// Observed length
        observed: usize,
    },

    /// This stream is not seekable.
    #[error("this stream is not seekable")]
    NotSeekable,

    /// The size of this stream cannot be determined.
    #[error("the size of this stream cannot be determined")]
    NotSizeable,

    /// Access to this file path is forbidden.
    #[error("access to the path {0} is forbidden")]
    PathForbidden(String),

    /// An engine had an unrecoverable error.
    #[error("the {0} engine had an unrecoverable error")]
    EngineError(&'static str),

    /// I/O error.
    #[error(transparent)]
    Io(#[from] io::Error),

    /// Formatting error.
    #[error(transparent)]
    Fmt(#[from] fmt::Error),

    /// Null byte in string.
    #[error(transparent)]
    Nul(#[from] ffi::NulError),

    /// Integer parsing error.
    #[error(transparent)]
    ParseInt(#[from] num::ParseIntError),

    /// Tempfile persistence error.
    #[error(transparent)]
    Persist(#[from] tempfile::PersistError),

    /// Configuration read error.
    #[error("configuration read error: {0}")]
    ConfigRead(#[source] ReadError),

    /// Configuration write error.
    #[error("configuration write error: {0}")]
    ConfigWrite(#[source] WriteError),

    /// New-style error from tectonic_errors crate.
    #[error(transparent)]
    NewStyle(#[from] NewError),

    /// XML parsing error.
    #[error(transparent)]
    QuickXml(#[from] quick_xml::Error),

    /// System time error.
    #[error(transparent)]
    Time(#[from] std::time::SystemTimeError),

    /// UTF-8 decoding error.
    #[error(transparent)]
    Utf8(#[from] str::Utf8Error),

    /// XDV parsing error.
    #[error(transparent)]
    Xdv(#[from] tectonic_xdv::XdvError),

    /// ZIP archive error.
    #[error(transparent)]
    Zip(#[from] ZipError),
}

// Manual From implementations for types that need special handling
impl From<ReadError> for TectonicError {
    fn from(e: ReadError) -> Self {
        TectonicError::ConfigRead(e)
    }
}

impl From<WriteError> for TectonicError {
    fn from(e: WriteError) -> Self {
        TectonicError::ConfigWrite(e)
    }
}

/// The main error type for Tectonic, using anyhow for flexibility.
pub type Error = anyhow::Error;

/// The main result type for Tectonic operations.
pub type Result<T> = anyhow::Result<T>;

/// Legacy compatibility: ErrorKind is now TectonicError
pub type ErrorKind = TectonicError;

/// Use string formatting to create an error message.
#[macro_export]
macro_rules! errmsg {
    ($( $fmt_args:expr ),*) => {
        anyhow::anyhow!($( $fmt_args ),*)
    };
}

/// "Chained try" — like `?`, but with the ability to add context to the error message.
#[macro_export]
macro_rules! ctry {
    ($op:expr ; $( $chain_fmt_args:expr ),*) => {{
        use anyhow::Context;
        $op.with_context(|| format!($( $chain_fmt_args ),*))?
    }}
}

impl From<TectonicError> for io::Error {
    fn from(err: TectonicError) -> io::Error {
        io::Error::other(err.to_string())
    }
}

/// Extension trait for adding context to errors (anyhow::Context re-export).
pub use anyhow::Context as ResultExt;

/// Helper to dump errors to stderr in a user-friendly format.
pub fn dump_uncolorized(err: &Error) {
    let mut prefix = "error:";
    let mut s = io::stderr();

    for cause in err.chain() {
        writeln!(s, "{prefix} {cause}").expect("write to stderr failed");
        prefix = "caused by:";
    }
}

/// The DefinitelySame trait is a helper trait implemented because Errors do
/// not generically implement PartialEq. This is a bit of a drag for testing
/// since it's nice to be able to check if an error matches the one that's
/// expected. DefinitelySame addresses this by providing a weak equivalence
/// test: definitely_same() returns true if the two values definitely are
/// equivalent, and false otherwise.
pub trait DefinitelySame {
    /// Returns true if the two values are definitely equivalent.
    fn definitely_same(&self, other: &Self) -> bool;
}

impl DefinitelySame for Error {
    fn definitely_same(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl DefinitelySame for TectonicError {
    fn definitely_same(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl DefinitelySame for Box<dyn std::error::Error + Send> {
    /// Hack alert! We only compare stringifications.
    fn definitely_same(&self, other: &Self) -> bool {
        self.to_string() == other.to_string()
    }
}

impl<T: DefinitelySame> DefinitelySame for Option<T> {
    fn definitely_same(&self, other: &Self) -> bool {
        match (self, other) {
            (None, None) => true,
            (Some(a), Some(b)) => a.definitely_same(b),
            _ => false,
        }
    }
}

impl<T: DefinitelySame, E: DefinitelySame> DefinitelySame for StdResult<T, E> {
    fn definitely_same(&self, other: &Self) -> bool {
        match *self {
            Ok(ref st) => {
                if let Ok(ref ot) = *other {
                    st.definitely_same(ot)
                } else {
                    false
                }
            }
            Err(ref se) => {
                if let Err(ref oe) = *other {
                    se.definitely_same(oe)
                } else {
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_def_same_option() {
        let a = Some(Box::from(NewError::msg("A")));
        let b = Some(Box::from(NewError::msg("A")));

        assert!(a.definitely_same(&b));
        assert!(b.definitely_same(&a));

        let b = Some(Box::from(NewError::msg("B")));
        assert!(!a.definitely_same(&b));

        let b = None;
        let c = None;
        assert!(!a.definitely_same(&b));
        assert!(b.definitely_same(&c));
    }

    #[test]
    fn test_tectonic_error_display() {
        let err = TectonicError::BadLength {
            expected: 10,
            observed: 5,
        };
        assert_eq!(err.to_string(), "expected length 10; found 5");

        let err = TectonicError::NotSeekable;
        assert_eq!(err.to_string(), "this stream is not seekable");

        let err = TectonicError::PathForbidden("/etc/passwd".to_string());
        assert_eq!(
            err.to_string(),
            "access to the path /etc/passwd is forbidden"
        );

        let err = TectonicError::EngineError("xetex");
        assert_eq!(
            err.to_string(),
            "the xetex engine had an unrecoverable error"
        );
    }

    #[test]
    fn test_errmsg_macro() {
        let err: Error = errmsg!("test error {}", 42);
        assert_eq!(err.to_string(), "test error 42");
    }
}
