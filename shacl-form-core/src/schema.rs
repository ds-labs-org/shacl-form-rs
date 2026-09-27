//! Walks a parsed shapes graph and produces a [`FormSchema`]. This is the
//! one module that knows what a `sh:NodeShape`/`sh:PropertyShape` *means*;
//! [`crate::model`] only knows what a form looks like once this module is
//! done deciding.
use crate::error::ShapesError;
use crate::model::{Field, FieldKind, FormSchema, SelectOption};
use crate::sh;
use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Graph, NamedNode, NamedNodeRef, Subject, SubjectRef, Term, TermRef, TripleRef};
use std::collections::HashSet;

/// How many `sh:node`/`sh:class` levels this crate will nest before it
/// stops and falls back to a plain IRI field instead. Not a SHACL concept,
/// and deliberately not "stop the instant a shape reappears anywhere on the
/// path" either: real ontologies commonly nest a shape inside itself on
/// purpose (`foaf:knows` pointing back at `foaf:Person`, `hive:inGroup`
/// eventually reaching back to a `hive:Group`-shaped thing), and each such
/// level is a real, useful, fillable field — only runaway recursion is the
/// problem. A fixed depth bounds that in every case (self-cycle, mutual
/// cycle, or a long acyclic chain that just happens to be deep) without
/// needing to tell those cases apart.
const MAX_NESTING_DEPTH: u32 = 6;

pub fn from_shape_iri(graph: &Graph, shape_iri: &NamedNode) -> Result<FormSchema, ShapesError> {
    let subject = Subject::NamedNode(shape_iri.clone());
    if !graph.contains(TripleRef::new(subject.as_ref(), rdf::TYPE, sh::NODE_SHAPE)) {
        return Err(ShapesError::NotANodeShape(shape_iri.as_str().to_string()));
    }
    Ok(walk_node_shape(graph, subject.as_ref(), 0))
}

/// SHACL lets more than one `sh:NodeShape` target the same class — this
/// repository's own `vendor/ds-honeycomb-editor-rs/shapes.ttl` does exactly
/// that (`hive:Diagram` is targeted by `hsh:DiagramShape`, which has
/// `sh:property`, *and* by three SPARQL-only shapes with none) — so this
/// merges every matching shape's fields into one schema rather than picking
/// one arbitrarily, the way `sh:and` already merges branches within one
/// property shape.
pub fn from_target_class(graph: &Graph, class_iri: &NamedNode) -> Result<FormSchema, ShapesError> {
    let shapes = find_node_shapes_for_class(graph, class_iri.as_ref());
    if shapes.is_empty() {
        return Err(ShapesError::NoShapeForClass(class_iri.as_str().to_string()));
    }
    let mut merged = FormSchema::default();
    for shape in shapes {
        let one = walk_node_shape(graph, shape.as_ref(), 0);
        merged.title = merged.title.or(one.title);
        merged.description = merged.description.or(one.description);
        merged.closed |= one.closed;
        merged.unsupported.extend(one.unsupported);
        merged.fields.extend(one.fields);
    }
    merged.fields.sort_by(|a, b| {
        order_key(a.order)
            .total_cmp(&order_key(b.order))
            .then_with(|| tie_break_key(a).cmp(tie_break_key(b)))
    });
    Ok(merged)
}

fn find_node_shape_for_class(graph: &Graph, class: NamedNodeRef<'_>) -> Option<Subject> {
    find_node_shapes_for_class(graph, class).into_iter().next()
}

fn find_node_shapes_for_class(graph: &Graph, class: NamedNodeRef<'_>) -> Vec<Subject> {
    let mut shapes: Vec<Subject> = graph
        .subjects_for_predicate_object(sh::TARGET_CLASS, class)
        .filter(|s| graph.contains(TripleRef::new(*s, rdf::TYPE, sh::NODE_SHAPE)))
        .map(SubjectRef::into_owned)
        .collect();
    // `oxrdf::Graph` iterates in whatever order its internal set happens to
    // hold triples, which is not necessarily insertion order and is not
    // guaranteed stable across versions — sorting by the shape's own IRI
    // (or blank node id) is what makes `from_target_class`'s merged field
    // order deterministic between runs, not just "whatever HashSet handed
    // back this time".
    shapes.sort_by_key(shape_key_owned);
    shapes
}

fn shape_key_owned(subject: &Subject) -> String {
    shape_key(subject.as_ref())
}

