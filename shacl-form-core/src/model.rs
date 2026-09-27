//! The output of reading a shape: a [`FormSchema`], deliberately not an RDF
//! structure itself. A host renders this — or, given the same input, writes
//! its own renderer without linking `shacl-form-yew` at all.
use oxrdf::{NamedNode, Term};

/// One field an HTML form should offer for one `sh:property` of the shape
/// (or, at the top level, for the target subject's own asserted type).
#[derive(Debug, Clone, PartialEq)]
pub struct Field {
    /// The predicate this field writes/reads, when the path is a plain
    /// `sh:path <iri>`. `None` for a path this crate could not represent (a
    /// blank-node sequence/alternative/inverse path) — the field still
    /// exists, so a reader is not silently missing a property the shape
    /// actually declares; see `unsupported`.
    pub path: Option<NamedNode>,
    /// `sh:name` if present, else the path's own last IRI segment, else the
    /// literal string `"value"` for a path this crate could not name.
    pub label: String,
    pub description: Option<String>,
    /// `sh:order`, when the shape stated one — used to sort fields; absent
    /// means "after every field that stated one", in `sh:property` order.
    pub order: Option<f64>,
    pub min_count: u32,
    /// `None` means unbounded (`sh:maxCount` absent).
    pub max_count: Option<u32>,
    pub kind: FieldKind,
    /// The exact `sh:datatype` the shape stated, when it stated one more
    /// specific than `kind`'s own category (e.g. `xsd:nonNegativeInteger`,
    /// not just "a number"). Carried so serialisation can round-trip the
    /// precise datatype instead of collapsing every integer subtype to
    /// plain `xsd:integer`.
    pub original_datatype: Option<NamedNode>,
    /// `sh:defaultValue`, applied only when creating a new instance (never
    /// overwrites a value read back from an existing one).
    pub default_value: Option<Term>,
    /// `Some(reason)` when a constraint on this property could not be
    /// faithfully turned into a control — the field still renders (as
    /// whatever `kind` says, usually plain text), so the property is never
    /// just missing, but a reader must not assume the rendered control
    /// enforces everything the shape said.
    pub unsupported: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum FieldKind {
    Text {
        pattern: Option<String>,
        min_length: Option<u32>,
        max_length: Option<u32>,
    },
    Number {
        integer_only: bool,
        min: Option<f64>,
        max: Option<f64>,
    },
    Boolean,
    Date,
    DateTime,
    /// A resource reference with no closed set of options: `sh:nodeKind
    /// sh:IRI`, or `sh:class` naming a class this crate found no matching
    /// `sh:NodeShape` for (so it cannot offer a nested form or a fixed list).
    Iri,
    /// `sh:in` (a closed enumeration) or `sh:hasValue` (exactly one fixed
    /// value — still an editable `Select` only when `min_count == 0` allows
    /// omitting it; a `sh:hasValue` field with `min_count >= 1` is instead
    /// rendered read-only by the host, the same `SelectOption` list making
    /// that trivial: one entry, pre-selected).
    Select {
        options: Vec<SelectOption>,
    },
    /// `sh:node` (or `sh:class` resolved to a matching `sh:NodeShape`): the
    /// value is itself shaped, so it gets its own nested `FormSchema` rather
    /// than one control.
    Nested {
        schema: Box<FormSchema>,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectOption {
    pub value: Term,
    pub label: String,
}

/// One shape's worth of fields, in the order a form should render them.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormSchema {
    pub title: Option<String>,
    pub description: Option<String>,
    pub fields: Vec<Field>,
    /// `sh:closed true` on the shape this schema came from — informational
    /// only (see the crate README's "What sh:closed means here" note): this
    /// crate never emits a property it does not know about, so a closed
    /// shape needs no extra enforcement here, only a place to say so.
    pub closed: bool,
    /// A constraint this crate saw on the *shape itself* (not one property)
    /// and did not implement — `sh:sparql`, `sh:disjoint`, `sh:equals`,
    /// `sh:lessThan`, `sh:qualifiedValueShape`, and any `sh:`-namespaced
    /// predicate this crate does not otherwise branch on. Never silently
    /// dropped; a host can show these as "this form does not fully validate
    /// against the shape" text.
    pub unsupported: Vec<String>,
}

impl FormSchema {
    /// Every field carrying its own `unsupported` note, plus every
    /// shape-level one — flattened, for a host that just wants one list to
    /// show a reader rather than walking the tree itself.
    pub fn all_unsupported(&self) -> Vec<String> {
        let mut out = self.unsupported.clone();
        for field in &self.fields {
            if let Some(reason) = &field.unsupported {
                out.push(format!("{}: {reason}", field.label));
            }
            if let FieldKind::Nested { schema } = &field.kind {
                out.extend(
                    schema
                        .all_unsupported()
                        .into_iter()
                        .map(|r| format!("{}.{r}", field.label)),
                );
            }
        }
        out
    }
}
