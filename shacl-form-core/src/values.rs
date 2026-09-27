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
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

/// One entry a field holds. A `Select`/`Iri` field holds `Node` when its
/// chosen value is a resource and `Literal` when (for `sh:in` over
/// literals) it is not; a `Nested` field holds one `Nested` entry per
/// repeated sub-instance, carrying the subject it was read from (`None` for
/// a value created fresh in this session, not read from anywhere) so
/// serialising an edited instance reuses that identity instead of minting a
/// new blank node that silently replaces it — see `write_triples`.
#[derive(Debug, Clone, PartialEq)]
pub enum ValueEntry {
    Literal(Literal),
    Node(NamedNode),
    Nested {
        subject: Option<NamedOrBlankNode>,
        values: Rc<FormValues>,
    },
}

/// Values for one [`FormSchema`], parallel to its `fields` by index — entry
/// `i` here answers for `schema.fields[i]`. Indexed rather than keyed by
/// path because a field with an unsupported (non-single-predicate) path has
/// no path to key by at all, and still needs somewhere to (not) hold a
/// value.
///
/// Each field's row is an `Rc<[ValueEntry]>`, not a plain `Vec`: an edit
/// three levels deep in a nested shape (see `shacl-form-yew`'s
/// `paths::update_at`) rebuilds the *spine* down to that field — cloning
/// `FormValues` itself at each level on the way — but every other field's
/// row, at every level, is a pointer bump, not a copy of its contents. A
/// plain `Vec<Vec<ValueEntry>>` would deep-clone the whole tree (all
/// literals, all nested repetitions, everywhere) on every single keystroke.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct FormValues {
    entries: Vec<Rc<[ValueEntry]>>,
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
                Rc::from(entries)
            })
            .collect();
        FormValues { entries }
    }

    /// Reads whatever `subject` already asserts for each of `schema`'s
    /// fields out of `graph` — the starting point for an *edit* form. A
    /// `Nested` field's values are read recursively off whichever
    /// subject(s) `subject` points at through that field's own predicate,
    /// and that subject is kept (see [`ValueEntry::Nested`]) so re-saving
    /// an untouched value reproduces it exactly rather than replacing it
    /// with a fresh blank node.
    ///
    /// An instance graph is a graph, not a tree: the same subject can be
    /// reached through more than one path, and — `foaf:knows`-style — can
    /// point back at one of its own ancestors. Two guards make that safe:
    /// a subject already being expanded higher up the *current* path is a
    /// real cycle and is not re-expanded (an empty, otherwise-identity-only
    /// value instead — the data-driven equivalent of `schema.rs`'s own
    /// nesting-depth cap, which stops the same recursion from the *shape*
    /// side); a subject reached again at the same nested schema via a
    /// *different*, non-ancestor path (any instance with several
    /// cross-references to the same resource — not necessarily cyclic at
    /// all) reuses the already-computed [`Rc<FormValues>`] instead of
    /// re-walking an identical subtree, the same trade [`crate::schema`]'s
    /// own `ShapeCache` makes for shapes. Without both, a handful of
    /// mutually cross-referencing subjects (any real "people who know each
    /// other" instance) expands combinatorially — see
    /// `shacl-form-core/tests/opus3_audit.rs`.
    pub fn read_from_instance(schema: &FormSchema, graph: &Graph, subject: SubjectRef<'_>) -> Self {
        let mut cache: ReadCache = HashMap::new();
        let mut ancestors: HashSet<String> = HashSet::new();
        ancestors.insert(subject_key(subject));
        let entries = read_fields(schema, graph, subject, &mut cache, &mut ancestors);
        FormValues { entries }
    }

    pub fn get(&self, field_index: usize) -> &[ValueEntry] {
        self.entries
            .get(field_index)
            .map(|rc| rc.as_ref())
            .unwrap_or(&[])
    }

    pub fn set(&mut self, field_index: usize, values: Vec<ValueEntry>) {
        if field_index >= self.entries.len() {
            self.entries.resize(field_index + 1, Rc::from(Vec::new()));
        }
        self.entries[field_index] = Rc::from(values);
    }

    /// Serialises `subject`'s fields (and, recursively, every `Nested`
    /// value's own subject) as Turtle. `prefixes` are cosmetic only; the
    /// triples are the same either way. Also asserts `rdf:type` for
    /// `subject` when `schema.target_class` names one — a `sh:targetClass`
    /// form, or a value nested under a `sh:class` constraint, is not a
    /// valid instance of that class without it.
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
        if let Some(class) = &schema.target_class {
            let _ =
                writer.serialize_triple(TripleRef::new(subject, oxrdf::vocab::rdf::TYPE, class));
        }
        self.write_triples(schema, subject, &mut writer, "");
        String::from_utf8(
            writer
                .finish()
                .expect("writing to an in-memory Vec<u8> cannot fail"),
        )
        .expect("oxttl only ever writes valid UTF-8")
    }

    /// `tree_path` names *this call's* position in the value tree as a
    /// short string (`"f{field}r{rep}"` segments, one per nesting level) —
    /// used only to mint a stable blank node id for an unsaved (`subject:
    /// None`) `Nested` value, so that serialising the *same* `FormValues*`
    /// twice (a user re-submitting after a failed save, with no edits in
    /// between) names the same blank node both times instead of a fresh
    /// random one per call — an id `BlankNode::default()` cannot give,
    /// since it exists precisely to be unique every time it's called.
    fn write_triples<W: std::io::Write>(
        &self,
        schema: &FormSchema,
        subject: &NamedOrBlankNode,
        writer: &mut oxttl::turtle::WriterTurtleSerializer<W>,
        tree_path: &str,
    ) {
        for (field_idx, (field, entries)) in schema.fields.iter().zip(&self.entries).enumerate() {
            let Some(path) = &field.path else { continue };
            for (rep, entry) in entries.iter().enumerate() {
                match entry {
                    // An empty lexical form / empty IRI is what an untouched
                    // required-but-not-yet-typed-into control looks like
                    // (see `default_entry`'s Text/Iri blanks) — writing it
                    // out would assert `ex:name ""` or `ex:home <>`, an
                    // ill-formed-in-spirit triple that makes "required"
                    // meaningless. Skipped, not written.
                    ValueEntry::Literal(lit) if lit.value().is_empty() => {}
                    ValueEntry::Node(node) if node.as_str().is_empty() => {}
                    ValueEntry::Literal(lit) => {
                        let _ = writer.serialize_triple(TripleRef::new(subject, path, lit));
                    }
                    ValueEntry::Node(node) => {
                        // An IRI the user typed may not be well-formed —
                        // `controls.rs`'s Iri control accepts free text as
                        // they type, on purpose, so a half-typed value
                        // doesn't fight the user mid-keystroke. Validated
                        // here, at the one point that matters: what
                        // actually gets written. An invalid one is skipped
                        // rather than written as unparseable Turtle.
                        if NamedNode::new(node.as_str()).is_ok() {
                            let _ = writer.serialize_triple(TripleRef::new(subject, path, node));
                        }
                    }
                    ValueEntry::Nested {
                        subject: nested_subject,
                        values: nested_values,
                    } => {
                        let FieldKind::Nested {
                            schema: nested_schema,
                        } = &field.kind
                        else {
                            continue;
                        };
                        // Reuse the subject this value was read from when
                        // there is one, rather than always minting a fresh
                        // blank node — the difference between re-saving
                        // `ex:bob` as `ex:bob` and silently replacing every
                        // reference to `ex:bob` with a copy of it. Absent
                        // that, derive a stable id from this value's own
                        // position in the tree rather than a fresh random
                        // one, so re-serialising unedited data twice names
                        // the same blank node both times.
                        let child_path = format!("{tree_path}f{field_idx}r{rep}");
                        let owned_subject;
                        let subject_ref = match nested_subject {
                            Some(s) => s,
                            None => {
                                owned_subject = NamedOrBlankNode::BlankNode(
                                    BlankNode::new_unchecked(format!("shaclform{child_path}")),
                                );
                                &owned_subject
                            }
                        };
                        let _ = writer.serialize_triple(TripleRef::new(subject, path, subject_ref));
                        if let Some(class) = &nested_schema.target_class {
                            let _ = writer.serialize_triple(TripleRef::new(
                                subject_ref,
                                oxrdf::vocab::rdf::TYPE,
                                class,
                            ));
                        }
                        nested_values.write_triples(
                            nested_schema,
                            subject_ref,
                            writer,
                            &child_path,
                        );
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
///
/// `DateTime` gets one normalisation: an HTML `datetime-local` control's own
/// value omits seconds when the user hasn't touched them (`"2024-05-01T10:30"`),
/// which is not a legal `xsd:dateTime` lexical form (seconds are mandatory).
/// Padded to `:00` here rather than left for a caller to remember, since
/// every caller goes through this function.
pub fn literal_entry(field: &Field, lexical: &str) -> ValueEntry {
    let datatype = field
        .original_datatype
        .clone()
        .unwrap_or_else(|| canonical_datatype(&field.kind));
    let lexical = if matches!(field.kind, FieldKind::DateTime)
        && lexical.len() == 16
        && lexical.as_bytes().get(10) == Some(&b'T')
    {
        format!("{lexical}:00")
    } else {
        lexical.to_string()
    };
    ValueEntry::Literal(Literal::new_typed_literal(lexical, datatype))
}

/// A blank starting value for one more repetition of `field` — what
/// [`FormValues::new_for`] pads a required field's slots with, and what a
/// host's own "add another" control should reach for too, so there is one
/// definition of "blank" for a given `FieldKind` rather than two that could
/// drift apart.
pub fn default_entry(field: &Field) -> ValueEntry {
    match &field.kind {
        // Assigned a real, globally-unique identity right now, at creation
        // — not left as `None` for `write_triples` to derive one from this
        // value's position in the tree later. A position-derived id can
        // collide with a *different*, already-real subject that a previous
        // save happened to mint at that very label (e.g. this same position,
        // after an earlier entry there was removed and a new one added) —
        // silently merging two distinct resources into one. `BlankNode`'s
        // own random generator (see `oxrdf`) cannot collide with a name any
        // save has ever produced or any instance could ever have been read
        // with. `write_triples` still derives a position-based id for a
        // `None` subject — kept for a value built directly rather than
        // through this function — solely so *that* narrower case still
        // serialises identically across repeated unedited calls.
        FieldKind::Nested { schema } => ValueEntry::Nested {
            subject: Some(NamedOrBlankNode::BlankNode(BlankNode::default())),
            values: Rc::new(FormValues::new_for(schema)),
        },
        FieldKind::Select { options } => options
            .first()
            .map(|o| option_to_entry(&o.value))
            .unwrap_or_else(|| literal_entry(field, "")),
        FieldKind::Iri => ValueEntry::Node(NamedNode::new_unchecked("")),
        // "" is not a legal xsd:boolean lexical form; a genuinely blank
        // boolean is a checkbox that starts unchecked, i.e. false.
        FieldKind::Boolean => literal_entry(field, "false"),
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

/// Every `(nested schema, subject)` pair [`read_fields`] has already fully
/// expanded into an `Rc<FormValues>` — keyed by the schema's own `Rc`
/// pointer (each nesting *depth* gets its own distinct cached
/// `Rc<FormSchema>` in `schema.rs`'s `ShapeCache`, so this key, like that
/// one, is really `(schema, depth, subject)` in effect) so the same subject
/// reached twice at the same depth is read once and shared, not re-walked.
type ReadCache = HashMap<(usize, String), Rc<FormValues>>;

fn subject_key(subject: SubjectRef<'_>) -> String {
    match subject {
        SubjectRef::NamedNode(n) => n.as_str().to_string(),
        SubjectRef::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

/// One `FormValues` level's worth of rows, recursing into `read_nested` for
/// every `Nested` field's value — the part `read_from_instance` and
/// `read_nested` both need, extracted so the entry point doesn't require an
/// `Rc<FormSchema>` it may not have (a caller's *root* schema is often a
/// plain `&FormSchema`; only a `Nested` field's own schema is ever `Rc`'d).
fn read_fields(
    schema: &FormSchema,
    graph: &Graph,
    subject: SubjectRef<'_>,
    cache: &mut ReadCache,
    ancestors: &mut HashSet<String>,
) -> Vec<Rc<[ValueEntry]>> {
    schema
        .fields
        .iter()
        .map(|field| {
            let Some(path) = &field.path else {
                return Rc::from(Vec::new());
            };
            let row: Vec<ValueEntry> = graph
                .objects_for_subject_predicate(subject, path.as_ref())
                .filter_map(|term| match (&field.kind, term) {
                    (
                        FieldKind::Nested { schema: nested },
                        TermRef::NamedNode(_) | TermRef::BlankNode(_),
                    ) => {
                        let nested_subject = term_ref_to_subject(term)?;
                        Some(ValueEntry::Nested {
                            subject: Some(subject_ref_to_owned(nested_subject)),
                            values: read_nested(nested, graph, nested_subject, cache, ancestors),
                        })
                    }
                    (_, TermRef::Literal(l)) => Some(ValueEntry::Literal(l.into_owned())),
                    (_, TermRef::NamedNode(n)) => Some(ValueEntry::Node(n.into_owned())),
                    _ => None,
                })
                .collect();
            Rc::from(row)
        })
        .collect()
}

/// Reads (or reuses the cached read of) one `Nested` field's value.
/// `ancestors` is the set of subjects currently being expanded somewhere
/// *above* this call on the current path — reaching one of them again here
/// is a real cycle (this exact resource nested inside itself, however many
/// steps removed), not merely "the same subject shared from two unrelated
/// places", and is not re-expanded: doing so would recurse until
/// `schema.rs`'s own nesting-depth cap cut it off, producing one
/// independently-read copy per depth level that can silently disagree with
/// every other copy of the very same subject once only one of them is
/// edited (see `opus3_audit.rs`). Once a subject's read is complete it is
/// removed from `ancestors` (so a sibling branch that is not itself a
/// cycle can still read it fresh) and, keyed by `(schema, subject)`, kept in
/// `cache` for the rest of this whole read — reused by every other
/// occurrence at that same nesting depth instead of re-walked.
fn read_nested(
    schema: &Rc<FormSchema>,
    graph: &Graph,
    subject: SubjectRef<'_>,
    cache: &mut ReadCache,
    ancestors: &mut HashSet<String>,
) -> Rc<FormValues> {
    let subject_key = subject_key(subject);
    if ancestors.contains(&subject_key) {
        return Rc::new(FormValues::default());
    }
    let cache_key = (Rc::as_ptr(schema) as usize, subject_key.clone());
    if let Some(cached) = cache.get(&cache_key) {
        return cached.clone();
    }
    ancestors.insert(subject_key.clone());
    let entries = read_fields(schema, graph, subject, cache, ancestors);
    ancestors.remove(&subject_key);
    let rc = Rc::new(FormValues { entries });
    cache.insert(cache_key, rc.clone());
    rc
}

fn term_ref_to_subject(term: TermRef<'_>) -> Option<SubjectRef<'_>> {
    match term {
        TermRef::NamedNode(n) => Some(SubjectRef::NamedNode(n)),
        TermRef::BlankNode(b) => Some(SubjectRef::BlankNode(b)),
        _ => None,
    }
}

fn subject_ref_to_owned(subject: SubjectRef<'_>) -> NamedOrBlankNode {
    match subject {
        SubjectRef::NamedNode(n) => NamedOrBlankNode::NamedNode(n.into_owned()),
        SubjectRef::BlankNode(b) => NamedOrBlankNode::BlankNode(b.into_owned()),
    }
}
