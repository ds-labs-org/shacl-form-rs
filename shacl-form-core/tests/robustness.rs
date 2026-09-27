//! Regression tests for the Opus audit of commit d4faacc — each test names
//! the exact scenario that used to hang, crash, corrupt data, or silently
//! drop a constraint, reproduced from the audit's own examples.
use oxrdf::{NamedNode, NamedOrBlankNode};
use shacl_form_core::{
    FieldKind, FormValues, ValueEntry, default_entry, from_shape_iri, from_target_class,
    literal_entry, parse_turtle,
};
use std::time::{Duration, Instant};

const PREFIXES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix ex: <http://example.org/> .
"#;

fn schema_for(body: &str) -> shacl_form_core::FormSchema {
    let turtle = format!("{PREFIXES}\n{body}");
    let graph = parse_turtle(&turtle).expect("fixture must parse");
    from_shape_iri(&graph, &NamedNode::new("http://example.org/S").unwrap())
        .expect("fixture must be a sh:NodeShape")
}

/// Audit C1: `_:l rdf:first ex:a ; rdf:rest _:l` — a self-referential
/// rdf:List used as `sh:in`'s value. Previously grew `rdf_list`'s output
/// vector without bound.
#[test]
fn a_self_referential_rdf_list_terminates_instead_of_growing_forever() {
    let started = Instant::now();
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:p ; sh:in ex:l ] .
        ex:l rdf:first ex:a ; rdf:rest ex:l .
        "#,
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "rdf_list must not loop forever on a cyclic list"
    );
    let FieldKind::Select { options } = &schema.fields[0].kind else {
        panic!("expected Select, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(
        options.len(),
        1,
        "a cyclic list contributes its one real element once, not infinitely"
    );
}

/// Audit C1: a property shape whose own `sh:and` list contains itself.
/// Previously recursed through `collect_constraints` with no depth limit,
/// overflowing the stack — not a catchable panic on wasm.
#[test]
fn a_self_referential_sh_and_terminates_instead_of_overflowing_the_stack() {
    let started = Instant::now();
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property ex:P .
        ex:P sh:path ex:p ; sh:and ( ex:P ) .
        "#,
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(schema.fields.len(), 1);
    assert!(
        schema
            .all_unsupported()
            .iter()
            .any(|u| u.contains("nested too deep")),
        "{:?}",
        schema.all_unsupported()
    );
}

/// Audit C10: `sh:message`/`sh:severity` are validation-result metadata, not
/// constraints — a real shapes graph states `sh:message` on nearly every
/// property (the honeycomb fixture does on all 12 of DiagramShape's
/// fields), and flagging each one as "unrecognised" drowns out constraints
/// that actually matter. `sh:deactivated true` on a property shape means
/// "not used" per the spec — the field must not appear at all.
#[test]
fn message_and_severity_are_not_reported_as_unsupported_and_deactivated_properties_disappear() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [
            sh:path ex:p ; sh:datatype xsd:string ;
            sh:message "must be a string" ; sh:severity sh:Violation
          ] ;
          sh:property [ sh:path ex:gone ; sh:datatype xsd:string ; sh:deactivated true ] .
        "#,
    );
    assert_eq!(
        schema.fields.len(),
        1,
        "the deactivated property must not produce a field at all"
    );
    assert_eq!(schema.fields[0].label, "p");
    assert!(
        schema.all_unsupported().is_empty(),
        "sh:message/sh:severity must not be reported as unrecognised: {:?}",
        schema.all_unsupported()
    );
}

