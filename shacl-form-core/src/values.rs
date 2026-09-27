//! What a filled-in form actually holds, and the two directions it moves:
//! [`FormValues::to_turtle`] writes it out as RDF conforming to the shape it
//! was built from; [`FormValues::read_from_instance`] reads it back in from
//! an existing instance graph, for an edit form rather than a create one.
use crate::model::{Field, FieldKind, FormSchema};
use oxrdf::vocab::xsd;
use oxrdf::{
    BlankNode, Graph, Literal, NamedNode, NamedOrBlankNode, SubjectRef, Term, TermRef, TripleRef,
};
use oxttl::TurtleSerializer;

/// One entry a field holds. A `Select`/`Iri` field holds `Node` when its
/// chosen value is a resource and `Literal` when (for `sh:in` over
/// literals) it is not; a `Nested` field holds one `Nested` entry per
/// repeated sub-instance.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueEntry {
    Literal(Literal),
    Node(NamedNode),
    Nested(FormValues),
}

/// Values for one [`FormSchema`], parallel to its `fields` by index — entry
/// `i` here answers for `schema.fields[i]`. Indexed rather than keyed by
/// path because a field with an unsupported (non-single-predicate) path has
/// no path to key by at all, and still needs somewhere to (not) hold a
/// value.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormValues {
    entries: Vec<Vec<ValueEntry>>,
}

impl FormValues {
    /// One slot per field, pre-filled from `sh:defaultValue` where the
    /// shape gave one, then padded up to `sh:minCount` with
    /// [`default_entry`] — a required field a host renders as one visible,
    /// empty control the moment the form opens, not an empty list a user
    /// has to click "add" on before they can satisfy the very
    /// `sh:minCount` that made it required. The starting point for a *new*
    /// instance; an existing one instead starts from
    /// [`FormValues::read_from_instance`], which has real values to show
    /// and does not need this padding.
    pub fn new_for(schema: &FormSchema) -> Self {
        let entries = schema
            .fields
            .iter()
            .map(|field| {
                let mut entries: Vec<ValueEntry> = field
                    .default_value
                    .as_ref()
                    .and_then(term_to_entry)
                    .into_iter()
                    .collect();
                while (entries.len() as u32) < field.min_count {
                    entries.push(default_entry(field));
                }
                entries
            })
            .collect();
        FormValues { entries }
    }

    /// Reads whatever `subject` already asserts for each of `schema`'s
    /// fields out of `graph` — the starting point for an *edit* form. A
    /// `Nested` field's values are read recursively off whichever
    /// subject(s) `subject` points at through that field's own predicate.
    pub fn read_from_instance(schema: &FormSchema, graph: &Graph, subject: SubjectRef<'_>) -> Self {
        let entries = schema
            .fields
            .iter()
            .map(|field| {
                let Some(path) = &field.path else {
                    return Vec::new();
                };
                graph
                    .objects_for_subject_predicate(subject, path.as_ref())
                    .filter_map(|term| match (&field.kind, term) {
                        (
                            FieldKind::Nested { schema },
                            TermRef::NamedNode(_) | TermRef::BlankNode(_),
                        ) => {
                            let nested_subject = term_ref_to_subject(term)?;
                            Some(ValueEntry::Nested(FormValues::read_from_instance(
                                schema,
                                graph,
                                nested_subject,
                            )))
                        }
                        (_, TermRef::Literal(l)) => Some(ValueEntry::Literal(l.into_owned())),
                        (_, TermRef::NamedNode(n)) => Some(ValueEntry::Node(n.into_owned())),
                        _ => None,
                    })
                    .collect()
            })
            .collect();
        FormValues { entries }
    }

