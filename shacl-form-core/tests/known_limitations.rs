//! TDD closing several items the README's own "Known limitations" section
//! named as real-but-unfixed after the third (Opus 5.5) audit round. Each
//! test names the exact silent drop being closed and, where the underlying
//! constraint genuinely can't be enforced by this crate's controls, checks
//! that it is now *reported* rather than vanishing without a trace — this
//! crate's own stated design principle, applied to its own backlog.
use oxrdf::NamedNode;
use shacl_form_core::{
    FieldKind, ValueEntry, from_shape_iri, literal_entry_with_language, parse_turtle,
};

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

/// `sh:in`/`sh:hasValue` stated alongside `sh:class`/`sh:node` used to be
/// silently dropped with no note at all — `resolve_kind` returns as soon as
/// it sees `sh:node`/`sh:class`, never even looking at `acc.in_list`/
/// `acc.has_value`. The nesting still wins (an exact shape reference is
/// more useful than "however many things happen to be in this list"), but
/// the dropped enumeration must say so.
#[test]
fn sh_in_next_to_sh_class_is_noted_not_silently_dropped() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:favourite ; sh:class ex:Color ; sh:in ( ex:Red ex:Green ) ] .
        ex:ColorShape a sh:NodeShape ; sh:targetClass ex:Color ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(matches!(field.kind, FieldKind::Nested { .. }));
    assert!(
        field.unsupported.as_deref().unwrap_or("").contains("sh:in"),
        "{:?}",
        field.unsupported
    );
}

/// Same silent drop, for `sh:hasValue` next to `sh:node`.
#[test]
fn sh_has_value_next_to_sh_node_is_noted_not_silently_dropped() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:manager ; sh:node ex:PersonShape ; sh:hasValue ex:alice ] .
        ex:PersonShape a sh:NodeShape ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(matches!(field.kind, FieldKind::Nested { .. }));
    assert!(
        field
            .unsupported
            .as_deref()
            .unwrap_or("")
            .contains("sh:hasValue"),
        "{:?}",
        field.unsupported
    );
}

/// A single property shape stating `sh:pattern` more than once (legal RDF —
/// a node can have several objects for one predicate) used to have all but
/// one silently ignored via `object_for_subject_predicate`, with no note,
/// unlike two different `sh:and` branches each stating one pattern, which
/// do accumulate. Both patterns must now be enforced.
#[test]
fn several_sh_pattern_values_on_one_property_shape_all_apply() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ; sh:pattern "^A", "Z$" ] .
        "#,
    );
    let FieldKind::Text { patterns, .. } = &schema.fields[0].kind else {
        panic!("expected Text, got {:?}", schema.fields[0].kind)
    };
    assert_eq!(patterns.len(), 2, "{patterns:?}");
    assert!(patterns.contains(&"^A".to_string()));
    assert!(patterns.contains(&"Z$".to_string()));
}

/// `sh:hasValue` stated more than once on one property shape means "must
/// have *every* one of these values" (a repeatable-property idiom), not "one
/// of these" — this crate models `sh:hasValue` as a single fixed option, so
/// it can't honour that, but it used to pick one arbitrarily with no note
/// at all. Now reported.
#[test]
fn several_sh_has_value_values_on_one_property_shape_are_noted() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:tag ; sh:hasValue ex:a, ex:b ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(
        field
            .unsupported
            .as_deref()
            .unwrap_or("")
            .contains("sh:hasValue"),
        "{:?}",
        field.unsupported
    );
}

/// A numeric bound (`sh:minInclusive`, say) whose literal isn't itself
/// numeric (a date, for instance — a legal pairing with `xsd:date`, just
/// one this crate's `Date` control has no `min`/`max` support for yet) used
/// to be dropped at the very first step (`literal_f64` fails to parse it)
/// with no trace at all. Now reported, distinctly from a bound this crate
/// simply never saw.
#[test]
fn a_non_numeric_bound_literal_is_noted_not_silently_unparsed() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:born ; sh:datatype xsd:date ; sh:minInclusive "2000-01-01"^^xsd:date ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(matches!(field.kind, FieldKind::Date));
    assert!(
        field
            .unsupported
            .as_deref()
            .unwrap_or("")
            .contains("sh:minInclusive"),
        "{:?}",
        field.unsupported
    );
}

