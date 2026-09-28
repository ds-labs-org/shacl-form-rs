//! `check_constraints`: the executable-Rust twin of the HTML attributes
//! `shacl-form-yew`'s built-in controls encode SHACL constraints as — for
//! a host-supplied custom control (see `shacl-form-yew`'s `FieldOverride`)
//! that isn't a real `<input>` the browser can natively validate.
use oxrdf::{Literal, NamedNode};
use shacl_form_core::{ValueEntry, check_constraints, from_shape_iri, parse_turtle};

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
fn a_value_matching_every_pattern_and_length_bound_passes() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ;
                        sh:pattern "^A" ; sh:minLength 2 ; sh:maxLength 5 ] .
        "#,
    );
    let entry = ValueEntry::Literal(Literal::new_simple_literal("ABC"));
    assert!(check_constraints(&schema.fields[0], &entry).is_ok());
}

#[test]
fn a_value_failing_a_pattern_is_rejected_with_a_reason() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ; sh:pattern "^A" ] .
        "#,
    );
    let entry = ValueEntry::Literal(Literal::new_simple_literal("zzz"));
    let err = check_constraints(&schema.fields[0], &entry).unwrap_err();
    assert!(err.contains("^A"), "{err}");
}

/// SHACL's sh:pattern is an unanchored substring match — "A" anywhere in
/// the value is enough, not just at the start.
#[test]
fn a_pattern_match_is_unanchored() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ; sh:pattern "A" ] .
        "#,
    );
    let entry = ValueEntry::Literal(Literal::new_simple_literal("zzzAzzz"));
    assert!(check_constraints(&schema.fields[0], &entry).is_ok());
}

#[test]
fn several_patterns_all_apply() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ; sh:pattern "^A", "Z$" ] .
        "#,
    );
    assert!(
        check_constraints(
            &schema.fields[0],
            &ValueEntry::Literal(Literal::new_simple_literal("AxxZ"))
        )
        .is_ok()
    );
    let err = check_constraints(
        &schema.fields[0],
        &ValueEntry::Literal(Literal::new_simple_literal("Axxx")),
    )
    .unwrap_err();
    assert!(err.contains("Z$"), "{err}");
}

#[test]
fn length_uses_unicode_scalar_values_not_utf16_units() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ; sh:maxLength 1 ] .
        "#,
    );
    // A single emoji is one Unicode scalar value but two UTF-16 code units
    // (HTML's own maxlength would, per spec, count it as 2 and reject it —
    // this function must not).
    let entry = ValueEntry::Literal(Literal::new_simple_literal("\u{1F600}"));
    assert!(check_constraints(&schema.fields[0], &entry).is_ok());
}

#[test]
fn an_untouched_empty_entry_is_never_rejected() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ;
                        sh:minLength 3 ; sh:pattern "^A" ] .
        "#,
    );
    let entry = ValueEntry::Literal(Literal::new_simple_literal(""));
    assert!(check_constraints(&schema.fields[0], &entry).is_ok());
}

#[test]
fn a_numeric_bound_is_enforced() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:age ; sh:datatype xsd:integer ; sh:minInclusive 0 ; sh:maxInclusive 130 ] .
        "#,
    );
    assert!(
        check_constraints(
            &schema.fields[0],
            &shacl_form_core::literal_entry(&schema.fields[0], "30")
        )
        .is_ok()
    );
    let err = check_constraints(
        &schema.fields[0],
        &shacl_form_core::literal_entry(&schema.fields[0], "200"),
    )
    .unwrap_err();
    assert!(err.contains("130"), "{err}");
}

#[test]
fn a_non_numeric_value_in_a_numeric_field_is_rejected() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:age ; sh:datatype xsd:integer ] .
        "#,
    );
    let entry = ValueEntry::Literal(Literal::new_simple_literal("not-a-number"));
    assert!(check_constraints(&schema.fields[0], &entry).is_err());
}

#[test]
fn a_malformed_pattern_does_not_block_submission() {
    // `(?!` is a negative lookahead: valid JS/PCRE, but `regex`'s
    // linear-time engine refuses to compile it. A shape-authoring problem,
    // not this value's fault — must not block the user.
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:code ; sh:datatype xsd:string ; sh:pattern "(?!x)" ] .
        "#,
    );
    let entry = ValueEntry::Literal(Literal::new_simple_literal("anything"));
    assert!(check_constraints(&schema.fields[0], &entry).is_ok());
}
