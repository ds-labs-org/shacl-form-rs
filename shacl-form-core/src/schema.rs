//! Walks a parsed shapes graph and produces a [`FormSchema`]. This is the
//! one module that knows what a `sh:NodeShape`/`sh:PropertyShape` *means*;
//! [`crate::model`] only knows what a form looks like once this module is
//! done deciding.
use crate::error::ShapesError;
use crate::model::{Field, FieldKind, FormSchema, SelectOption};
use crate::sh;
use oxrdf::vocab::{rdf, xsd};
use oxrdf::{Graph, NamedNode, NamedNodeRef, Subject, SubjectRef, Term, TermRef, TripleRef};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

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

/// How many `sh:and`/`sh:or`/`sh:xone` levels [`collect_constraints`] will
/// recurse through on one property before it stops. Unlike shape nesting,
/// this has no legitimate reason to go deep in a real shapes graph — it
/// exists only so a pathological or malicious shape (a property shape whose
/// own `sh:and` list contains itself, directly or through a longer cycle)
/// gets an "unsupported" note instead of a stack overflow, which on wasm is
/// not a catchable panic at all.
const MAX_COMBINATOR_DEPTH: u32 = 32;

/// Every distinct `(expanded shape set, target class, nesting depth)` this
/// crate has already turned into a schema, so that reaching the same shape
/// twice — the same `sh:node`/`sh:class` referenced from two different
/// properties, or a shape nested inside itself several times over — shares
/// one `Rc<FormSchema>` instead of re-walking the graph and re-allocating an
/// identical tree each time. Without this, a shape with `k` occurrences of
/// a mutually-self-referencing pair expands to a tree with roughly
/// `k^MAX_NESTING_DEPTH` fields.
type ShapeCache = HashMap<(Vec<String>, Option<String>, u32), Rc<FormSchema>>;

pub fn from_shape_iri(graph: &Graph, shape_iri: &NamedNode) -> Result<FormSchema, ShapesError> {
    let subject = Subject::NamedNode(shape_iri.clone());
    if !graph.contains(TripleRef::new(subject.as_ref(), rdf::TYPE, sh::NODE_SHAPE)) {
        return Err(ShapesError::NotANodeShape(shape_iri.as_str().to_string()));
    }
    let target_class = shape_own_target_class(graph, &subject);
    let mut cache = ShapeCache::new();
    let rc = walk_shapes(graph, vec![subject], target_class, 0, &mut cache);
    Ok((*rc).clone())
}

/// SHACL lets more than one `sh:NodeShape` target the same class — this
/// repository's own `vendor/ds-honeycomb-editor-rs/shapes.ttl` does exactly
/// that (`hive:Diagram` is targeted by `hsh:DiagramShape`, which has
/// `sh:property`, *and* by three SPARQL-only shapes with none) — so this
/// merges every matching shape's `sh:property` list, and every shape's own
/// `sh:node`/`sh:and` node-level references, before grouping by path and
/// building fields — the same machinery `resolve_kind`'s `sh:class` branch
/// uses for a *nested* multi-shape reference. Two property shapes across
/// that merged set sharing one `sh:path` become one field, not two
/// (`walk_shapes`'s grouping step), the way two `sh:and` branches on one
/// property shape already merged into one field.
pub fn from_target_class(graph: &Graph, class_iri: &NamedNode) -> Result<FormSchema, ShapesError> {
    let shapes = find_node_shapes_for_class(graph, class_iri.as_ref());
    if shapes.is_empty() {
        return Err(ShapesError::NoShapeForClass(class_iri.as_str().to_string()));
    }
    let mut cache = ShapeCache::new();
    let rc = walk_shapes(graph, shapes, Some(class_iri.clone()), 0, &mut cache);
    Ok((*rc).clone())
}

