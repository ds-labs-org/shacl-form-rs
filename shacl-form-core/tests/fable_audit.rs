//! Regression tests for the Fable audit of commit 8e452be (the state after
//! the Opus-audit fixes) — findings against the *rewritten* shape-merging
//! and nesting logic that earlier audit never saw.
use oxrdf::NamedNode;
use shacl_form_core::{FieldKind, from_shape_iri, from_target_class, parse_turtle};

const PREFIXES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .
"#;

fn schema_for(body: &str) -> shacl_form_core::FormSchema {
    let turtle = format!("{PREFIXES}\n{body}");
    let graph = parse_turtle(&turtle).expect("fixture must parse");
    from_shape_iri(&graph, &NamedNode::new("http://example.org/S").unwrap())
        .expect("fixture must be a sh:NodeShape")
}

/// Finding 1: `sh:node` and `sh:class` both stated on one property shape.
/// `resolve_kind` used to take the `sh:node` branch unconditionally and
/// never even look at `acc.class` — no `unsupported` note, and (finding 2)
/// the nested value's `target_class` stayed `None`, so `sh:class`'s own
/// class was silently dropped: not reported, and not asserted as
/// `rdf:type` on submit either, even though a real `sh:class` constraint
/// requires exactly that.
#[test]
fn sh_node_and_sh_class_together_notes_the_conflict_and_still_asserts_the_class() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:manager ; sh:node ex:PersonShape ; sh:class ex:Organization ] .
        ex:PersonShape a sh:NodeShape ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(
        field
            .unsupported
            .as_deref()
            .unwrap_or("")
            .contains("sh:node"),
        "{:?}",
        field.unsupported
    );
    assert!(
        field
            .unsupported
            .as_deref()
            .unwrap_or("")
            .contains("sh:class"),
        "{:?}",
        field.unsupported
    );
    let FieldKind::Nested { schema: nested } = &field.kind else {
        panic!("expected Nested, got {:?}", field.kind)
    };
    // Still nests via sh:node's own shape (its properties)...
    assert_eq!(nested.fields[0].label, "name");
    // ...but must not silently drop sh:class's own class: the value still
    // needs `rdf:type ex:Organization` asserted on submit.
    assert_eq!(
        nested.target_class.as_ref().map(|c| c.as_str()),
        Some("http://example.org/Organization")
    );
}

/// Finding 2: the *same* shape reached two different ways ended up with a
/// different `target_class` depending only on how it was reached — direct
/// `from_shape_iri` read the shape's own `sh:targetClass`, but a bare
/// `sh:node <shape>` reference (no accompanying `sh:class`) hardcoded
/// `target_class: None` instead of reading the very same triple off the
/// very same shape. Consequence: a value nested via plain `sh:node` never
/// got `rdf:type` asserted even when the referenced shape states its own
/// `sh:targetClass` — the identical shape gets `rdf:type` when used as the
/// form's own root, but not as a nested value.
#[test]
fn sh_node_alone_infers_target_class_from_the_referenced_shape_s_own_sh_target_class() {
    let graph = parse_turtle(&format!(
        "{PREFIXES}\nex:S a sh:NodeShape ; sh:property [ sh:path ex:manager ; sh:node ex:PersonShape ] .\nex:PersonShape a sh:NodeShape ; sh:targetClass ex:Person ; sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ."
    ))
    .unwrap();
    let direct = from_shape_iri(
        &graph,
        &NamedNode::new("http://example.org/PersonShape").unwrap(),
    )
    .unwrap();
    assert_eq!(
        direct.target_class.as_ref().map(|c| c.as_str()),
        Some("http://example.org/Person"),
        "sanity: from_shape_iri already reads sh:targetClass directly"
    );

    let outer = from_shape_iri(&graph, &NamedNode::new("http://example.org/S").unwrap()).unwrap();
    let FieldKind::Nested { schema: nested } = &outer.fields[0].kind else {
        panic!("expected Nested, got {:?}", outer.fields[0].kind)
    };
    assert_eq!(
        nested.target_class.as_ref().map(|c| c.as_str()),
        Some("http://example.org/Person"),
        "the identical ex:PersonShape must get the same target_class whether reached directly or via sh:node"
    );
}

/// Finding 5: `sh:or ()` / `sh:xone ()` — an empty branch list — is
/// spec-invalid input, but this crate's own stated design is "nothing here
/// is silently approximated"; an empty combinator vanished with no trace
/// at all (no field-level note), unlike every other malformed-input case.
#[test]
fn an_empty_sh_or_list_is_reported_not_silently_ignored() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:p ; sh:datatype xsd:string ; sh:or () ] .
        "#,
    );
    assert!(
        schema.fields[0].unsupported.is_some(),
        "an empty sh:or list must be reported, not silently ignored"
    );
}

/// Finding 6: `find_node_shapes_for_class` required an explicit
/// `a sh:NodeShape` triple on top of `sh:targetClass`, so a shape that
/// states `sh:targetClass`/`sh:property` but omits the (optional, per
/// SHACL's own structural target-recognition rules) type triple was
/// invisible to `sh:class` resolution — and the resulting error message
/// claimed "no sh:NodeShape has sh:targetClass <X>" even when one exists,
/// just untyped.
#[test]
fn a_shape_with_targetclass_but_no_explicit_a_sh_node_shape_is_still_found() {
    let graph = parse_turtle(&format!(
        "{PREFIXES}\nex:PersonShape sh:targetClass ex:Person ; sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ."
    ))
    .unwrap();
    let schema = from_target_class(&graph, &NamedNode::new("http://example.org/Person").unwrap()).expect("an implicitly-typed shape (sh:targetClass/sh:property, no explicit `a sh:NodeShape`) must still be found");
    assert_eq!(schema.fields[0].label, "name");
}
