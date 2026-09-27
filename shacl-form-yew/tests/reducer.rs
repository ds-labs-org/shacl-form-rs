//! `FormState::reduce` is plain Rust logic with no DOM/wasm dependency, so
//! it's tested directly here on the host, not through `tests/dom.rs`'s
//! browser harness — much faster, and these are exactly the cases that
//! don't need a real `<input>` to reproduce.
use shacl_form_core::oxrdf::{Literal, NamedNode};
use shacl_form_core::{FormValues, ValueEntry, from_shape_iri, parse_turtle};
use shacl_form_yew::paths::{FormAction, FormState, Loc};
use std::rc::Rc;
use yew::functional::Reducible;

const REPEATABLE: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .
ex:S a sh:NodeShape ;
  sh:property [ sh:path ex:tag ; sh:name "tag" ; sh:datatype xsd:string ] .
"#;

fn initial_state() -> Rc<FormState> {
    let shapes = parse_turtle(REPEATABLE).unwrap();
    let schema = from_shape_iri(&shapes, &NamedNode::new("http://example.org/S").unwrap()).unwrap();
    let mut values = FormValues::new_for(&schema);
    // Two repetitions of the one field, "a" and "b".
    values.set(
        0,
        vec![
            ValueEntry::Literal(Literal::new_simple_literal("a")),
            ValueEntry::Literal(Literal::new_simple_literal("b")),
        ],
    );
    Rc::new(FormState {
        schema: Rc::new(schema),
        values,
    })
}

fn tags(state: &FormState) -> Vec<String> {
    state
        .values
        .get(0)
        .iter()
        .map(|e| match e {
            ValueEntry::Literal(l) => l.value().to_string(),
            other => panic!("expected Literal, got {other:?}"),
        })
        .collect()
}

/// Fable audit finding 3: a `Remove` at index 0, followed — in the same
/// dispatch batch, i.e. both actions built from the *same* pre-removal
/// render, exactly like two Yew event handlers firing in one JS task — by
/// a `Set` at index 1 (valid *before* the removal, stale *after* it: the
/// survivor "b" is now at index 0). `set_leaf`'s own "index out of range →
/// push a new entry" fallback used to turn that stale `Set` into a
/// fabricated third entry instead of either editing the survivor or being
/// a safe no-op.
#[test]
fn a_set_at_a_stale_post_removal_index_does_not_fabricate_a_phantom_entry() {
    let state = initial_state();
    assert_eq!(tags(&state), ["a", "b"]);

    let loc = Loc::root(0);
    let after_remove = state.reduce(FormAction::Remove {
        loc: loc.clone(),
        repetition: 0,
    });
    assert_eq!(
        tags(&after_remove),
        ["b"],
        "sanity: the remove itself worked"
    );

    // The stale action: built against the ORIGINAL two-entry state (as if
    // rendered before the remove was processed), so it still says
    // repetition 1 — which no longer exists once "a" is gone.
    let stale_set = FormAction::Set {
        loc,
        repetition: 1,
        entry: ValueEntry::Literal(Literal::new_simple_literal("b-edited")),
    };
    let final_state = after_remove.reduce(stale_set);

    // Two acceptable outcomes for "the index no longer means anything":
    // either it's a no-op (still just ["b"]), or it edits the one
    // remaining entry. What must NOT happen is ending up with two entries
    // — "b" untouched AND "b-edited" appended as a fabricated new one,
    // which is what `set_leaf`'s old push-on-out-of-range fallback did.
    let result = tags(&final_state);
    assert_eq!(
        result.len(),
        1,
        "a stale Set must not fabricate a new entry: got {result:?}"
    );
}
