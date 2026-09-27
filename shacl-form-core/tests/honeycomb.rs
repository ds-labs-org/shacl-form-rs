//! One real, dense shapes graph this crate did not write:
//! `vendor/ds-honeycomb-editor-rs/shapes.ttl` from ds42.org's `dataspace`
//! repo (`hive:`/`hsh:` — see dataspace ADR-0024), mirrored verbatim into
//! `tests/fixtures/honeycomb-shapes.ttl`. Where the hand-written tests in
//! `tests/schema.rs` each isolate one SHACL construct, this file's job is
//! to prove they all still work together against a shape file with real
//! stakes: `sh:closed`, `sh:xone`, four `sh:sparql` node shapes sharing one
//! `sh:targetClass`, and a genuinely blank-node-free-text tangle of
//! `sh:nodeKind`/`sh:class` choices this crate did not get to design around.
use oxrdf::NamedNode;
use shacl_form_core::{FieldKind, from_shape_iri, from_target_class, parse_turtle};

const FIXTURE: &str = include_str!("fixtures/honeycomb-shapes.ttl");
const HSH: &str = "https://semantic.ds-labs.org/shapes/honeycomb#";
const HIVE: &str = "https://semantic.ds-labs.org/vocab/honeycomb#";

fn graph() -> oxrdf::Graph {
    parse_turtle(FIXTURE).expect("the vendored fixture must still parse as Turtle")
}

fn shape(local: &str) -> NamedNode {
    NamedNode::new(format!("{HSH}{local}")).unwrap()
}

#[test]
fn diagram_shape_offers_every_documented_metadata_field() {
    let schema = from_shape_iri(&graph(), &shape("DiagramShape")).unwrap();
    let labels: Vec<_> = schema.fields.iter().map(|f| f.label.as_str()).collect();
    for expected in [
        "slug",
        "label",
        "mode",
        "lattice",
        "formatVersion",
        "placement",
        "group",
    ] {
        assert!(
            labels.contains(&expected),
            "missing field {expected:?} in {labels:?}"
        );
    }
}

#[test]
fn placement_shape_is_closed_and_offers_exactly_its_five_properties() {
    // hsh:PlacementShape is the one sh:closed true shape in the contract —
    // see shapes.ttl's own header comment on why. This crate does not need
    // to enforce closedness itself (it only ever writes fields it read
    // from sh:property), but it must still surface that the shape claims
    // it, and it must not invent a field beyond the five the shape lists.
    let schema = from_shape_iri(&graph(), &shape("PlacementShape")).unwrap();
    assert!(schema.closed, "hsh:PlacementShape states sh:closed true");
    let labels: Vec<_> = schema.fields.iter().map(|f| f.label.as_str()).collect();
    assert_eq!(labels.len(), 5, "{labels:?}");
    for expected in ["col", "row", "inGroup", "represents", "tile"] {
        assert!(
            labels.contains(&expected),
            "missing field {expected:?} in {labels:?}"
        );
    }
}

#[test]
fn diagram_mode_is_a_select_over_exactly_the_two_named_individuals() {
    let schema = from_shape_iri(&graph(), &shape("DiagramShape")).unwrap();
    let mode = schema.fields.iter().find(|f| f.label == "mode").unwrap();
    let FieldKind::Select { options } = &mode.kind else {
        panic!("expected Select, got {:?}", mode.kind)
    };
    let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["pinned", "standalone"]);
}

#[test]
fn link_routing_select_has_all_three_named_individuals_in_source_order() {
    let schema = from_shape_iri(&graph(), &shape("LinkShape")).unwrap();
    let routing = schema.fields.iter().find(|f| f.label == "routing").unwrap();
    let FieldKind::Select { options } = &routing.kind else {
        panic!("expected Select")
    };
    let labels: Vec<_> = options.iter().map(|o| o.label.as_str()).collect();
    assert_eq!(labels, ["straight", "latticePath", "arc"]);
}

#[test]
fn from_target_class_merges_the_four_shapes_that_all_target_hive_diagram() {
    // hsh:DiagramShape, hsh:NoTwoInOneCell, hsh:ModeIsHonest and
    // hsh:GroupBelongsToItsDiagram all state `sh:targetClass hive:Diagram`.
    // The first has sh:property; the other three are sh:sparql-only. A
    // form for "the class hive:Diagram" must still come out with
    // DiagramShape's fields, not silently pick one of the property-less
    // ones because it happened to sort first.
    let class = NamedNode::new(format!("{HIVE}Diagram")).unwrap();
    let schema = from_target_class(&graph(), &class).unwrap();
    assert!(schema.fields.iter().any(|f| f.label == "slug"));
    // The three SPARQL-only shapes contribute no fields but must not be
    // silently invisible either — each is a real, un-enforced constraint.
    assert!(
        schema
            .all_unsupported()
            .iter()
            .any(|u| u.contains("sparql")),
        "{:?}",
        schema.all_unsupported()
    );
}

#[test]
fn every_shape_in_the_fixture_builds_a_schema_without_panicking() {
    // The blunt instrument: walk every sh:NodeShape the fixture declares,
    // not just the ones named above, so a future edit to shapes.ttl that
    // adds a construct this crate mishandles fails here first.
    let g = graph();
    let sh_node_shape = NamedNode::new("http://www.w3.org/ns/shacl#NodeShape").unwrap();
    let rdf_type = NamedNode::new("http://www.w3.org/1999/02/22-rdf-syntax-ns#type").unwrap();
    let shapes: Vec<_> = g
        .subjects_for_predicate_object(rdf_type.as_ref(), sh_node_shape.as_ref())
        .filter_map(|s| match s {
            oxrdf::SubjectRef::NamedNode(n) => Some(n.into_owned()),
            _ => None,
        })
        .collect();
    assert!(
        shapes.len() >= 8,
        "expected the fixture's known shape count, got {}",
        shapes.len()
    );
    for shape in shapes {
        let schema = from_shape_iri(&g, &shape)
            .unwrap_or_else(|e| panic!("<{shape}> failed to build a schema: {e}"));
        // Not a deep assertion — just that walking it terminated and
        // produced *something*, including for the three property-less
        // SPARQL-only shapes (an empty field list is the correct answer
        // for those, not a panic).
        let _ = schema.fields.len();
    }
}