/// Node shapes this crate reads a form-level fact from directly — anything
/// else in the `sh:` namespace on the shape subject is a constraint this
/// crate does not enforce, recorded rather than ignored.
const KNOWN_NODE_SHAPE_PREDS: &[NamedNodeRef<'static>] = &[
    sh::PROPERTY,
    sh::TARGET_CLASS,
    sh::TARGET_NODE,
    sh::CLOSED,
    sh::IGNORED_PROPERTIES,
    sh::NAME,
    sh::DESCRIPTION,
];

fn walk_node_shape(graph: &Graph, shape: SubjectRef<'_>, depth: u32) -> FormSchema {
    let mut schema = FormSchema {
        title: literal_string(graph.object_for_subject_predicate(shape, sh::NAME)),
        description: literal_string(graph.object_for_subject_predicate(shape, sh::DESCRIPTION)),
        closed: graph
            .object_for_subject_predicate(shape, sh::CLOSED)
            .is_some_and(is_true),
        ..Default::default()
    };
    record_unknown_predicates(
        graph,
        shape,
        KNOWN_NODE_SHAPE_PREDS,
        &mut schema.unsupported,
    );

    let mut fields: Vec<Field> = graph
        .objects_for_subject_predicate(shape, sh::PROPERTY)
        .filter_map(|prop| as_subject(prop))
        .map(|prop| walk_property_shape(graph, prop.as_ref(), depth))
        .collect();
    // `sh:order` first, absent order last (`order_key` maps `None` to
    // +infinity — the opposite of `Option`'s own derived ordering, which
    // would put an unordered field *before* every explicitly-ordered one).
    // SHACL itself does not define an order for `sh:property` values that
    // state no `sh:order` — and `oxrdf::Graph` is backed by an interned,
    // id-sorted store, NOT insertion order, so there is no "declaration
    // order" to fall back on even informally. Breaking remaining ties by
    // path/label is what makes an unordered shape's field order at least
    // deterministic across runs, rather than however term interning
    // happened to land this time.
    fields.sort_by(|a, b| {
        order_key(a.order)
            .total_cmp(&order_key(b.order))
            .then_with(|| tie_break_key(a).cmp(tie_break_key(b)))
    });
    schema.fields = fields;
    schema
}

/// Property shapes this crate reads a constraint from directly. Anything
/// else in `sh:` on the property node — `sh:equals`, `sh:disjoint`,
/// `sh:lessThan`, `sh:qualifiedValueShape`, `sh:sparql`, and so on — is
/// recorded on the field rather than silently unenforced-and-unmentioned.
const KNOWN_PROPERTY_SHAPE_PREDS: &[NamedNodeRef<'static>] = &[
    sh::PATH,
    sh::NAME,
    sh::DESCRIPTION,
    sh::ORDER,
    sh::DATATYPE,
    sh::CLASS,
    sh::NODE,
    sh::NODE_KIND,
    sh::MIN_COUNT,
    sh::MAX_COUNT,
    sh::MIN_LENGTH,
    sh::MAX_LENGTH,
    sh::PATTERN,
    sh::FLAGS,
    sh::MIN_INCLUSIVE,
    sh::MAX_INCLUSIVE,
    sh::MIN_EXCLUSIVE,
    sh::MAX_EXCLUSIVE,
    sh::IN,
    sh::HAS_VALUE,
    sh::DEFAULT_VALUE,
    sh::OR,
    sh::AND,
    sh::NOT,
    sh::XONE,
];

/// Everything this crate extracted from one property shape's own
/// constraints, before `resolve_kind` turns it into one [`FieldKind`].
/// `sh:and` merges into one `Acc` (constraints on the same value can
/// legitimately combine); `sh:or`/`sh:xone` process only their first
/// branch (see the crate README's SHACL-coverage table) and note the rest.
#[derive(Default)]
struct Acc {
    datatype: Option<NamedNode>,
    class: Option<NamedNode>,
    node: Option<Term>,
    node_kind: Option<NamedNode>,
    pattern: Option<String>,
    min_length: Option<u32>,
    max_length: Option<u32>,
    min_inclusive: Option<f64>,
    max_inclusive: Option<f64>,
    min_exclusive: Option<f64>,
    max_exclusive: Option<f64>,
    in_list: Option<Vec<Term>>,
    has_value: Option<Term>,
    unsupported: Vec<String>,
}

