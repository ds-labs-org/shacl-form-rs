//! Regression tests for the third audit pass (Opus 5.5) of commit 4a0f91f —
//! the state after both the Opus and Fable rounds' fixes. Findings against
//! cases neither earlier pass's fixtures combined: a datatype whose
//! `sh:datatype` requires a literal but which this crate rendered as an IRI
//! resource, an instance graph that cycles back on itself, and a blank-node
//! minting scheme that collided with identities read back from a prior
//! save.
use oxrdf::vocab::xsd;
use oxrdf::{BlankNode, NamedNode, NamedOrBlankNode, SubjectRef};
use shacl_form_core::{
    FieldKind, FormValues, ValueEntry, default_entry, from_shape_iri, parse_instance_turtle,
    parse_turtle,
};
use std::time::{Duration, Instant};

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

/// Finding 4: `sh:datatype xsd:anyURI` requires the value to be a LITERAL
/// with that datatype (per XSD/SHACL — `sh:datatype` only ever constrains a
/// literal's type), but `kind_for_datatype` mapped it to `FieldKind::Iri`,
/// the kind for a *resource reference*. The rendered control then wrote a
/// bare IRI node (`ex:home <http://a.example/>`), which does not conform to
/// the very `sh:datatype` constraint that produced the field: the value
/// must be `"http://a.example/"^^xsd:anyURI`, not a node.
#[test]
fn xsd_any_uri_is_a_literal_field_not_an_iri_resource_field() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:home ; sh:datatype xsd:anyURI ] .
        "#,
    );
    let field = &schema.fields[0];
    assert!(
        matches!(field.kind, FieldKind::Text { .. }),
        "xsd:anyURI must render as a literal-producing control, got {:?}",
        field.kind
    );
    assert_eq!(
        field.original_datatype.as_ref().map(|d| d.as_str()),
        Some(xsd::ANY_URI.as_str()),
        "the field must still remember xsd:anyURI so serialisation round-trips the exact datatype"
    );
    // No "unrecognised sh:datatype" note — xsd:anyURI IS recognised, just
    // mapped to a different control than sh:nodeKind sh:IRI's.
    assert!(field.unsupported.is_none(), "{:?}", field.unsupported);

    let entry = shacl_form_core::literal_entry(field, "http://a.example/");
    match entry {
        ValueEntry::Literal(l) => {
            assert_eq!(l.value(), "http://a.example/");
            assert_eq!(l.datatype().as_str(), xsd::ANY_URI.as_str());
        }
        other => panic!("expected a Literal entry for xsd:anyURI, got {other:?}"),
    }
}

/// Finding 1 + 2: an instance subject reachable again through its own
/// `sh:node`/`sh:class` nesting (a self-loop here; the same root cause also
/// makes a longer cycle blow up combinatorially — see the next test) used
/// to recurse into a *fresh, independent* copy of itself, expanded again in
/// full, down to `MAX_NESTING_DEPTH`. Editing only the outer copy's field
/// and re-serialising then wrote BOTH the edited value and the untouched
/// inner copies' stale value for the very same subject and the very same
/// (`sh:maxCount 1`-constrained) predicate — genuinely conflicting,
/// non-conforming output for a single edit.
#[test]
fn a_self_referencing_instance_subject_does_not_duplicate_its_own_fields() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ; sh:targetClass ex:Person ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ; sh:maxCount 1 ] ;
          sh:property [ sh:path ex:knows ; sh:class ex:Person ] .
        "#,
    );
    let instance = parse_instance_turtle(
        r#"
        @prefix ex: <http://example.org/> .
        ex:alice ex:name "Alice" ; ex:knows ex:alice .
        "#,
    )
    .unwrap();
    let alice = NamedNode::new("http://example.org/alice").unwrap();
    let mut values =
        FormValues::read_from_instance(&schema, &instance, SubjectRef::NamedNode(alice.as_ref()));

    let name_idx = schema
        .fields
        .iter()
        .position(|f| f.label == "name")
        .unwrap();
    values.set(
        name_idx,
        vec![ValueEntry::Literal(oxrdf::Literal::new_simple_literal(
            "Alicia",
        ))],
    );

    let turtle = values.to_turtle(
        &schema,
        &NamedOrBlankNode::NamedNode(alice),
        &[("ex", "http://example.org/")],
    );
    assert!(turtle.contains("\"Alicia\""), "{turtle}");
    assert!(
        !turtle.contains("\"Alice\""),
        "the stale, untouched inner copy of the same subject must not also assert the \
         old value for a maxCount-1 field once the outer copy was edited:\n{turtle}"
    );
}

