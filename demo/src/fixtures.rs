//! Two starting points for the demo textarea: a small hand-written shape
//! exercising several constructs at once, and a real, dense shapes graph
//! this crate did not write — the same `hive:` honeycomb fixture
//! `shacl-form-core`'s own integration tests run against (mirrored from
//! `ds42.org`'s `dataspace` repo, `vendor/ds-honeycomb-editor-rs/shapes.ttl`
//! — see dataspace ADR-0024).
pub struct Preset {
    pub label: &'static str,
    pub shapes_ttl: &'static str,
    pub target_shape_iri: &'static str,
}

pub const SIMPLE: Preset = Preset {
    label: "Simple example (Person, nested Address)",
    shapes_ttl: r#"@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .

ex:PersonShape a sh:NodeShape ;
  sh:name "Person" ;
  sh:property [
    sh:path ex:name ; sh:name "Name" ; sh:datatype xsd:string ;
    sh:minCount 1 ; sh:maxCount 1 ; sh:order 1
  ] ;
  sh:property [
    sh:path ex:age ; sh:name "Age" ; sh:datatype xsd:integer ;
    sh:minInclusive 0 ; sh:maxInclusive 150 ; sh:order 2
  ] ;
  sh:property [
    sh:path ex:status ; sh:name "Status" ;
    sh:in ( ex:Active ex:Inactive ex:Pending ) ; sh:order 3
  ] ;
  sh:property [
    sh:path ex:email ; sh:name "Email addresses" ; sh:datatype xsd:string ;
    sh:pattern "^[^@]+@[^@]+$" ; sh:order 4
  ] ;
  sh:property [
    sh:path ex:address ; sh:name "Home address" ; sh:node ex:AddressShape ; sh:order 5
  ] .

ex:AddressShape a sh:NodeShape ;
  sh:property [ sh:path ex:street ; sh:name "Street" ; sh:datatype xsd:string ; sh:order 1 ] ;
  sh:property [ sh:path ex:city ; sh:name "City" ; sh:datatype xsd:string ; sh:minCount 1 ; sh:order 2 ] .
"#,
    target_shape_iri: "http://example.org/PersonShape",
};

pub const HONEYCOMB: Preset = Preset {
    label: "Real-world: hive: honeycomb diagram shapes (hsh:TileShape)",
    shapes_ttl: include_str!("../../shacl-form-core/tests/fixtures/honeycomb-shapes.ttl"),
    target_shape_iri: "https://semantic.ds-labs.org/shapes/honeycomb#TileShape",
};

pub const ALL: &[&Preset] = &[&SIMPLE, &HONEYCOMB];
