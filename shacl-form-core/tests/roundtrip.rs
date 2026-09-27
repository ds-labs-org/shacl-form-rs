//! A form is filled in, serialised to Turtle, and read back — twice: once
//! as a fresh instance (`FormValues::new_for` + edits), once by re-reading
//! that exact Turtle as if it were an existing instance being edited
//! (`FormValues::read_from_instance`). The two must agree, or "edit this
//! thing the form just created" is broken.
use oxrdf::vocab::xsd;
use oxrdf::{Literal, NamedNode, NamedOrBlankNode, SubjectRef};
use shacl_form_core::{
    FieldKind, FormValues, ValueEntry, from_shape_iri, literal_entry, parse_instance_turtle,
    parse_turtle,
};

#[test]
fn new_for_pre_fills_required_fields_so_a_form_never_opens_with_zero_slots_for_one() {
    let shapes = parse_turtle(
        r#"
        @prefix sh: <http://www.w3.org/ns/shacl#> .
        @prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
        @prefix ex: <http://example.org/> .
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:required ; sh:name "required" ; sh:datatype xsd:string ; sh:minCount 1 ] ;
          sh:property [ sh:path ex:optional ; sh:name "optional" ; sh:datatype xsd:string ] ;
          sh:property [ sh:path ex:twoRequired ; sh:name "twoRequired" ; sh:datatype xsd:string ; sh:minCount 2 ] .
        "#,
    )
    .unwrap();
    let schema = from_shape_iri(&shapes, &NamedNode::new("http://example.org/S").unwrap()).unwrap();
    let values = FormValues::new_for(&schema);

    let idx = |label: &str| schema.fields.iter().position(|f| f.label == label).unwrap();
    assert_eq!(
        values.get(idx("required")).len(),
        1,
        "a required field must start with one slot to type into"
    );
    assert_eq!(
        values.get(idx("optional")).len(),
        0,
        "an optional field starts empty"
    );
    assert_eq!(
        values.get(idx("twoRequired")).len(),
        2,
        "sh:minCount 2 pre-fills two slots, not one"
    );
}

const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .

ex:PersonShape a sh:NodeShape ;
  sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:minCount 1 ; sh:maxCount 1 ] ;
  sh:property [ sh:path ex:age ; sh:datatype xsd:nonNegativeInteger ] ;
  sh:property [ sh:path ex:knows ; sh:node ex:PersonShape ; sh:name "friend" ] .
"#;

#[test]
fn a_filled_in_form_serialises_and_reads_back_the_same_values() {
    let shapes = parse_turtle(SHAPES).unwrap();
    let schema = from_shape_iri(
        &shapes,
        &NamedNode::new("http://example.org/PersonShape").unwrap(),
    )
    .unwrap();

    let name_idx = schema
        .fields
        .iter()
        .position(|f| f.label == "name")
        .unwrap();
    let age_idx = schema.fields.iter().position(|f| f.label == "age").unwrap();
    let friend_idx = schema
        .fields
        .iter()
        .position(|f| f.label == "friend")
        .unwrap();

    // `age`'s original sh:datatype (xsd:nonNegativeInteger) must survive
    // the round trip rather than widening to plain xsd:integer.
    assert_eq!(
        schema.fields[age_idx]
            .original_datatype
            .as_ref()
            .unwrap()
            .as_str(),
        xsd::NON_NEGATIVE_INTEGER.as_str()
    );

    let mut values = FormValues::new_for(&schema);
    values.set(
        name_idx,
        vec![ValueEntry::Literal(Literal::new_simple_literal("Alice"))],
    );
    values.set(age_idx, vec![literal_entry(&schema.fields[age_idx], "30")]);

    let mut friend_values = FormValues::new_for(match &schema.fields[friend_idx].kind {
        FieldKind::Nested { schema } => schema,
        other => panic!("expected Nested, got {other:?}"),
    });
    let friend_name_idx = match &schema.fields[friend_idx].kind {
        FieldKind::Nested { schema } => schema
            .fields
            .iter()
            .position(|f| f.label == "name")
            .unwrap(),
        _ => unreachable!(),
    };
    friend_values.set(
        friend_name_idx,
        vec![ValueEntry::Literal(Literal::new_simple_literal("Bob"))],
    );
    values.set(friend_idx, vec![ValueEntry::Nested(friend_values)]);

    let subject = NamedOrBlankNode::NamedNode(NamedNode::new("http://example.org/alice").unwrap());
    let turtle = values.to_turtle(
        &schema,
        &subject,
        &[
            ("ex", "http://example.org/"),
            ("xsd", "http://www.w3.org/2001/XMLSchema#"),
        ],
    );

    assert!(turtle.contains("\"Alice\""), "{turtle}");
    assert!(
        turtle.contains("\"30\"^^xsd:nonNegativeInteger")
            || turtle.contains("\"30\"^^<http://www.w3.org/2001/XMLSchema#nonNegativeInteger>"),
        "{turtle}"
    );
    assert!(turtle.contains("\"Bob\""), "{turtle}");

    // Read it back as an existing instance and confirm the values agree.
    let instance = parse_instance_turtle(&turtle).unwrap();
    let read_back = FormValues::read_from_instance(
        &schema,
        &instance,
        SubjectRef::NamedNode(NamedNode::new("http://example.org/alice").unwrap().as_ref()),
    );

    let ValueEntry::Literal(name) = &read_back.get(name_idx)[0] else {
        panic!()
    };
    assert_eq!(name.value(), "Alice");
    let ValueEntry::Literal(age) = &read_back.get(age_idx)[0] else {
        panic!()
    };
    assert_eq!(age.value(), "30");
    assert_eq!(age.datatype().as_str(), xsd::NON_NEGATIVE_INTEGER.as_str());
    let ValueEntry::Nested(friend) = &read_back.get(friend_idx)[0] else {
        panic!()
    };
    let ValueEntry::Literal(friend_name) = &friend.get(friend_name_idx)[0] else {
        panic!()
    };
    assert_eq!(friend_name.value(), "Bob");
}