/// Audit C11: two `sh:property` entries in *one* shape sharing a path used
/// to produce two competing fields (and, on read-back, duplicate triples).
/// They must merge into one field with the tightest combination of both
/// constraint sets — the same rule `sh:and` already applies within one
/// property shape.
#[test]
fn two_property_shapes_sharing_one_path_in_the_same_node_shape_merge_into_one_field() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:minCount 1 ] ;
          sh:property [ sh:path ex:name ; sh:maxLength 10 ] .
        "#,
    );
    assert_eq!(
        schema.fields.len(),
        1,
        "one field, not two, for the shared path"
    );
    assert_eq!(schema.fields[0].min_count, 1);
    let FieldKind::Text { max_length, .. } = &schema.fields[0].kind else {
        panic!("expected Text, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(*max_length, Some(10));
}

/// Audit C6: `from_target_class` must merge sibling shapes' fields even
/// when the properties-bearing shape does not happen to sort first —
/// previously only true by luck (`hsh:DiagramShape` sorts before its
/// SPARQL-only siblings in the honeycomb fixture); this fixture is
/// constructed so the property-bearing shape sorts *last*.
#[test]
fn from_target_class_finds_properties_even_when_the_bearing_shape_sorts_last() {
    let graph = parse_turtle(&format!(
        "{PREFIXES}\nex:ZShapeWithProperties a sh:NodeShape ; sh:targetClass ex:Thing ; sh:property [ sh:path ex:name ; sh:datatype xsd:string ] .\nex:AShapeSparqlOnly a sh:NodeShape ; sh:targetClass ex:Thing ."
    ))
    .unwrap();
    let schema =
        from_target_class(&graph, &NamedNode::new("http://example.org/Thing").unwrap()).unwrap();
    assert_eq!(
        schema.fields.len(),
        1,
        "must find ex:ZShapeWithProperties's field despite sorting after ex:AShapeSparqlOnly"
    );
    assert_eq!(schema.fields[0].label, "name");
}

/// Audit C13: `sh:node`/`sh:and` at the *node-shape* level ("shape
/// inheritance" — `ex:EmployeeShape sh:node ex:PersonShape`) is a common
/// SHACL idiom; the inherited shape's own `sh:property` list must be
/// collected too, not reported as an unrecognised `sh:node`/`sh:and` on the
/// node shape.
#[test]
fn node_level_sh_node_and_sh_and_pull_in_the_referenced_shapes_own_properties() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:node ex:BaseShape ;
          sh:and ( ex:MixinShape ) ;
          sh:property [ sh:path ex:own ; sh:datatype xsd:string ] .
        ex:BaseShape a sh:NodeShape ;
          sh:property [ sh:path ex:inherited ; sh:datatype xsd:string ] .
        ex:MixinShape a sh:NodeShape ;
          sh:property [ sh:path ex:mixedIn ; sh:datatype xsd:string ] .
        "#,
    );
    let labels: Vec<_> = schema.fields.iter().map(|f| f.label.as_str()).collect();
    assert!(labels.contains(&"own"), "{labels:?}");
    assert!(labels.contains(&"inherited"), "{labels:?}");
    assert!(labels.contains(&"mixedIn"), "{labels:?}");
    assert!(
        schema.all_unsupported().is_empty(),
        "{:?}",
        schema.all_unsupported()
    );
}

/// Audit C5: a later, looser `sh:and` branch must not widen an earlier,
/// tighter constraint — `sh:maxLength 5` combined with `sh:maxLength 50`
/// must keep the tighter bound (5), and both patterns must survive, not
/// just the last one seen.
#[test]
fn sh_and_tightens_rather_than_overwrites_and_keeps_every_pattern() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [
            sh:path ex:c ; sh:maxLength 5 ;
            sh:and ( [ sh:maxLength 50 ] [ sh:pattern "^a" ] [ sh:pattern "z$" ] )
          ] .
        "#,
    );
    let FieldKind::Text {
        patterns,
        max_length,
        ..
    } = &schema.fields[0].kind
    else {
        panic!("expected Text, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(
        *max_length,
        Some(5),
        "the tighter maxLength (5) must win over the looser one (50)"
    );
    assert_eq!(
        patterns.as_slice(),
        ["^a", "z$"],
        "both patterns must survive, not just the last"
    );
}