/// `sh:pattern`/`sh:minLength`/`sh:maxLength` alongside `sh:nodeKind
/// sh:IRI` used to vanish with no note at all — `resolve_kind`'s IRI branch
/// returned before ever looking at `acc.patterns`/`acc.min_length`/
/// `acc.max_length`. An IRI's lexical form is exactly as valid a target for
/// these as a string literal's.
#[test]
fn sh_pattern_and_length_apply_to_an_iri_nodekind_field_too() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:page ; sh:nodeKind sh:IRI ;
                        sh:pattern "^https://" ; sh:minLength 10 ; sh:maxLength 200 ] .
        "#,
    );
    let field = &schema.fields[0];
    let FieldKind::Iri {
        patterns,
        min_length,
        max_length,
    } = &field.kind
    else {
        panic!("expected Iri, got {:?}", field.kind)
    };
    assert_eq!(patterns, &vec!["^https://".to_string()]);
    assert_eq!(*min_length, Some(10));
    assert_eq!(*max_length, Some(200));
    assert!(field.unsupported.is_none(), "{:?}", field.unsupported);
}

/// `sh:pattern` next to a kind that genuinely has no control to apply it to
/// (a numeric datatype, here) used to be silently dropped; now named.
#[test]
fn sh_pattern_next_to_a_kind_with_no_control_for_it_is_noted() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:integer ; sh:pattern "^[0-9]+$" ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(matches!(field.kind, FieldKind::Number { .. }));
    assert!(
        field
            .unsupported
            .as_deref()
            .unwrap_or("")
            .contains("sh:pattern"),
        "{:?}",
        field.unsupported
    );
}

/// `literal_entry_with_language` (what `shacl-form-yew`'s Text control
/// calls, passing through whatever language tag the value it's replacing
/// already carried) must keep that tag rather than silently discarding it
/// for a plain typed literal.
#[test]
fn editing_a_language_tagged_value_keeps_its_language_tag() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:greeting ; sh:datatype xsd:string ] .
        "#,
    );
    let entry = literal_entry_with_language(&schema.fields[0], "Bonjour!", Some("fr"));
    let ValueEntry::Literal(lit) = entry else {
        panic!("expected Literal")
    };
    assert_eq!(lit.value(), "Bonjour!");
    assert_eq!(lit.language(), Some("fr"));
}

/// `sh:datatype rdf:langString` is a recognised, common (SKOS/DCAT) case —
/// mapping it through the fallback "unrecognised sh:datatype" path was
/// itself wrong, independent of the literal-legality issue below.
#[test]
fn rdf_lang_string_is_a_recognised_datatype_not_an_unrecognised_one() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:label ; sh:datatype rdf:langString ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(matches!(field.kind, FieldKind::Text { .. }));
    assert!(
        field.unsupported.is_none(),
        "rdf:langString is a real, recognised datatype: {:?}",
        field.unsupported
    );
}

/// `rdf:langString` has no legal lexical form without a language tag —
/// typing into a field whose datatype is `rdf:langString`, with no tag to
/// keep, must not silently mint `"…"^^rdf:langString`, which is ill-formed
/// RDF.
#[test]
fn a_fresh_rdf_lang_string_value_with_no_tag_is_a_plain_literal_not_an_illegal_one() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:label ; sh:datatype rdf:langString ] .
        "#,
    );
    let entry = literal_entry_with_language(&schema.fields[0], "hello", None);
    let ValueEntry::Literal(lit) = entry else {
        panic!("expected Literal")
    };
    assert_eq!(lit.value(), "hello");
    assert_eq!(lit.language(), None);
    assert_ne!(
        lit.datatype().as_str(),
        "http://www.w3.org/1999/02/22-rdf-syntax-ns#langString",
        "a langString literal with no language tag is not legal RDF"
    );
}

/// A `sh:deactivated` node shape's own `sh:node`/`sh:and` ("shape
/// inheritance") references used to still be expanded and merged in, even
/// though the shape stating them is itself switched off — only its direct
/// `sh:property` list was excluded. A deactivated shape should contribute
/// nothing, including what it would otherwise have inherited.
#[test]
fn a_deactivated_shape_s_own_inherited_references_are_not_pulled_in() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:own ; sh:datatype xsd:string ] ;
          sh:node ex:Deactivated .
        ex:Deactivated a sh:NodeShape ; sh:deactivated true ;
          sh:node ex:Inherited ;
          sh:property [ sh:path ex:fromDeactivated ; sh:datatype xsd:string ] .
        ex:Inherited a sh:NodeShape ;
          sh:property [ sh:path ex:fromInherited ; sh:datatype xsd:string ] .
        "#,
    );
    let labels: Vec<&str> = schema.fields.iter().map(|f| f.label.as_str()).collect();
    assert!(labels.contains(&"own"), "{labels:?}");
    assert!(
        !labels.contains(&"fromDeactivated"),
        "a deactivated shape's own properties must not show up: {labels:?}"
    );
    assert!(
        !labels.contains(&"fromInherited"),
        "a deactivated shape's own inherited (sh:node) references must not be pulled in either: {labels:?}"
    );
}