fn walk_property_shape(graph: &Graph, prop: SubjectRef<'_>, depth: u32) -> Field {
    let path_term = graph.object_for_subject_predicate(prop, sh::PATH);
    let path = path_term.and_then(|p| match p {
        TermRef::NamedNode(n) => Some(n.into_owned()),
        _ => None,
    });
    let complex_path_reason = path_term
        .filter(|p| !p.is_named_node())
        .map(|p| complex_path_reason(graph, p));

    let label =
        literal_string(graph.object_for_subject_predicate(prop, sh::NAME)).unwrap_or_else(|| {
            path.as_ref()
                .map(|p| local_name(p.as_str()).to_string())
                .unwrap_or_else(|| "value".to_string())
        });
    let description = literal_string(graph.object_for_subject_predicate(prop, sh::DESCRIPTION));
    let order = graph
        .object_for_subject_predicate(prop, sh::ORDER)
        .and_then(literal_f64);
    let min_count = graph
        .object_for_subject_predicate(prop, sh::MIN_COUNT)
        .and_then(literal_u32)
        .unwrap_or(0);
    let max_count = graph
        .object_for_subject_predicate(prop, sh::MAX_COUNT)
        .and_then(literal_u32);
    let default_value = graph
        .object_for_subject_predicate(prop, sh::DEFAULT_VALUE)
        .map(TermRef::into_owned);

    let mut acc = Acc::default();
    if let Some(reason) = complex_path_reason {
        acc.unsupported.push(format!(
            "sh:path is {reason}, not a single predicate — this field cannot be edited"
        ));
    }
    collect_constraints(graph, prop, &mut acc);

    let mut unsupported = acc.unsupported.clone();
    record_unknown_predicates(graph, prop, KNOWN_PROPERTY_SHAPE_PREDS, &mut unsupported);

    let original_datatype = acc.datatype.clone();
    let kind = resolve_kind(graph, &acc, depth, &mut unsupported);

    Field {
        path,
        label,
        description,
        order,
        min_count,
        max_count,
        kind,
        original_datatype,
        default_value,
        unsupported: (!unsupported.is_empty()).then(|| unsupported.join("; ")),
    }
}

/// Reads the simple (non-combinator) constraints directly on `node` into
/// `acc`, then recurses into `sh:and`/`sh:or`/`sh:xone`/`sh:not` found on
/// the same node. Called once for the property shape itself, and again
/// (accumulating into the *same* `acc`) for each `sh:and` branch — which is
/// what lets `sh:and ( [ sh:datatype xsd:string ] [ sh:pattern "^a" ] )`
/// merge into one `Text` field instead of two competing ones.
fn collect_constraints(graph: &Graph, node: SubjectRef<'_>, acc: &mut Acc) {
    if let Some(TermRef::NamedNode(dt)) = graph.object_for_subject_predicate(node, sh::DATATYPE) {
        acc.datatype = Some(dt.into_owned());
    }
    if let Some(TermRef::NamedNode(c)) = graph.object_for_subject_predicate(node, sh::CLASS) {
        acc.class = Some(c.into_owned());
    }
    if let Some(n) = graph.object_for_subject_predicate(node, sh::NODE) {
        acc.node = Some(n.into_owned());
    }
    if let Some(TermRef::NamedNode(nk)) = graph.object_for_subject_predicate(node, sh::NODE_KIND) {
        acc.node_kind = Some(nk.into_owned());
    }
    if let Some(p) = literal_string(graph.object_for_subject_predicate(node, sh::PATTERN)) {
        acc.pattern = Some(p);
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MIN_LENGTH)
        .and_then(literal_u32)
    {
        acc.min_length = Some(n);
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MAX_LENGTH)
        .and_then(literal_u32)
    {
        acc.max_length = Some(n);
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MIN_INCLUSIVE)
        .and_then(literal_f64)
    {
        acc.min_inclusive = Some(n);
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MAX_INCLUSIVE)
        .and_then(literal_f64)
    {
        acc.max_inclusive = Some(n);
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MIN_EXCLUSIVE)
        .and_then(literal_f64)
    {
        acc.min_exclusive = Some(n);
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MAX_EXCLUSIVE)
        .and_then(literal_f64)
    {
        acc.max_exclusive = Some(n);
    }
    if let Some(head) = graph.object_for_subject_predicate(node, sh::IN) {
        acc.in_list = Some(rdf_list(graph, head));
    }
    if let Some(v) = graph.object_for_subject_predicate(node, sh::HAS_VALUE) {
        acc.has_value = Some(v.into_owned());
    }

    if let Some(head) = graph.object_for_subject_predicate(node, sh::AND) {
        for branch in rdf_list(graph, head) {
            if let Some(s) = as_subject_term(&branch) {
                collect_constraints(graph, s.as_ref(), acc);
            }
        }
    }
    for (pred, combinator) in [(sh::OR, "sh:or"), (sh::XONE, "sh:xone")] {
        if let Some(head) = graph.object_for_subject_predicate(node, pred) {
            let branches = rdf_list(graph, head);
            if let Some(first) = branches.first().and_then(as_subject_term) {
                collect_constraints(graph, first.as_ref(), acc);
            }
            if branches.len() > 1 {
                acc.unsupported.push(format!(
                    "{combinator} has {} branches; only the first is reflected in this field — see the shape source for the rest",
                    branches.len()
                ));
            }
        }
    }
    if graph.object_for_subject_predicate(node, sh::NOT).is_some() {
        acc.unsupported
            .push("sh:not (a negative constraint) is not enforced by this field".to_string());
    }
}

