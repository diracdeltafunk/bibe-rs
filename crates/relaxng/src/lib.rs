//! A RELAX NG validator, written from the specifications:
//!
//! * the schema language: *RELAX NG Specification*, OASIS Committee
//!   Specification, 3 December 2001 (the XML syntax, the simplification
//!   rules of its section 4 and the restrictions of section 7);
//! * the validation algorithm: James Clark, *An algorithm for RELAX NG
//!   validation* (2002), which validates by computing derivatives of
//!   patterns with respect to the document, with hash-consed patterns and
//!   memoised derivatives;
//! * the datatypes: *Guidelines for using W3C XML Schema Datatypes with
//!   RELAX NG* (OASIS, 2001) and *XML Schema Part 2: Datatypes*, plus the
//!   built-in `string`/`token` library.
//!
//! bibers uses it for biber's `--validate-control`, `--validate-config` and
//! `--validate-bltxml` (docs/compat.md DC-033), where only the verdict is
//! observable: biber reports "failed to validate" without libxml2's
//! individual messages. The messages here follow the shape of libxml2's
//! (`file:line: element NAME: Relax-NG validity error : …`) but their texts
//! are our own.
//!
//! ```
//! let schema = relaxng::Schema::parse(r#"
//!   <element name="a" xmlns="http://relaxng.org/ns/structure/1.0">
//!     <attribute name="n"><data type="integer"
//!       datatypeLibrary="http://www.w3.org/2001/XMLSchema-datatypes"/></attribute>
//!   </element>"#).unwrap();
//! assert!(schema.validate_str(r#"<a n="12"/>"#).is_ok());
//! assert!(schema.validate_str(r#"<a n="x"/>"#).is_err());
//! ```

mod datatype;
mod pattern;
mod schema;
mod validate;
mod xsdregex;

use std::fmt;

pub use roxmltree;

/// The RELAX NG structure namespace.
pub const RNG_NS: &str = "http://relaxng.org/ns/structure/1.0";
/// The W3C XML Schema datatype library URI.
pub const XSD_DATATYPES: &str = "http://www.w3.org/2001/XMLSchema-datatypes";

/// An error in a schema (not well-formed, not a correct RELAX NG schema, or
/// using an unsupported datatype).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SchemaError {
    /// What is wrong.
    pub message: String,
    /// The line in the schema document, when known.
    pub line: Option<u32>,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for SchemaError {}

/// One validity error in a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidityError {
    /// The line of the start tag of `element`.
    pub line: u32,
    /// The local name of the element the error is reported on.
    pub element: String,
    /// What is wrong.
    pub message: String,
}

impl ValidityError {
    /// The error in xmllint's format: `FILE:LINE: element NAME: Relax-NG
    /// validity error : MESSAGE`.
    pub fn display_in(&self, file: &str) -> String {
        format!(
            "{file}:{}: element {}: Relax-NG validity error : {}",
            self.line, self.element, self.message
        )
    }
}

impl fmt::Display for ValidityError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "line {}: element {}: {}",
            self.line, self.element, self.message
        )
    }
}

/// Why a document did not validate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ValidateError {
    /// The document is not well-formed XML.
    NotWellFormed(String),
    /// The document is well-formed but invalid. At least one error.
    Invalid(Vec<ValidityError>),
}

