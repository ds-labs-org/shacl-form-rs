//! Reads a SHACL shapes graph (real Turtle, an arbitrary shape — see the
//! workspace README for what "arbitrary" does and doesn't mean yet) and
//! turns one `sh:NodeShape` into a [`FormSchema`]: an ordered list of
//! fields a host renders however it likes. [`shacl_form_yew`] is one such
//! host; this crate has no idea it exists.
//!
//! ```
//! use shacl_form_core::{parse_turtle, from_shape_iri};
//! use oxrdf::NamedNode;
//!
//! let shapes = r#"
//!   @prefix sh: <http://www.w3.org/ns/shacl#> .
//!   @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
//!   @prefix ex: <http://example.org/> .
//!   ex:PersonShape a sh:NodeShape ;
//!     sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:minCount 1 ] .
//! "#;
//! let graph = parse_turtle(shapes).unwrap();
//! let schema = from_shape_iri(&graph, &NamedNode::new("http://example.org/PersonShape").unwrap()).unwrap();
//! assert_eq!(schema.fields.len(), 1);
//! assert_eq!(schema.fields[0].label, "name");
//! ```
mod error;
mod graph;
mod model;
mod schema;
mod sh;
mod values;

pub use error::ShapesError;
pub use graph::{parse_instance_turtle, parse_turtle};
pub use model::{Field, FieldKind, FormSchema, SelectOption};
pub use schema::{from_shape_iri, from_target_class};
pub use values::{FormValues, ValueEntry, default_entry, literal_entry};

// Re-exported so a caller needn't add oxrdf as a direct dependency just to
// name a NamedNode/Graph/Term when calling into this crate.
pub use oxrdf;