fn resolve_kind(graph: &Graph, acc: &Acc, depth: u32, unsupported: &mut Vec<String>) -> FieldKind {
    if let Some(node) = &acc.node {
        return nest(graph, node, depth, unsupported).unwrap_or_else(|| {
            fallback_iri(
                unsupported,
                format!("sh:node <{node}> could not be expanded"),
            )
        });
    }
    if let Some(class) = &acc.class {
        if let Some(shape) = find_node_shape_for_class(graph, class.as_ref()) {
            let shape_term = subject_to_term(&shape);
            if let Some(k) = nest(graph, &shape_term, depth, unsupported) {
                return k;
            }
        }
        unsupported.push(format!(
            "no sh:NodeShape has sh:targetClass <{}>; rendered as a plain IRI field",
            class.as_str()
        ));
        return FieldKind::Iri;
    }
    if let Some(list) = &acc.in_list {
        return FieldKind::Select {
            options: list.iter().map(term_option).collect(),
        };
    }
    if let Some(v) = &acc.has_value {
        return FieldKind::Select {
            options: vec![term_option(v)],
        };
    }
    if let Some(nk) = &acc.node_kind {
        let r = nk.as_ref();
        if r == sh::IRI {
            return FieldKind::Iri;
        }
        if r == sh::LITERAL {
            // The expected/common case alongside sh:datatype — nothing to add.
        } else if r == sh::BLANK_NODE {
            unsupported.push("sh:nodeKind sh:BlankNode: a value with no human-meaningful identity — rendered as free text".to_string());
        } else if r == sh::IRI_OR_LITERAL
            || r == sh::BLANK_NODE_OR_IRI
            || r == sh::BLANK_NODE_OR_LITERAL
        {
            unsupported.push(format!(
                "sh:nodeKind {} allows more than one kind of value; this form only offers free text",
                local_name(nk.as_str())
            ));
        }
    }
    if let Some(dt) = &acc.datatype {
        let base = kind_for_datatype(dt).unwrap_or_else(|| {
            unsupported.push(format!(
                "unrecognised sh:datatype <{}>; rendered as free text",
                dt.as_str()
            ));
            FieldKind::Text {
                pattern: None,
                min_length: None,
                max_length: None,
            }
        });
        // Exactly one of these actually changes `base`: whichever matches
        // its own kind. Chained rather than branched on `dt` a second time,
        // so a datatype this crate maps to `Text` still gets its
        // pattern/length constraints and one mapped to `Number` still gets
        // its inclusive/exclusive bounds, without restating the datatype
        // dispatch here too.
        return apply_text_constraints(acc, apply_numeric_bounds(acc, base));
    }
    if acc.min_inclusive.is_some()
        || acc.max_inclusive.is_some()
        || acc.min_exclusive.is_some()
        || acc.max_exclusive.is_some()
    {
        return apply_numeric_bounds(
            acc,
            FieldKind::Number {
                integer_only: false,
                min: None,
                max: None,
            },
        );
    }
    apply_text_constraints(
        acc,
        FieldKind::Text {
            pattern: None,
            min_length: None,
            max_length: None,
        },
    )
}