/// Every subject asserting `sh:targetClass <class>` — deliberately NOT
/// filtered to ones also carrying an explicit `a sh:NodeShape` triple.
/// SHACL recognises a node shape structurally (stating `sh:targetClass`,
/// `sh:property`, or any other shape-defining predicate already makes a
/// subject a shape); requiring the type triple on top of that made a
/// perfectly real, spec-legal shape invisible to `sh:class` resolution
/// just because its author omitted an optional assertion. `from_shape_iri`
/// keeps its own explicit check — a caller naming a shape directly by IRI
/// is a different, narrower question ("is *this* the shape I mean")
/// than "which shapes, however stated, target this class".
fn find_node_shapes_for_class(graph: &Graph, class: NamedNodeRef<'_>) -> Vec<Subject> {
    let mut shapes: Vec<Subject> = graph
        .subjects_for_predicate_object(sh::TARGET_CLASS, class)
        .map(SubjectRef::into_owned)
        .collect();
    // `oxrdf::Graph` iterates in whatever order its internal set happens to
    // hold triples, which is not necessarily insertion order and is not
    // guaranteed stable across versions — sorting by the shape's own IRI
    // (or blank node id) is what makes the merged field order deterministic
    // between runs, not just "whatever HashSet handed back this time".
    shapes.sort_by_key(shape_key_owned);
    shapes
}

fn shape_key_owned(subject: &Subject) -> String {
    shape_key(subject.as_ref())
}

/// Reads `sh:targetClass` directly off `shape`, when it states one — the
/// same lookup `from_shape_iri` does for the root shape, reused by
/// `resolve_kind`'s `sh:node` branch so the identical shape gets the same
/// `target_class` (and so the same `rdf:type` assertion on submit)
/// whether it's used as the form's own root or reached as a nested value.
fn shape_own_target_class(graph: &Graph, shape: &Subject) -> Option<NamedNode> {
    match graph.object_for_subject_predicate(shape.as_ref(), sh::TARGET_CLASS) {
        Some(TermRef::NamedNode(c)) => Some(c.into_owned()),
        _ => None,
    }
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
    sh::NODE,
    sh::AND,
    sh::MESSAGE,
    sh::SEVERITY,
    sh::DEACTIVATED,
];

/// Expands `seed` (node shapes a form is being built directly for) through
/// node-shape-level `sh:node` (a single shape reference: "conforms to this
/// shape too") and `sh:and` (a list of shape references: the common
/// SHACL "shape inheritance" idiom, `ex:EmployeeShape sh:and (ex:PersonShape
/// ex:StaffShape)`), so an inherited shape's own `sh:property` list is
/// collected right alongside the seed's — the same way `from_target_class`
/// already merges *sibling* shapes that target one class. Has its own
/// visited set and terminates unconditionally, independent of the
/// `ShapeCache`/nesting-depth machinery in [`walk_shapes`]: a self- or
/// mutually-referencing pair of shapes here (`A sh:and (B)`, `B sh:and (A)`)
/// is a property-list cycle, not a value-nesting one, and would not be
/// caught by [`MAX_NESTING_DEPTH`] at all.
fn expand_shape_refs(graph: &Graph, seed: Vec<Subject>) -> Vec<Subject> {
    let mut visited: HashSet<String> = HashSet::new();
    let mut queue = seed;
    let mut result = Vec::new();
    while let Some(s) = queue.pop() {
        if !visited.insert(shape_key(s.as_ref())) {
            continue;
        }
        result.push(s.clone());
        // A deactivated shape contributes nothing — including what it
        // would otherwise have inherited. Its own sh:property list is
        // already excluded later (walk_shapes filters `expanded` on this
        // same predicate), but without this check its sh:node/sh:and
        // targets were still followed and merged in: switching a shape off
        // stopped its own properties from showing up, but not the
        // properties it names as inherited on its behalf.
        if graph
            .object_for_subject_predicate(s.as_ref(), sh::DEACTIVATED)
            .is_some_and(is_true)
        {
            continue;
        }
        if let Some(n) = graph.object_for_subject_predicate(s.as_ref(), sh::NODE)
            && let Some(sub) = as_subject_term(&n.into_owned())
        {
            queue.push(sub);
        }
        if let Some(head) = graph.object_for_subject_predicate(s.as_ref(), sh::AND) {
            for branch in rdf_list(graph, head) {
                if let Some(sub) = as_subject_term(&branch) {
                    queue.push(sub);
                }
            }
        }
    }
    result
}

