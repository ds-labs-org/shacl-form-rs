//! Small, hand-written shapes graphs, one construct at a time. See
//! `tests/honeycomb.rs` for a single real, dense shapes graph exercising
//! all of these at once against a shapes file this crate did not write.
use oxrdf::NamedNode;
use shacl_form_core::{FieldKind, from_shape_iri, parse_turtle};

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

#[test]
fn datatype_maps_to_the_right_field_kind() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:age ; sh:datatype xsd:integer ] ;
          sh:property [ sh:path ex:active ; sh:datatype xsd:boolean ] ;
          sh:property [ sh:path ex:born ; sh:datatype xsd:date ] ;
          sh:property [ sh:path ex:home ; sh:datatype xsd:anyURI ] .
        "#,
    );
    let field = |label: &str| {
        schema
            .fields
            .iter()
            .find(|f| f.label == label)
            .unwrap_or_else(|| panic!("no field {label}"))
    };
    assert!(matches!(
        field("age").kind,
        FieldKind::Number {
            integer_only: true,
            ..
        }
    ));
    assert!(matches!(field("active").kind, FieldKind::Boolean));
    assert!(matches!(field("born").kind, FieldKind::Date));
    assert!(matches!(field("home").kind, FieldKind::Iri));
    assert!(
        schema.all_unsupported().is_empty(),
        "{:?}",
        schema.all_unsupported()
    );
}

#[test]
fn sh_in_becomes_a_select_with_those_exact_options() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:status ; sh:in ( ex:Open ex:Closed ) ] .
        "#,
    );
    let FieldKind::Select { options } = &schema.fields[0].kind else {
        panic!("expected Select, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].label, "Open");
    assert_eq!(options[1].label, "Closed");
}

#[test]
fn sh_has_value_becomes_a_one_option_select() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:kind ; sh:hasValue ex:Fixed ] .
        "#,
    );
    let FieldKind::Select { options } = &schema.fields[0].kind else {
        panic!("expected Select")
    };
    assert_eq!(options.len(), 1);
    assert_eq!(options[0].label, "Fixed");
}

#[test]
fn sh_node_nests_a_sub_schema() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:address ; sh:node ex:AddressShape ] .
        ex:AddressShape a sh:NodeShape ;
          sh:property [ sh:path ex:city ; sh:datatype xsd:string ] .
        "#,
    );
    let FieldKind::Nested { schema: nested } = &schema.fields[0].kind else {
        panic!("expected Nested")
    };
    assert_eq!(nested.fields.len(), 1);
    assert_eq!(nested.fields[0].label, "city");
}

#[test]
fn sh_class_resolves_to_the_node_shape_targeting_it_when_one_exists() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:manager ; sh:class ex:Person ] .
        ex:PersonShape a sh:NodeShape ;
          sh:targetClass ex:Person ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ] .
        "#,
    );
    let FieldKind::Nested { schema: nested } = &schema.fields[0].kind else {
        panic!("expected Nested, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(nested.fields[0].label, "name");
}

#[test]
fn sh_class_with_no_matching_shape_falls_back_to_a_plain_iri_field_and_says_so() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:manager ; sh:class ex:Person ] .
        "#,
    );
    assert!(matches!(schema.fields[0].kind, FieldKind::Iri));
    assert!(
        schema.fields[0]
            .unsupported
            .as_deref()
            .unwrap()
            .contains("no sh:NodeShape")
    );
}

#[test]
fn a_shape_nested_inside_itself_stops_rather_than_recursing_forever() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:self ; sh:node ex:S ] .
        "#,
    );
    // Must terminate at all (the test itself is the timeout guard). A
    // direct self-reference is allowed to nest several real, fillable
    // levels deep (see MAX_NESTING_DEPTH's own doc comment for why that is
    // the useful behaviour, not just a safety net) before it stops and
    // says so, rather than refusing on the very first level.
    assert!(
        schema
            .all_unsupported()
            .iter()
            .any(|u| u.contains("nesting stopped")),
        "{:?}",
        schema.all_unsupported()
    );
}

#[test]
fn sh_and_merges_constraints_from_every_branch_onto_one_field() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [
            sh:path ex:code ;
            sh:and ( [ sh:datatype xsd:string ] [ sh:pattern "^[A-Z]+$" ] [ sh:minLength 2 ] )
          ] .
        "#,
    );
    let FieldKind::Text {
        patterns,
        min_length,
        ..
    } = &schema.fields[0].kind
    else {
        panic!("expected Text, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(patterns.as_slice(), ["^[A-Z]+$"]);
    assert_eq!(*min_length, Some(2));
}

#[test]
fn sh_or_renders_the_first_branch_and_notes_the_rest() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [
            sh:path ex:value ;
            sh:or ( [ sh:datatype xsd:string ] [ sh:datatype xsd:integer ] )
          ] .
        "#,
    );
    assert!(matches!(schema.fields[0].kind, FieldKind::Text { .. }));
    assert!(
        schema.fields[0]
            .unsupported
            .as_deref()
            .unwrap()
            .contains("sh:or has 2 branches")
    );
}

#[test]
fn an_unrecognised_sh_constraint_is_reported_not_dropped() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:note ; sh:datatype xsd:string ; sh:equals ex:other ] .
        "#,
    );
    assert!(
        schema.fields[0]
            .unsupported
            .as_deref()
            .unwrap()
            .contains("equals")
    );
}

#[test]
fn a_blank_node_path_is_reported_by_name_and_still_shows_up_as_a_field() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path [ sh:inversePath ex:parent ] ; sh:name "children" ] .
        "#,
    );
    assert_eq!(
        schema.fields.len(),
        1,
        "the property must still be listed even though it can't be edited"
    );
    assert!(schema.fields[0].path.is_none());
    assert!(
        schema.fields[0]
            .unsupported
            .as_deref()
            .unwrap()
            .contains("inverse path")
    );
}

#[test]
fn min_and_max_count_default_to_shacl_s_own_defaults() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:tag ] ;
          sh:property [ sh:path ex:id ; sh:minCount 1 ; sh:maxCount 1 ] .
        "#,
    );
    let field = |label: &str| schema.fields.iter().find(|f| f.label == label).unwrap();
    assert_eq!(field("tag").min_count, 0);
    assert_eq!(field("tag").max_count, None);
    assert_eq!(field("id").min_count, 1);
    assert_eq!(field("id").max_count, Some(1));
}

#[test]
fn sh_order_sorts_fields_and_absent_order_comes_last() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:third ; sh:order 3 ] ;
          sh:property [ sh:path ex:unordered ] ;
          sh:property [ sh:path ex:first ; sh:order 1 ] .
        "#,
    );
    let labels: Vec<_> = schema.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels, ["first", "third", "unordered"]);
}