fn nest(
    graph: &Graph,
    node: &Term,
    depth: u32,
    unsupported: &mut Vec<String>,
) -> Option<FieldKind> {
    if depth >= MAX_NESTING_DEPTH {
        unsupported.push(format!("nesting stopped at depth {MAX_NESTING_DEPTH}"));
        return None;
    }
    let subject = as_subject_term(node)?;
    let nested = walk_node_shape(graph, subject.as_ref(), depth + 1);
    Some(FieldKind::Nested {
        schema: Box::new(nested),
    })
}

fn fallback_iri(unsupported: &mut Vec<String>, reason: String) -> FieldKind {
    unsupported.push(reason);
    FieldKind::Iri
}

fn apply_numeric_bounds(acc: &Acc, kind: FieldKind) -> FieldKind {
    match kind {
        FieldKind::Number { integer_only, .. } => {
            let min = match (acc.min_inclusive, acc.min_exclusive) {
                (Some(v), _) => Some(v),
                (None, Some(v)) => Some(v),
                (None, None) => None,
            };
            let max = match (acc.max_inclusive, acc.max_exclusive) {
                (Some(v), _) => Some(v),
                (None, Some(v)) => Some(v),
                (None, None) => None,
            };
            FieldKind::Number {
                integer_only,
                min,
                max,
            }
        }
        other => other,
    }
}

fn apply_text_constraints(acc: &Acc, kind: FieldKind) -> FieldKind {
    match kind {
        FieldKind::Text { .. } => FieldKind::Text {
            pattern: acc.pattern.clone(),
            min_length: acc.min_length,
            max_length: acc.max_length,
        },
        other => other,
    }
}

fn kind_for_datatype(dt: &NamedNode) -> Option<FieldKind> {
    let r = dt.as_ref();
    let text = || FieldKind::Text {
        pattern: None,
        min_length: None,
        max_length: None,
    };
    if [
        xsd::STRING,
        xsd::NORMALIZED_STRING,
        xsd::TOKEN,
        xsd::LANGUAGE,
        xsd::NAME,
        xsd::NC_NAME,
        xsd::NMTOKEN,
    ]
    .contains(&r)
    {
        return Some(text());
    }
    if r == xsd::BOOLEAN {
        return Some(FieldKind::Boolean);
    }
    if r == xsd::DATE {
        return Some(FieldKind::Date);
    }
    if r == xsd::DATE_TIME || r == xsd::DATE_TIME_STAMP {
        return Some(FieldKind::DateTime);
    }
    if r == xsd::ANY_URI {
        return Some(FieldKind::Iri);
    }
    let integers = [
        xsd::INTEGER,
        xsd::INT,
        xsd::LONG,
        xsd::SHORT,
        xsd::BYTE,
        xsd::NON_NEGATIVE_INTEGER,
        xsd::NON_POSITIVE_INTEGER,
        xsd::NEGATIVE_INTEGER,
        xsd::POSITIVE_INTEGER,
        xsd::UNSIGNED_LONG,
        xsd::UNSIGNED_INT,
        xsd::UNSIGNED_SHORT,
        xsd::UNSIGNED_BYTE,
    ];
    if integers.contains(&r) {
        return Some(FieldKind::Number {
            integer_only: true,
            min: None,
            max: None,
        });
    }
    if [xsd::DECIMAL, xsd::DOUBLE, xsd::FLOAT].contains(&r) {
        return Some(FieldKind::Number {
            integer_only: false,
            min: None,
            max: None,
        });
    }
    None
}

fn term_option(term: &Term) -> SelectOption {
    let label = match term {
        Term::NamedNode(n) => local_name(n.as_str()).to_string(),
        Term::Literal(l) => l.value().to_string(),
        Term::BlankNode(b) => format!("_:{}", b.as_str()),
    };
    SelectOption {
        value: term.clone(),
        label,
    }
}

