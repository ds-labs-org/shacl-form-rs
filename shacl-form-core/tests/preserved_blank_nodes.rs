//! A blank-node value on a field this crate cannot nest — `sh:class` with
//! no matching shape, `sh:nodeKind sh:BlankNodeOrIRI`, or a `sh:node`/
//! `sh:class` reference past `MAX_NESTING_DEPTH` — used to be invisible to
//! `read_from_instance`: neither shown nor kept, so re-saving an
//! otherwise-untouched instance silently deleted it (and everything only
//! reachable through it). These pin that it is now carried through
//! unedited, description and all.
use oxrdf::{Graph, Literal, NamedNode, NamedNodeRef, NamedOrBlankNode, SubjectRef, TermRef};
use shacl_form_core::{
    FormValues, ValueEntry, from_shape_iri, parse_instance_turtle, parse_turtle,
};

const PREFIXES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .
"#;

fn ex(local: &str) -> NamedNode {
    NamedNode::new(format!("http://example.org/{local}")).unwrap()
}

fn alice() -> NamedOrBlankNode {
    NamedOrBlankNode::NamedNode(ex("alice"))
}

/// Reads `instance` through the shape `ex:S`, re-serialises it untouched,
/// and parses the result back — what a host's "open, then save" does.
fn resave(shapes: &str, instance: &str) -> (Graph, FormValues, shacl_form_core::FormSchema) {
    let shapes = parse_turtle(&format!("{PREFIXES}\n{shapes}")).unwrap();
    let schema = from_shape_iri(&shapes, &ex("S")).unwrap();
    let data = parse_instance_turtle(&format!("{PREFIXES}\n{instance}")).unwrap();
    let values =
        FormValues::read_from_instance(&schema, &data, SubjectRef::NamedNode(ex("alice").as_ref()));
    let out = values.to_turtle(&schema, &alice(), &[]);
    (parse_instance_turtle(&out).unwrap(), values, schema)
}

/// Follows `path` from `from` and returns the one object it finds.
fn follow<'a>(g: &'a Graph, from: SubjectRef<'a>, path: &NamedNode) -> TermRef<'a> {
    let objs: Vec<_> = g.objects_for_subject_predicate(from, path).collect();
    assert_eq!(
        objs.len(),
        1,
        "expected exactly one {path} on {from}: {objs:?}"
    );
    objs[0]
}