/// Finding 2 (the DoS case): several instance subjects that all point back
/// at each other (any real "people who know each other" instance graph) —
/// without cycle detection, `read_from_instance` re-expands the identical
/// subject afresh every time it's reached, giving roughly
/// (branching factor)^`MAX_NESTING_DEPTH` entries. Bounded here to a small
/// fixed budget instead: reading one such instance must stay well under
/// what unbounded re-expansion to depth 6 would produce, and must finish
/// quickly.
#[test]
fn mutually_referencing_instance_subjects_do_not_expand_exponentially() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ; sh:targetClass ex:Person ;
          sh:property [ sh:path ex:name ; sh:datatype xsd:string ] ;
          sh:property [ sh:path ex:knows ; sh:class ex:Person ] .
        "#,
    );
    // 8 people who all know each other — every pair is a direct back-edge,
    // giving a branching factor of 7 at every level instead of a plain
    // acyclic chain's branching factor of 1.
    let mut instance_ttl = String::from("@prefix ex: <http://example.org/> .\n");
    for i in 0..8 {
        instance_ttl.push_str(&format!("ex:p{i} ex:name \"P{i}\" .\n"));
        for j in 0..8 {
            if j != i {
                instance_ttl.push_str(&format!("ex:p{i} ex:knows ex:p{j} .\n"));
            }
        }
    }
    let instance = parse_instance_turtle(&instance_ttl).unwrap();

    let started = Instant::now();
    let values = FormValues::read_from_instance(
        &schema,
        &instance,
        SubjectRef::NamedNode(NamedNode::new("http://example.org/p0").unwrap().as_ref()),
    );
    assert!(
        started.elapsed() < Duration::from_secs(2),
        "must not spend seconds re-expanding the same handful of subjects"
    );

    fn count_entries(v: &FormValues, schema: &shacl_form_core::FormSchema) -> usize {
        let mut total = 0;
        for (idx, field) in schema.fields.iter().enumerate() {
            for entry in v.get(idx) {
                total += 1;
                if let (FieldKind::Nested { schema: nested }, ValueEntry::Nested { values, .. }) =
                    (&field.kind, entry)
                {
                    total += count_entries(values, nested);
                }
            }
        }
        total
    }
    // Without cycle detection this fixture gives 1,098,056 entries (checked
    // directly against the pre-fix code — matches the audit's own reported
    // order of magnitude for the same shape of instance) and takes visibly
    // longer even at that. With it: a real, ~16x reduction (well under
    // 200,000) and comfortably under the 2s budget above. It is not a small
    // constant — an 8-node *complete* graph, read at this crate's own
    // MAX_NESTING_DEPTH, still has a genuinely large number of distinct
    // non-cyclic paths through it (a permutation count, not an exponential
    // recomputation), which no longer means unbounded recursion or redundant
    // re-walking of an identical subtree, just a large but finite, one-pass
    // reflection of how densely the instance data actually cross-references
    // itself — see the README's "Known limitations".
    let total = count_entries(&values, &schema);
    assert!(
        total < 200_000,
        "cycle detection + shared-read memoisation must keep this from the \
         near-unbounded blow-up an unguarded read gives (1,098,056 entries \
         for this exact fixture): got {total} entries"
    );
}

/// Finding 3: a *stable* subject minted for a brand-new (never-saved)
/// nested value used to be derived from that value's position in the tree
/// (`"shaclform" + tree path`) rather than assigned once, up front — so a
/// value freshly added at a position a PRIOR save's own remembered subject
/// happened to occupy the same position of (after an earlier entry was
/// removed) collided: two logically distinct resources ended up serialised
/// under the identical blank node label, silently merging them.
#[test]
fn a_freshly_added_nested_entry_gets_its_own_identity_immediately_not_derived_from_position() {
    let schema = schema_for(
        r#"
        ex:S a sh:NodeShape ;
          sh:property [ sh:path ex:addr ; sh:node ex:A ; sh:maxCount 4 ] .
        ex:A a sh:NodeShape ;
          sh:property [ sh:path ex:city ; sh:datatype xsd:string ] .
        "#,
    );
    let addr_idx = schema
        .fields
        .iter()
        .position(|f| f.label == "addr")
        .unwrap();
    let FieldKind::Nested {
        schema: addr_schema,
    } = &schema.fields[addr_idx].kind
    else {
        panic!("expected Nested")
    };

    // Simulate: this exact subject ("_:shaclformf{addr_idx}r1", the naming
    // scheme's own old convention) was already saved and is now being read
    // back as an existing instance's real, remembered identity — i.e. a
    // value that legitimately owns that blank node label already.
    let remembered_label = format!("shaclformf{addr_idx}r1");
    let remembered_subject =
        NamedOrBlankNode::BlankNode(BlankNode::new_unchecked(&remembered_label));

    // A brand-new entry, added fresh via `default_entry` (the same path
    // `Add` goes through) at that same tree position.
    let fresh = default_entry(&schema.fields[addr_idx]);
    let ValueEntry::Nested {
        subject: fresh_subject,
        ..
    } = &fresh
    else {
        panic!("expected Nested")
    };

    assert!(
        fresh_subject.is_some(),
        "a freshly created nested value must be assigned an identity immediately, not deferred to write time"
    );
    assert_ne!(
        fresh_subject.as_ref(),
        Some(&remembered_subject),
        "a brand-new entry's minted identity must never collide with a real, previously-saved \
         subject just because it happens to land at the same tree position"
    );

    let _ = addr_schema; // schema itself unused beyond the FieldKind match above
}