/// Audit C7: a `from_target_class` form's own root, and any value nested
/// under a `sh:class` constraint, must assert `rdf:type` for that class —
/// otherwise the produced instance does not even conform to the `sh:class`
/// constraint that required it to be shaped in the first place.
#[test]
fn nested_sh_class_values_and_the_form_s_own_root_assert_rdf_type() {
    let graph = parse_turtle(&format!(
        "{PREFIXES}\nex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ; sh:property [ sh:path ex:manager ; sh:class ex:Person ] ."
    ))
    .unwrap();
    let schema = from_target_class(
        &graph,
        &NamedNode::new("http://example.org/Person").unwrap(),
    )
    .unwrap();
    assert_eq!(
        schema.target_class.as_ref().unwrap().as_str(),
        "http://example.org/Person"
    );
    let FieldKind::Nested { schema: nested } = &schema.fields[0].kind else {
        panic!("expected Nested, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(
        nested.target_class.as_ref().unwrap().as_str(),
        "http://example.org/Person"
    );
}

/// Audit P2: a schema with several mutually- and self-referencing `sh:node`
/// properties used to expand to roughly `k^MAX_NESTING_DEPTH` fields (the
/// audit measured 4,372 fields at k=3, 27,305 at k=4) because every
/// occurrence re-walked and re-allocated an identical subtree. With the
/// shape-expansion cache, the same shape reached the same way at the same
/// depth is computed once — the tree stays small regardless of how many
/// self-referencing properties one shape has.
#[test]
fn a_self_referencing_shape_with_several_such_properties_does_not_expand_exponentially() {
    let started = Instant::now();
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:knows ; sh:node ex:S ] ;
          sh:property [ sh:path ex:parent ; sh:node ex:S ] ;
          sh:property [ sh:path ex:child ; sh:node ex:S ] ;
          sh:property [ sh:path ex:spouse ; sh:node ex:S ] .
        "#,
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "construction took {:?}",
        started.elapsed()
    );

    // The metric that actually matters: how many *distinct* FormSchema
    // allocations exist, not how many ways a naive traversal could reach
    // one (four sibling properties pointing at the *same* cached Rc still
    // look like four separate subtrees to a traversal that doesn't track
    // pointer identity — that's what `FormSchema::all_unsupported()`'s own
    // recursion does, and it isn't what regressed here). Without the
    // shape-expansion cache, this shape (k=4 self-referencing properties)
    // allocated 27,305 real Field/FormSchema structs; with it, one distinct
    // schema per nesting depth (0..=MAX_NESTING_DEPTH), shared by every
    // property that reaches it.
    fn count_distinct_schemas(
        schema: &shacl_form_core::FormSchema,
        visited: &mut std::collections::HashSet<usize>,
    ) -> usize {
        let mut count = 1;
        for field in &schema.fields {
            if let FieldKind::Nested { schema: nested } = &field.kind
                && visited.insert(std::rc::Rc::as_ptr(nested) as usize)
            {
                count += count_distinct_schemas(nested, visited);
            }
        }
        count
    }
    let distinct = count_distinct_schemas(&schema, &mut std::collections::HashSet::new());
    assert!(
        distinct <= 8,
        "expected one distinct schema per nesting depth (0..=6); got {distinct} distinct allocations"
    );
}

/// Audit C3: an untouched required field (a blank Text/Iri slot from
/// `FormValues::new_for`'s padding) must not be written out as `ex:name ""`
/// or `ex:home <>` — "required" would then be satisfied by nothing at all.
/// A checkbox's own blank default must be `false`, not the ill-typed `""`.
#[test]
fn untouched_required_slots_are_never_serialised_and_boolean_defaults_to_false() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:minCount 1 ] ;
          sh:property [ sh:path ex:home ; sh:nodeKind sh:IRI ; sh:minCount 1 ] ;
          sh:property [ sh:path ex:ok ; sh:datatype xsd:boolean ; sh:minCount 1 ] .
        "#,
    );
    let ok_idx = schema.fields.iter().position(|f| f.label == "ok").unwrap();
    let ValueEntry::Literal(default_bool) = default_entry(&schema.fields[ok_idx]) else {
        panic!()
    };
    assert_eq!(default_bool.value(), "false");

    let values = FormValues::new_for(&schema);
    let subject = NamedOrBlankNode::NamedNode(NamedNode::new("http://example.org/x").unwrap());
    let turtle = values.to_turtle(&schema, &subject, &[("ex", "http://example.org/")]);
    assert!(
        !turtle.contains("\"\""),
        "an empty literal must never be written: {turtle}"
    );
    assert!(
        !turtle.contains("<>"),
        "an empty IRI must never be written: {turtle}"
    );
}

/// Audit C8: an HTML `datetime-local` control omits seconds when untouched
/// (`"2024-05-01T10:30"`), which is not a legal `xsd:dateTime` lexical form
/// — seconds are mandatory. `literal_entry` must pad it.
#[test]
fn a_datetime_value_missing_seconds_is_padded_to_a_legal_lexical_form() {
    let schema = schema_for(
        "ex:S a sh:NodeShape ; sh:property [ sh:path ex:when ; sh:datatype xsd:dateTime ] .",
    );
    let ValueEntry::Literal(lit) = literal_entry(&schema.fields[0], "2024-05-01T10:30") else {
        panic!()
    };
    assert_eq!(lit.value(), "2024-05-01T10:30:00");
}