/// Builds (or returns the cached) [`FormSchema`] for one or more node
/// shapes taken together — `from_shape_iri`'s single seed shape,
/// `from_target_class`'s sibling shapes sharing a `sh:targetClass`, or
/// `resolve_kind`'s nested `sh:node`/`sh:class` reference, all funnelled
/// through the same path: expand shape-level inheritance
/// ([`expand_shape_refs`]), skip anything `sh:deactivated`, flatten every
/// remaining shape's `sh:property` list, group by resolved path, and build
/// one [`Field`] per group ([`build_field`]) — so two property shapes
/// sharing a path, whether from the same node shape or two shapes in the
/// merged set, become one field instead of a silent duplicate.
fn walk_shapes(
    graph: &Graph,
    shapes: Vec<Subject>,
    target_class: Option<NamedNode>,
    depth: u32,
    cache: &mut ShapeCache,
) -> Rc<FormSchema> {
    let mut expanded: Vec<Subject> = expand_shape_refs(graph, shapes)
        .into_iter()
        .filter(|s| {
            !graph
                .object_for_subject_predicate(s.as_ref(), sh::DEACTIVATED)
                .is_some_and(is_true)
        })
        .collect();
    expanded.sort_by_key(shape_key_owned);
    expanded.dedup_by_key(|s| shape_key_owned(s));

    let keys: Vec<String> = expanded.iter().map(|s| shape_key(s.as_ref())).collect();
    let cache_key = (
        keys,
        target_class.as_ref().map(|c| c.as_str().to_string()),
        depth,
    );
    if let Some(cached) = cache.get(&cache_key) {
        return cached.clone();
    }

    let mut schema = FormSchema {
        target_class: target_class.clone(),
        ..Default::default()
    };
    for s in &expanded {
        if schema.title.is_none() {
            schema.title = literal_string(graph.object_for_subject_predicate(s.as_ref(), sh::NAME));
        }
        if schema.description.is_none() {
            schema.description =
                literal_string(graph.object_for_subject_predicate(s.as_ref(), sh::DESCRIPTION));
        }
        if graph
            .object_for_subject_predicate(s.as_ref(), sh::CLOSED)
            .is_some_and(is_true)
        {
            schema.closed = true;
        }
        record_unknown_predicates(
            graph,
            s.as_ref(),
            KNOWN_NODE_SHAPE_PREDS,
            &mut schema.unsupported,
        );
    }

    // Flatten every expanded shape's sh:property list, dropping deactivated
    // property shapes, then group by resolved path — a complex (blank-node)
    // path gets its own singleton group rather than merging with anything,
    // since "the same complex path" is not a concept this crate resolves.
    let mut order_seen: Vec<String> = Vec::new();
    let mut groups: HashMap<String, Vec<Subject>> = HashMap::new();
    let mut singleton = 0usize;
    for s in &expanded {
        for prop in graph.objects_for_subject_predicate(s.as_ref(), sh::PROPERTY) {
            let Some(prop_subject) = as_subject(prop) else {
                continue;
            };
            if graph
                .object_for_subject_predicate(prop_subject.as_ref(), sh::DEACTIVATED)
                .is_some_and(is_true)
            {
                continue;
            }
            let key = match resolved_path_key(graph, prop_subject.as_ref()) {
                Some(k) => k,
                None => {
                    singleton += 1;
                    format!(
                        "\u{0}complex-path-{}-{singleton}",
                        shape_key(prop_subject.as_ref())
                    )
                }
            };
            if !groups.contains_key(&key) {
                order_seen.push(key.clone());
            }
            groups.entry(key).or_default().push(prop_subject);
        }
    }

    let mut fields: Vec<Field> = order_seen
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .map(|members| build_field(graph, &members, depth, cache))
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

    let rc = Rc::new(schema);
    cache.insert(cache_key, rc.clone());
    rc
}

fn resolved_path_key(graph: &Graph, prop: SubjectRef<'_>) -> Option<String> {
    match graph.object_for_subject_predicate(prop, sh::PATH) {
        Some(TermRef::NamedNode(n)) => Some(n.as_str().to_string()),
        _ => None,
    }
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
    sh::MESSAGE,
    sh::SEVERITY,
    sh::GROUP,
    sh::DEACTIVATED,
];