fn record_unknown_predicates(
    graph: &Graph,
    subject: SubjectRef<'_>,
    known: &[NamedNodeRef<'static>],
    out: &mut Vec<String>,
) {
    const SH_NS: &str = "http://www.w3.org/ns/shacl#";
    let mut seen = HashSet::new();
    for triple in graph.triples_for_subject(subject) {
        let pred = triple.predicate;
        if pred.as_str().starts_with(SH_NS)
            && !known.contains(&pred)
            && seen.insert(pred.as_str().to_string())
        {
            out.push(format!(
                "unrecognised constraint {}",
                local_name(pred.as_str())
            ));
        }
    }
}

fn rdf_list(graph: &Graph, head: TermRef<'_>) -> Vec<Term> {
    let mut out = Vec::new();
    let mut current = head.into_owned();
    loop {
        if matches!(&current, Term::NamedNode(n) if n.as_ref() == rdf::NIL) {
            break;
        }
        let Some(subject) = as_subject_term(&current) else {
            break;
        };
        let Some(first) = graph.object_for_subject_predicate(subject.as_ref(), rdf::FIRST) else {
            break;
        };
        out.push(first.into_owned());
        let Some(rest) = graph.object_for_subject_predicate(subject.as_ref(), rdf::REST) else {
            break;
        };
        current = rest.into_owned();
    }
    out
}

fn as_subject(term: TermRef<'_>) -> Option<Subject> {
    as_subject_term(&term.into_owned())
}

fn as_subject_term(term: &Term) -> Option<Subject> {
    match term {
        Term::NamedNode(n) => Some(Subject::NamedNode(n.clone())),
        Term::BlankNode(b) => Some(Subject::BlankNode(b.clone())),
        _ => None,
    }
}

fn subject_to_term(subject: &Subject) -> Term {
    match subject {
        Subject::NamedNode(n) => Term::NamedNode(n.clone()),
        Subject::BlankNode(b) => Term::BlankNode(b.clone()),
    }
}

fn shape_key(subject: SubjectRef<'_>) -> String {
    match subject {
        SubjectRef::NamedNode(n) => n.as_str().to_string(),
        SubjectRef::BlankNode(b) => format!("_:{}", b.as_str()),
    }
}

fn literal_string(term: Option<TermRef<'_>>) -> Option<String> {
    match term {
        Some(TermRef::Literal(l)) => Some(l.value().to_string()),
        _ => None,
    }
}

fn literal_f64(term: TermRef<'_>) -> Option<f64> {
    match term {
        TermRef::Literal(l) => l.value().parse().ok(),
        _ => None,
    }
}

fn literal_u32(term: TermRef<'_>) -> Option<u32> {
    match term {
        TermRef::Literal(l) => l.value().parse().ok(),
        _ => None,
    }
}

fn is_true(term: TermRef<'_>) -> bool {
    matches!(term, TermRef::Literal(l) if l.value() == "true" || l.value() == "1")
}

/// `sh:path`'s value was a blank node rather than a plain IRI — one of
/// SHACL's five path combinators. Named precisely rather than lumped into
/// one "complex path" message, since a reader deciding whether this is
/// worth hand-editing the shape over needs to know which one.
fn complex_path_reason(graph: &Graph, path: TermRef<'_>) -> &'static str {
    let Some(subject) = as_subject(path) else {
        return "a sequence path (an rdf:List)";
    };
    let has = |pred| {
        graph
            .object_for_subject_predicate(subject.as_ref(), pred)
            .is_some()
    };
    if has(sh::INVERSE_PATH) {
        "an inverse path (sh:inversePath)"
    } else if has(sh::ALTERNATIVE_PATH) {
        "an alternative path (sh:alternativePath)"
    } else if has(sh::ZERO_OR_MORE_PATH) {
        "a zero-or-more path (sh:zeroOrMorePath)"
    } else if has(sh::ONE_OR_MORE_PATH) {
        "a one-or-more path (sh:oneOrMorePath)"
    } else if has(sh::ZERO_OR_ONE_PATH) {
        "a zero-or-one path (sh:zeroOrOnePath)"
    } else {
        "a sequence path (an rdf:List)"
    }
}

/// `None` (no `sh:order`) sorts after every stated order, not before —
/// `Option<f64>`'s own derived `PartialOrd` puts `None` first, which is the
/// wrong default for "where do unordered fields land in a form".
fn order_key(order: Option<f64>) -> f64 {
    order.unwrap_or(f64::INFINITY)
}

fn tie_break_key(field: &Field) -> &str {
    field
        .path
        .as_ref()
        .map(NamedNode::as_str)
        .unwrap_or(&field.label)
}

fn local_name(iri: &str) -> &str {
    iri.rsplit(['#', '/'])
        .next()
        .filter(|s| !s.is_empty())
        .unwrap_or(iri)
}