impl fmt::Display for ValidateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ValidateError::NotWellFormed(e) => write!(f, "not well-formed: {e}"),
            ValidateError::Invalid(errs) => {
                for (i, e) in errs.iter().enumerate() {
                    if i > 0 {
                        f.write_str("\n")?;
                    }
                    write!(f, "{e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ValidateError {}

/// Validation options.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Options {
    /// Reproduce libxml2's interleave behaviour, which rejects some valid
    /// documents when an operand of an `interleave` is itself an
    /// `interleave` (black-box model of libxml2 2.9.13; see
    /// `validate.rs`). Off by default: the specification's verdict.
    pub libxml2_interleave: bool,
}

/// Loads the document an `include` or `externalRef` points to: given the
/// base URI of the referencing schema document and the `href`, returns the
/// referenced document's own base URI and its text.
pub type Resolver<'a> = dyn Fn(&str, &str) -> Result<(String, String), String> + 'a;

/// A compiled RELAX NG schema. Immutable and shareable; each validation
/// works on its own copy of the pattern store.
#[derive(Clone, Debug)]
pub struct Schema {
    compiled: pattern::Compiled,
}

/// The parsing options used for schemas and documents (DTDs allowed, for
/// their internal entities).
pub fn parsing_options<'a>() -> roxmltree::ParsingOptions<'a> {
    roxmltree::ParsingOptions {
        allow_dtd: true,
        ..Default::default()
    }
}

impl Schema {
    /// Parse a schema in the RELAX NG XML syntax. `include` and
    /// `externalRef` are errors (use [`Schema::parse_with_resolver`]).
    pub fn parse(text: &str) -> Result<Schema, SchemaError> {
        let no_refs = |_: &str, href: &str| -> Result<(String, String), String> {
            Err(format!("cannot load '{href}': no resolver for external references"))
        };
        Self::parse_with_resolver(text, "", &no_refs)
    }

    /// Parse a schema whose `include`/`externalRef` elements are loaded with
    /// `resolver`. `base` is the base URI of `text`, passed to the resolver.
    pub fn parse_with_resolver(
        text: &str,
        base: &str,
        resolver: &Resolver<'_>,
    ) -> Result<Schema, SchemaError> {
        let ast = schema::parse(text, base, resolver)?;
        let compiled = pattern::compile(&ast)?;
        Ok(Schema { compiled })
    }

    /// Parse a schema file, resolving relative `href`s against its path.
    pub fn from_file(path: &std::path::Path) -> Result<Schema, SchemaError> {
        let read = |p: &std::path::Path| {
            std::fs::read_to_string(p).map_err(|e| format!("cannot read '{}': {e}", p.display()))
        };
        let text = read(path).map_err(|message| SchemaError {
            message,
            line: None,
        })?;
        let resolver = |base: &str, href: &str| -> Result<(String, String), String> {
            let p = std::path::Path::new(base)
                .parent()
                .unwrap_or(std::path::Path::new(""))
                .join(href);
            Ok((p.to_string_lossy().into_owned(), read(&p)?))
        };
        Self::parse_with_resolver(&text, &path.to_string_lossy(), &resolver)
    }

    /// Validate an XML document given as text.
    pub fn validate_str(&self, text: &str) -> Result<(), ValidateError> {
        self.validate_str_with(text, &Options::default())
    }

    /// [`Schema::validate_str`] with options.
    pub fn validate_str_with(&self, text: &str, opts: &Options) -> Result<(), ValidateError> {
        let doc = roxmltree::Document::parse_with_options(text, parsing_options())
            .map_err(|e| ValidateError::NotWellFormed(e.to_string()))?;
        let errs = self.validate_with(&doc, opts);
        if errs.is_empty() {
            Ok(())
        } else {
            Err(ValidateError::Invalid(errs))
        }
    }

    /// Validate a parsed document. Returns the validity errors (empty if
    /// the document is valid). After an error the validator recovers (it
    /// skips the offending element, attribute or text) and goes on, so
    /// several errors can be reported; the verdict is exactly that of the
    /// derivative algorithm.
    pub fn validate(&self, doc: &roxmltree::Document<'_>) -> Vec<ValidityError> {
        validate::validate(&self.compiled, doc, &Options::default())
    }

    /// [`Schema::validate`] with options.
    pub fn validate_with(&self, doc: &roxmltree::Document<'_>, opts: &Options) -> Vec<ValidityError> {
        validate::validate(&self.compiled, doc, opts)
    }

    /// Whether `doc` is valid.
    pub fn is_valid(&self, doc: &roxmltree::Document<'_>) -> bool {
        self.validate(doc).is_empty()
    }
}