/// Everything this crate extracted from one *group* of property shapes
/// (ordinarily one, but see [`walk_shapes`]'s path-based grouping) sharing
/// a path, before `resolve_kind` turns it into one [`FieldKind`]. `sh:and`
/// — and now every member of a merged group — accumulates into one `Acc`
/// by tightening/merging rather than overwriting (a later `sh:maxLength 5`
/// no longer silently loosens an earlier `sh:maxLength 50`); `sh:or`/
/// `sh:xone` process only their first branch (see the crate README's
/// SHACL-coverage table) and note the rest.
#[derive(Default)]
struct Acc {
    datatype: Option<NamedNode>,
    class: Option<NamedNode>,
    node: Option<Term>,
    node_kind: Option<NamedNode>,
    patterns: Vec<String>,
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

/// One field for `members` (all sharing one resolved path, or a single
/// complex-path property shape in a singleton group of its own — see
/// [`walk_shapes`]). Name/description/order/default value come from the
/// first member that states one; cardinality merges by tightening
/// (`min_count` = the loosest member's floor is not enough, every member's
/// own minimum must hold, so the max across members; `max_count` = the
/// tightest ceiling any member states, so the min of the stated ones);
/// every other constraint accumulates into one [`Acc`] the same way
/// `sh:and`'s branches already did.
fn build_field(graph: &Graph, members: &[Subject], depth: u32, cache: &mut ShapeCache) -> Field {
    let path_term = graph.object_for_subject_predicate(members[0].as_ref(), sh::PATH);
    let path = path_term.and_then(|p| match p {
        TermRef::NamedNode(n) => Some(n.into_owned()),
        _ => None,
    });
    let mut unsupported = Vec::new();
    if let Some(reason) = path_term
        .filter(|p| !p.is_named_node())
        .map(|p| complex_path_reason(graph, p))
    {
        unsupported.push(format!(
            "sh:path is {reason}, not a single predicate — this field cannot be edited"
        ));
    }

    let mut label = None;
    let mut description = None;
    let mut order = None;
    let mut default_value = None;
    let mut min_count = 0u32;
    let mut max_count: Option<u32> = None;
    let mut acc = Acc::default();

    for member in members {
        let member = member.as_ref();
        if label.is_none() {
            label = literal_string(graph.object_for_subject_predicate(member, sh::NAME));
        }
        if description.is_none() {
            description =
                literal_string(graph.object_for_subject_predicate(member, sh::DESCRIPTION));
        }
        if order.is_none() {
            order = graph
                .object_for_subject_predicate(member, sh::ORDER)
                .and_then(literal_f64);
        }
        if default_value.is_none() {
            default_value = graph
                .object_for_subject_predicate(member, sh::DEFAULT_VALUE)
                .map(TermRef::into_owned);
        }
        min_count = min_count.max(
            graph
                .object_for_subject_predicate(member, sh::MIN_COUNT)
                .and_then(literal_u32)
                .unwrap_or(0),
        );
        if let Some(this_max) = graph
            .object_for_subject_predicate(member, sh::MAX_COUNT)
            .and_then(literal_u32)
        {
            max_count = Some(max_count.map_or(this_max, |m| m.min(this_max)));
        }
        record_unknown_predicates(graph, member, KNOWN_PROPERTY_SHAPE_PREDS, &mut unsupported);
        collect_constraints(graph, member, &mut acc, 0);
    }

    let label = label.unwrap_or_else(|| {
        path.as_ref()
            .map(|p| local_name(p.as_str()).to_string())
            .unwrap_or_else(|| "value".to_string())
    });
    unsupported.extend(acc.unsupported.clone());

    let original_datatype = acc.datatype.clone();
    let kind = resolve_kind(graph, &acc, depth, cache, &mut unsupported);
    // sh:pattern/sh:minLength/sh:maxLength apply to a Text or an Iri
    // control (both are, structurally, "a lexical form with syntax
    // rules") — every other kind (Number, Boolean, Date, DateTime, Select,
    // Nested) has no rendered attribute for them at all, and used to drop
    // them with no trace whenever the field's kind was decided by
    // something else (sh:datatype xsd:integer, sh:in, sh:class, ...).
    // Checked once here, after `kind` is fully decided, rather than at each
    // of `resolve_kind`'s several early-return sites.
    if !matches!(kind, FieldKind::Text { .. } | FieldKind::Iri { .. })
        && (!acc.patterns.is_empty() || acc.min_length.is_some() || acc.max_length.is_some())
    {
        unsupported.push(
            "sh:pattern/sh:minLength/sh:maxLength are stated but this field's kind has no control to apply them to — not enforced"
                .to_string(),
        );
    }

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
/// `acc` — merging with whatever `acc` already holds (from an earlier
/// member of the same field group, or an enclosing `sh:and`) rather than
/// overwriting it — then recurses into `sh:and`/`sh:or`/`sh:xone`/`sh:not`
/// found on the same node. `combinator_depth` bounds that recursion (see
/// [`MAX_COMBINATOR_DEPTH`]); it has nothing to do with [`MAX_NESTING_DEPTH`],
/// which bounds `sh:node`/`sh:class` value-nesting instead.
fn collect_constraints(graph: &Graph, node: SubjectRef<'_>, acc: &mut Acc, combinator_depth: u32) {
    if combinator_depth > MAX_COMBINATOR_DEPTH {
        acc.unsupported.push(
            "constraint combinators (sh:and/sh:or/sh:xone) nested too deep; stopped".to_string(),
        );
        return;
    }

    set_or_report_conflict(
        &mut acc.datatype,
        graph
            .object_for_subject_predicate(node, sh::DATATYPE)
            .and_then(as_named_node),
        "sh:datatype",
        &mut acc.unsupported,
    );
    set_or_report_conflict(
        &mut acc.class,
        graph
            .object_for_subject_predicate(node, sh::CLASS)
            .and_then(as_named_node),
        "sh:class",
        &mut acc.unsupported,
    );
    set_or_report_conflict_term(
        &mut acc.node,
        graph
            .object_for_subject_predicate(node, sh::NODE)
            .map(TermRef::into_owned),
        "sh:node",
        &mut acc.unsupported,
    );
    set_or_report_conflict(
        &mut acc.node_kind,
        graph
            .object_for_subject_predicate(node, sh::NODE_KIND)
            .and_then(as_named_node),
        "sh:nodeKind",
        &mut acc.unsupported,
    );

    // `objects_for_subject_predicate`, not `object_for_subject_predicate`:
    // a single property shape stating `sh:pattern` more than once (legal
    // RDF — a node can have several objects for one predicate) must have
    // every one of them enforced, the same as two different `sh:and`
    // branches each stating one already did.
    for p in graph.objects_for_subject_predicate(node, sh::PATTERN) {
        if let Some(p) = literal_string(Some(p)) {
            acc.patterns.push(p);
        }
    }
    if graph
        .object_for_subject_predicate(node, sh::FLAGS)
        .is_some()
    {
        acc.unsupported.push("sh:flags is stated but not applied to the rendered pattern (case-insensitive/other flag semantics are not translated to the HTML control)".to_string());
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MIN_LENGTH)
        .and_then(literal_u32)
    {
        acc.min_length = Some(acc.min_length.map_or(n, |e| e.max(n)));
    }
    if let Some(n) = graph
        .object_for_subject_predicate(node, sh::MAX_LENGTH)
        .and_then(literal_u32)
    {
        acc.max_length = Some(acc.max_length.map_or(n, |e| e.min(n)));
    }
    read_numeric_bound(
        graph,
        node,
        sh::MIN_INCLUSIVE,
        "sh:minInclusive",
        &mut acc.min_inclusive,
        f64::max,
        &mut acc.unsupported,
    );
    read_numeric_bound(
        graph,
        node,
        sh::MAX_INCLUSIVE,
        "sh:maxInclusive",
        &mut acc.max_inclusive,
        f64::min,
        &mut acc.unsupported,
    );
    read_numeric_bound(
        graph,
        node,
        sh::MIN_EXCLUSIVE,
        "sh:minExclusive",
        &mut acc.min_exclusive,
        f64::max,
        &mut acc.unsupported,
    );
    read_numeric_bound(
        graph,
        node,
        sh::MAX_EXCLUSIVE,
        "sh:maxExclusive",
        &mut acc.max_exclusive,
        f64::min,
        &mut acc.unsupported,
    );
    if let Some(head) = graph.object_for_subject_predicate(node, sh::IN) {
        let list = rdf_list(graph, head);
        match &acc.in_list {
            None => acc.in_list = Some(list),
            Some(existing) if existing == &list => {}
            Some(_) => acc
                .unsupported
                .push("sh:and combines conflicting sh:in lists; keeping the first".to_string()),
        }
    }
    let mut has_value_here = graph
        .objects_for_subject_predicate(node, sh::HAS_VALUE)
        .map(TermRef::into_owned);
    if let Some(v) = has_value_here.next() {
        // More than one sh:hasValue on ONE property shape means "must have
        // every one of these values" (a repeatable-property idiom), not
        // "any one of them" — this crate models sh:hasValue as a single
        // fixed Select option, so it can't honour "all of several required
        // values" at all. Reported rather than silently keeping an
        // arbitrary one, the same as sh:or's own "more branches than this
        // field can reflect" note.
        if has_value_here.next().is_some() {
            acc.unsupported.push("sh:hasValue states more than one required value on a single property shape; only one is reflected as this field's fixed option".to_string());
        }
        match &acc.has_value {
            None => acc.has_value = Some(v),
            Some(existing) if existing == &v => {}
            Some(_) => acc
                .unsupported
                .push("sh:and combines conflicting sh:hasValue; keeping the first".to_string()),
        }
    }

    if let Some(head) = graph.object_for_subject_predicate(node, sh::AND) {
        for branch in rdf_list(graph, head) {
            if let Some(s) = as_subject_term(&branch) {
                record_unknown_predicates(
                    graph,
                    s.as_ref(),
                    KNOWN_PROPERTY_SHAPE_PREDS,
                    &mut acc.unsupported,
                );
                collect_constraints(graph, s.as_ref(), acc, combinator_depth + 1);
            }
        }
    }
    for (pred, combinator) in [(sh::OR, "sh:or"), (sh::XONE, "sh:xone")] {
        if let Some(head) = graph.object_for_subject_predicate(node, pred) {
            let branches = rdf_list(graph, head);
            if let Some(first) = branches.first().and_then(as_subject_term) {
                record_unknown_predicates(
                    graph,
                    first.as_ref(),
                    KNOWN_PROPERTY_SHAPE_PREDS,
                    &mut acc.unsupported,
                );
                collect_constraints(graph, first.as_ref(), acc, combinator_depth + 1);
            }
            match branches.len() {
                0 => acc.unsupported.push(format!("{combinator} has no branches (an empty list) — nothing to enforce")),
                1 => {}
                n => acc.unsupported.push(format!("{combinator} has {n} branches; only the first is reflected in this field — see the shape source for the rest")),
            }
        }
    }
    if graph.object_for_subject_predicate(node, sh::NOT).is_some() {
        acc.unsupported
            .push("sh:not (a negative constraint) is not enforced by this field".to_string());
    }
}

fn set_or_report_conflict(
    slot: &mut Option<NamedNode>,
    new: Option<NamedNode>,
    label: &str,
    unsupported: &mut Vec<String>,
) {
    let Some(new) = new else { return };
    match slot {
        None => *slot = Some(new),
        Some(existing) if *existing == new => {}
        Some(existing) => unsupported.push(format!(
            "sh:and combines conflicting {label} (<{}> vs <{}>); keeping the first",
            existing.as_str(),
            new.as_str()
        )),
    }
}

fn set_or_report_conflict_term(
    slot: &mut Option<Term>,
    new: Option<Term>,
    label: &str,
    unsupported: &mut Vec<String>,
) {
    let Some(new) = new else { return };
    match slot {
        None => *slot = Some(new),
        Some(existing) if *existing == new => {}
        Some(_) => unsupported.push(format!(
            "sh:and combines conflicting {label}; keeping the first"
        )),
    }
}

fn as_named_node(term: TermRef<'_>) -> Option<NamedNode> {
    match term {
        TermRef::NamedNode(n) => Some(n.into_owned()),
        _ => None,
    }
}

fn resolve_kind(
    graph: &Graph,
    acc: &Acc,
    depth: u32,
    cache: &mut ShapeCache,
    unsupported: &mut Vec<String>,
) -> FieldKind {
    // sh:node/sh:class wins the field's own kind (a value that is itself
    // shaped is more useful nested than flattened into a fixed list), but
    // an enumeration stated alongside it is a real constraint this crate
    // then does not enforce at all — silently, until this note. Checked
    // before either nesting branch below, since both return early.
    if (acc.node.is_some() || acc.class.is_some())
        && (acc.in_list.is_some() || acc.has_value.is_some())
    {
        let which = match (acc.in_list.is_some(), acc.has_value.is_some()) {
            (true, true) => "sh:in and sh:hasValue are",
            (true, false) => "sh:in is",
            (false, true) => "sh:hasValue is",
            (false, false) => unreachable!(),
        };
        unsupported.push(format!(
            "{which} stated alongside sh:node/sh:class; nesting wins, {} not enforced",
            if acc.in_list.is_some() && acc.has_value.is_some() {
                "neither is"
            } else {
                "it is"
            }
        ));
    }
    if let Some(node) = &acc.node {
        return match as_subject_term(node) {
            Some(subject) => {
                // sh:node and sh:class stated together on one property is
                // legal but this crate can only nest via ONE shape
                // reference — sh:node's, since it names an exact shape
                // rather than "however many shapes happen to target this
                // class". sh:class's own class is not silently dropped for
                // that, though: it still names what the value must be an
                // instance of, so it's threaded through as the nested
                // schema's target_class (asserted as rdf:type on submit —
                // see FormValues::to_turtle) even though sh:node's shape is
                // what supplies the fields.
                if acc.class.is_some() {
                    unsupported.push(
                        "sh:node and sh:class are both stated on this property; nesting via sh:node's own shape, and still asserting sh:class's class on the value".to_string(),
                    );
                }
                let target_class = acc
                    .class
                    .clone()
                    .or_else(|| shape_own_target_class(graph, &subject));
                nest_shapes(
                    graph,
                    vec![subject],
                    target_class,
                    depth,
                    cache,
                    unsupported,
                )
            }
            None => fallback_iri(
                unsupported,
                format!("sh:node <{node}> is not a shape reference"),
            ),
        };
    }
    if let Some(class) = &acc.class {
        let shapes = find_node_shapes_for_class(graph, class.as_ref());
        if shapes.is_empty() {
            unsupported.push(format!(
                "no shape has sh:targetClass <{}>; rendered as a plain IRI field",
                class.as_str()
            ));
            return plain_iri();
        }
        return nest_shapes(
            graph,
            shapes,
            Some(class.clone()),
            depth,
            cache,
            unsupported,
        );
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
            // apply_text_constraints, not a bare FieldKind::Iri: an IRI's
            // own lexical form is exactly as valid a target for
            // sh:pattern/sh:minLength/sh:maxLength as a string literal's —
            // dropped here previously with no note at all.
            return apply_text_constraints(acc, plain_iri());
        }
        if r == sh::LITERAL {
            // The expected/common case alongside sh:datatype — nothing to add.
        } else if r == sh::BLANK_NODE {
            unsupported.push("sh:nodeKind sh:BlankNode: a value with no human-meaningful identity — rendered as free text".to_string());
        } else if r == sh::IRI_OR_LITERAL
            || r == sh::BLANK_NODE_OR_IRI
            || r == sh::BLANK_NODE_OR_LITERAL
        {
            unsupported.push(format!("sh:nodeKind {} allows more than one kind of value; this form only offers free text", local_name(nk.as_str())));
        }
    }
    if let Some(dt) = &acc.datatype {
        let base = kind_for_datatype(dt).unwrap_or_else(|| {
            unsupported.push(format!(
                "unrecognised sh:datatype <{}>; rendered as free text",
                dt.as_str()
            ));
            FieldKind::Text {
                patterns: Vec::new(),
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
            patterns: Vec::new(),
            min_length: None,
            max_length: None,
        },
    )
}

fn nest_shapes(
    graph: &Graph,
    shapes: Vec<Subject>,
    target_class: Option<NamedNode>,
    depth: u32,
    cache: &mut ShapeCache,
    unsupported: &mut Vec<String>,
) -> FieldKind {
    if depth >= MAX_NESTING_DEPTH {
        unsupported.push(format!("nesting stopped at depth {MAX_NESTING_DEPTH}"));
        return plain_iri();
    }
    FieldKind::Nested {
        schema: walk_shapes(graph, shapes, target_class, depth + 1, cache),
    }
}

fn fallback_iri(unsupported: &mut Vec<String>, reason: String) -> FieldKind {
    unsupported.push(reason);
    plain_iri()
}

fn plain_iri() -> FieldKind {
    FieldKind::Iri {
        patterns: Vec::new(),
        min_length: None,
        max_length: None,
    }
}

/// Turns SHACL's `sh:minExclusive`/`sh:maxExclusive` into the inclusive
/// bound HTML's `min`/`max` actually understand. Exact for an integer field
/// (`sh:minExclusive 0` becomes `min=1`, so 0 itself is correctly rejected);
/// approximated as the same value for a decimal/float field, where there is
/// no single "next representable value" to bump by — a known gap (see the
/// README), not silently pretended away: the bound is still tighter than
/// nothing, just not exact.
fn apply_numeric_bounds(acc: &Acc, kind: FieldKind) -> FieldKind {
    match kind {
        FieldKind::Number { integer_only, .. } => {
            let bump = if integer_only { 1.0 } else { 0.0 };
            let min = combine_bound(
                acc.min_inclusive,
                acc.min_exclusive.map(|v| v + bump),
                f64::max,
            );
            let max = combine_bound(
                acc.max_inclusive,
                acc.max_exclusive.map(|v| v - bump),
                f64::min,
            );
            FieldKind::Number {
                integer_only,
                min,
                max,
            }
        }
        other => other,
    }
}

fn combine_bound(a: Option<f64>, b: Option<f64>, tighten: impl Fn(f64, f64) -> f64) -> Option<f64> {
    match (a, b) {
        (Some(x), Some(y)) => Some(tighten(x, y)),
        (Some(x), None) => Some(x),
        (None, Some(y)) => Some(y),
        (None, None) => None,
    }
}

fn apply_text_constraints(acc: &Acc, kind: FieldKind) -> FieldKind {
    match kind {
        FieldKind::Text { .. } => FieldKind::Text {
            patterns: acc.patterns.clone(),
            min_length: acc.min_length,
            max_length: acc.max_length,
        },
        FieldKind::Iri { .. } => FieldKind::Iri {
            patterns: acc.patterns.clone(),
            min_length: acc.min_length,
            max_length: acc.max_length,
        },
        other => other,
    }
}

fn kind_for_datatype(dt: &NamedNode) -> Option<FieldKind> {
    let r = dt.as_ref();
    let text = || FieldKind::Text {
        patterns: Vec::new(),
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
    if r == rdf::LANG_STRING {
        // rdf:langString IS a recognised datatype (SKOS/DCAT shapes state
        // it routinely) — mapping it to the fallback "unrecognised"
        // Text control would falsely claim this crate doesn't know what it
        // is. It renders the same as any other Text field; what's actually
        // special about it (its lexical form requires a language tag,
        // which this crate's controls have no dedicated input for yet) is
        // handled at serialisation, in `values.rs`'s
        // `literal_entry_with_language`, not here.
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
        // xsd:anyURI is a LITERAL datatype ("http://a.example/"^^xsd:anyURI)
        // — sh:datatype only ever constrains what a literal looks like, per
        // SHACL/XSD, never whether the value is a literal at all. That's
        // FieldKind::Iri's job instead (sh:nodeKind sh:IRI, below): a
        // resource reference, no datatype possible. Mapping sh:datatype
        // xsd:anyURI to Iri wrote a bare IRI node, which does not conform
        // to the very sh:datatype constraint that produced the field.
        return Some(text());
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

/// Walks an `rdf:List` (`sh:in`'s value, `sh:and`/`sh:or`/`sh:xone`'s
/// branch list) into a `Vec`. Guards against a cyclic or self-referencing
/// list (`_:l rdf:first ex:a ; rdf:rest _:l`) with a visited set: without
/// one, such a list — malformed, but not something this crate should ever
/// trust a pasted shapes graph not to contain — grows `out` and loops
/// forever rather than erroring.
fn rdf_list(graph: &Graph, head: TermRef<'_>) -> Vec<Term> {
    let mut out = Vec::new();
    let mut current = head.into_owned();
    let mut visited = HashSet::new();
    loop {
        if matches!(&current, Term::NamedNode(n) if n.as_ref() == rdf::NIL) {
            break;
        }
        if !visited.insert(current.clone()) {
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

/// Reads one numeric-bound predicate (`sh:minInclusive` and friends) off
/// `node`, tightening `slot` the way `collect_constraints` already merges
/// every other constraint. A bound whose literal genuinely isn't numeric —
/// a legal pairing (`sh:minInclusive "2000-01-01"^^xsd:date` alongside
/// `xsd:date`, say) this crate's controls just don't apply yet — used to
/// vanish silently at this exact step (`literal_f64` returning `None` looks
/// identical to "no such triple at all"). Distinguished here: the predicate
/// being *present but unparsed* is reported, so a reader can at least tell
/// "this crate saw a constraint it couldn't use" from "there was nothing to
/// see".
#[allow(clippy::too_many_arguments)]
fn read_numeric_bound(
    graph: &Graph,
    node: SubjectRef<'_>,
    pred: NamedNodeRef<'_>,
    label: &str,
    slot: &mut Option<f64>,
    tighten: impl Fn(f64, f64) -> f64,
    unsupported: &mut Vec<String>,
) {
    let Some(term) = graph.object_for_subject_predicate(node, pred) else {
        return;
    };
    match literal_f64(term) {
        Some(n) => *slot = Some(slot.map_or(n, |e| tighten(e, n))),
        None => unsupported.push(format!(
            "{label} is stated but its value is not a number this crate can compare against — not applied"
        )),
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