fn as_subject(t: TermRef<'_>) -> SubjectRef<'_> {
    match t {
        TermRef::BlankNode(b) => SubjectRef::BlankNode(b),
        TermRef::NamedNode(n) => SubjectRef::NamedNode(n),
        other => panic!("expected a resource, got {other}"),
    }
}

fn literal(t: TermRef<'_>) -> String {
    match t {
        TermRef::Literal(l) => l.value().to_string(),
        other => panic!("expected a literal, got {other}"),
    }
}

#[test]
fn a_blank_node_under_sh_class_with_no_matching_shape_survives_a_resave() {
    let (out, values, _) = resave(
        r#"ex:S a sh:NodeShape ;
             sh:property [ sh:path ex:address ; sh:class ex:Address ] ;
             sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ."#,
        r#"ex:alice ex:name "Alice" ;
             ex:address [ a ex:Address ; ex:street "1 Main St" ; ex:city "Springfield" ] ."#,
    );
    let alice = SubjectRef::NamedNode(NamedNodeRef::new("http://example.org/alice").unwrap());
    let address = follow(&out, alice, &ex("address"));
    assert!(matches!(address, TermRef::BlankNode(_)), "{address}");
    let address = as_subject(address);
    assert_eq!(literal(follow(&out, address, &ex("street"))), "1 Main St");
    assert_eq!(literal(follow(&out, address, &ex("city"))), "Springfield");
    // A host can tell the value is there even though it has no control.
    let field = 0;
    assert_eq!(values.preserved(field).count(), 1);
    assert_eq!(values.get(field).len(), 0, "still not an editable entry");
}

#[test]
fn a_blank_node_under_blank_node_or_iri_keeps_its_whole_nested_description() {
    let (out, _, _) = resave(
        r#"ex:S a sh:NodeShape ;
             sh:property [ sh:path ex:pet ; sh:nodeKind sh:BlankNodeOrIRI ] ."#,
        r#"ex:alice ex:pet [ ex:name "Rex" ; ex:vet [ ex:name "Dr. Who" ] ] ."#,
    );
    let alice = SubjectRef::NamedNode(NamedNodeRef::new("http://example.org/alice").unwrap());
    let pet = as_subject(follow(&out, alice, &ex("pet")));
    assert_eq!(literal(follow(&out, pet, &ex("name"))), "Rex");
    let vet = as_subject(follow(&out, pet, &ex("vet")));
    assert_eq!(literal(follow(&out, vet, &ex("name"))), "Dr. Who");
}

#[test]
fn a_blank_node_past_the_nesting_depth_limit_survives_a_resave() {
    // Ten levels of anonymous friends: deeper than MAX_NESTING_DEPTH, so
    // the innermost ones land on a plain-Iri fallback field.
    let mut instance = String::from("ex:alice ex:name \"L0\" ; ex:knows ");
    for i in 1..=10 {
        instance.push_str(&format!("[ ex:name \"L{i}\" ; ex:knows "));
    }
    instance.push_str("ex:end");
    for _ in 1..=10 {
        instance.push_str(" ]");
    }
    instance.push_str(" .");
    let (out, _, _) = resave(
        r#"ex:S a sh:NodeShape ;
             sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ;
             sh:property [ sh:path ex:knows ; sh:node ex:S ] ."#,
        &instance,
    );
    let knows = ex("knows");
    let name = ex("name");
    let mut at = SubjectRef::NamedNode(NamedNodeRef::new("http://example.org/alice").unwrap());
    for i in 0..=10 {
        assert_eq!(
            literal(follow(&out, at, &name)),
            format!("L{i}"),
            "level {i}"
        );
        at = as_subject(follow(&out, at, &knows));
    }
    assert_eq!(at, SubjectRef::NamedNode(ex("end").as_ref()));
}

#[test]
fn a_cycle_between_preserved_blank_nodes_terminates_and_is_kept() {
    let (out, _, _) = resave(
        r#"ex:S a sh:NodeShape ;
             sh:property [ sh:path ex:ring ; sh:nodeKind sh:BlankNodeOrIRI ] ."#,
        r#"ex:alice ex:ring _:a .
           _:a ex:next _:b ; ex:label "a" .
           _:b ex:next _:a ; ex:label "b" ."#,
    );
    let alice = SubjectRef::NamedNode(NamedNodeRef::new("http://example.org/alice").unwrap());
    let a = as_subject(follow(&out, alice, &ex("ring")));
    let b = as_subject(follow(&out, a, &ex("next")));
    assert_eq!(literal(follow(&out, a, &ex("label"))), "a");
    assert_eq!(literal(follow(&out, b, &ex("label"))), "b");
    assert_eq!(as_subject(follow(&out, b, &ex("next"))), a);
}

#[test]
fn editing_a_sibling_field_keeps_the_preserved_value() {
    let shapes = parse_turtle(&format!(
        "{PREFIXES}\n{}",
        r#"ex:S a sh:NodeShape ;
             sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:order 0 ] ;
             sh:property [ sh:path ex:address ; sh:class ex:Address ; sh:order 1 ] ."#
    ))
    .unwrap();
    let schema = from_shape_iri(&shapes, &ex("S")).unwrap();
    let data = parse_instance_turtle(&format!(
        "{PREFIXES}\n{}",
        r#"ex:alice ex:name "Alice" ; ex:address [ ex:street "1 Main St" ] ."#
    ))
    .unwrap();
    let mut values =
        FormValues::read_from_instance(&schema, &data, SubjectRef::NamedNode(ex("alice").as_ref()));
    let mut edited = values.clone();
    edited.set(
        0,
        vec![ValueEntry::Literal(Literal::new_simple_literal("Alicia"))],
    );
    values = edited;
    let out = parse_instance_turtle(&values.to_turtle(&schema, &alice(), &[])).unwrap();
    let alice = SubjectRef::NamedNode(NamedNodeRef::new("http://example.org/alice").unwrap());
    assert_eq!(literal(follow(&out, alice, &ex("name"))), "Alicia");
    let address = as_subject(follow(&out, alice, &ex("address")));
    assert_eq!(literal(follow(&out, address, &ex("street"))), "1 Main St");
}

#[test]
fn a_fresh_form_has_nothing_preserved() {
    let shapes = parse_turtle(&format!(
        "{PREFIXES}\nex:S a sh:NodeShape ; sh:property [ sh:path ex:address ; sh:class ex:Address ] ."
    ))
    .unwrap();
    let schema = from_shape_iri(&shapes, &ex("S")).unwrap();
    let values = FormValues::new_for(&schema);
    assert_eq!(values.preserved(0).count(), 0);
}