    pub fn get(&self, field_index: usize) -> &[ValueEntry] {
        self.entries
            .get(field_index)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    pub fn set(&mut self, field_index: usize, values: Vec<ValueEntry>) {
        if field_index >= self.entries.len() {
            self.entries.resize(field_index + 1, Vec::new());
        }
        self.entries[field_index] = values;
    }

    /// Serialises `subject`'s fields (and, recursively, every `Nested`
    /// value's own subject — freshly minted blank nodes, one per nested
    /// entry) as Turtle. `prefixes` are cosmetic only; the triples are the
    /// same either way.
    pub fn to_turtle(
        &self,
        schema: &FormSchema,
        subject: &NamedOrBlankNode,
        prefixes: &[(&str, &str)],
    ) -> String {
        let mut serializer = TurtleSerializer::new();
        for (name, iri) in prefixes {
            serializer = serializer
                .with_prefix(*name, *iri)
                .expect("caller-supplied prefix IRI must be valid");
        }
        let mut writer = serializer.for_writer(Vec::new());
        self.write_triples(schema, subject, &mut writer);
        String::from_utf8(
            writer
                .finish()
                .expect("writing to an in-memory Vec<u8> cannot fail"),
        )
        .expect("oxttl only ever writes valid UTF-8")
    }

    fn write_triples<W: std::io::Write>(
        &self,
        schema: &FormSchema,
        subject: &NamedOrBlankNode,
        writer: &mut oxttl::turtle::WriterTurtleSerializer<W>,
    ) {
        for (field, entries) in schema.fields.iter().zip(&self.entries) {
            let Some(path) = &field.path else { continue };
            for entry in entries {
                match entry {
                    ValueEntry::Literal(lit) => {
                        let _ = writer.serialize_triple(TripleRef::new(subject, path, lit));
                    }
                    ValueEntry::Node(node) => {
                        let _ = writer.serialize_triple(TripleRef::new(subject, path, node));
                    }
                    ValueEntry::Nested(nested_values) => {
                        let FieldKind::Nested {
                            schema: nested_schema,
                        } = &field.kind
                        else {
                            continue;
                        };
                        let nested_subject = NamedOrBlankNode::BlankNode(BlankNode::default());
                        let _ =
                            writer.serialize_triple(TripleRef::new(subject, path, &nested_subject));
                        nested_values.write_triples(nested_schema, &nested_subject, writer);
                    }
                }
            }
        }
    }
}

/// Turns one entered value (a form-side string, boolean, whatever the field
/// asked for) into the [`ValueEntry`] serialisation should hold, giving it
/// the field's `original_datatype` when one was stated so a round trip does
/// not silently widen e.g. `xsd:nonNegativeInteger` to plain `xsd:integer`.
pub fn literal_entry(field: &Field, lexical: &str) -> ValueEntry {
    let datatype = field
        .original_datatype
        .clone()
        .unwrap_or_else(|| canonical_datatype(&field.kind));
    ValueEntry::Literal(Literal::new_typed_literal(lexical, datatype))
}

/// A blank starting value for one more repetition of `field` — what
/// [`FormValues::new_for`] pads a required field's slots with, and what a
/// host's own "add another" control should reach for too, so there is one
/// definition of "blank" for a given `FieldKind` rather than two that could
/// drift apart.
pub fn default_entry(field: &Field) -> ValueEntry {
    match &field.kind {
        FieldKind::Nested { schema } => ValueEntry::Nested(FormValues::new_for(schema)),
        FieldKind::Select { options } => options
            .first()
            .map(|o| option_to_entry(&o.value))
            .unwrap_or_else(|| literal_entry(field, "")),
        FieldKind::Iri => ValueEntry::Node(NamedNode::new_unchecked("")),
        _ => literal_entry(field, ""),
    }
}

fn option_to_entry(term: &Term) -> ValueEntry {
    match term {
        Term::NamedNode(n) => ValueEntry::Node(n.clone()),
        Term::Literal(l) => ValueEntry::Literal(l.clone()),
        Term::BlankNode(b) => ValueEntry::Literal(Literal::new_simple_literal(b.as_str())),
    }
}

fn canonical_datatype(kind: &FieldKind) -> NamedNode {
    match kind {
        FieldKind::Boolean => xsd::BOOLEAN.into_owned(),
        FieldKind::Date => xsd::DATE.into_owned(),
        FieldKind::DateTime => xsd::DATE_TIME.into_owned(),
        FieldKind::Number {
            integer_only: true, ..
        } => xsd::INTEGER.into_owned(),
        FieldKind::Number {
            integer_only: false,
            ..
        } => xsd::DECIMAL.into_owned(),
        _ => xsd::STRING.into_owned(),
    }
}

fn term_to_entry(term: &Term) -> Option<ValueEntry> {
    match term {
        Term::Literal(l) => Some(ValueEntry::Literal(l.clone())),
        Term::NamedNode(n) => Some(ValueEntry::Node(n.clone())),
        _ => None,
    }
}

fn term_ref_to_subject(term: TermRef<'_>) -> Option<SubjectRef<'_>> {
    match term {
        TermRef::NamedNode(n) => Some(SubjectRef::NamedNode(n)),
        TermRef::BlankNode(b) => Some(SubjectRef::BlankNode(b)),
        _ => None,
    }
}
