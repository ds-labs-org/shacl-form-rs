//! Mounts the real `<ShaclForm>` in a real browser (see `.cargo/config.toml`
//! for the `wasm-bindgen-test-runner` wiring this needs) against a tiny
//! shapes graph, types into the rendered `<input>`, submits, and reads the
//! Turtle `onsubmit` actually received — proving the whole pipeline (parse
//! shapes -> FormSchema -> rendered control -> typed edit -> FormValues ->
//! serialised Turtle) end to end, not just that each piece compiles.
#![cfg(target_arch = "wasm32")]

use shacl_form_core::ValueEntry;
use shacl_form_core::oxrdf::{Literal, NamedNode as CoreNamedNode};
use shacl_form_yew::overrides::{FieldOverride, FieldOverrides, FieldSelector};
use shacl_form_yew::{ShaclForm, ShaclFormProps, Target};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_test::*;
use web_sys::{Document, Element, HtmlInputElement};
use yew::prelude::*;

wasm_bindgen_test_configure!(run_in_browser);

fn document() -> Document {
    web_sys::window()
        .expect("a window")
        .document()
        .expect("a document")
}

async fn settle() {
    gloo_timers::future::TimeoutFuture::new(0).await;
}

const SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .
ex:PersonShape a sh:NodeShape ;
  sh:property [ sh:path ex:name ; sh:name "Name" ; sh:datatype xsd:string ; sh:minCount 1 ; sh:maxCount 1 ] .
"#;

#[wasm_bindgen_test]
async fn typing_a_name_and_submitting_emits_it_as_turtle() {
    let container: Element = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();

    let received: Rc<RefCell<Option<AttrValue>>> = Rc::new(RefCell::new(None));
    let onsubmit = {
        let received = received.clone();
        Callback::from(move |turtle: AttrValue| *received.borrow_mut() = Some(turtle))
    };

    let props = ShaclFormProps {
        shapes_ttl: AttrValue::from(SHAPES),
        target: Target::ShapeIri(AttrValue::from("http://example.org/PersonShape")),
        instance_ttl: None,
        instance_subject_iri: Some(AttrValue::from("http://example.org/alice")),
        new_subject_iri: None,
        field_overrides: None,
        onsubmit,
    };

    let _handle =
        yew::Renderer::<ShaclForm>::with_root_and_props(container.clone(), props).render();
    settle().await;
    settle().await;

    let input = container
        .query_selector("input[type=text]")
        .unwrap()
        .unwrap_or_else(|| {
            panic!(
                "the Name field rendered a text input; container was:\n{}",
                container.inner_html()
            )
        })
        .dyn_into::<HtmlInputElement>()
        .unwrap();
    input.set_value("Alice");
    let event = web_sys::Event::new("input").unwrap();
    input.dispatch_event(&event).unwrap();
    settle().await;

    let form = container
        .query_selector("form")
        .unwrap()
        .expect("the form rendered")
        .dyn_into::<web_sys::HtmlFormElement>()
        .unwrap();
    // requestSubmit (not .submit()) is what actually fires the `submit`
    // event and this component's `onsubmit` handler — plain `.submit()`
    // bypasses event handlers entirely by spec.
    form.request_submit().unwrap();
    settle().await;
    settle().await;

    let turtle = received.borrow().clone().expect("onsubmit must have fired");
    assert!(turtle.contains("Alice"), "{turtle}");

    document().body().unwrap().remove_child(&container).unwrap();
}

const OVERRIDE_SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .
ex:S a sh:NodeShape ;
  sh:property [ sh:path ex:email ; sh:name "Email" ; sh:datatype xsd:string ; sh:minCount 1 ] .
"#;

/// End-to-end proof of the `field_overrides` feature: a host-supplied
/// component replaces the built-in control for `ex:email`, and its own
/// validator (on top of `check_constraints`) blocks submission for a value
/// that doesn't look like an email address — something native HTML5
/// constraint validation has no way to do here, since the override's
/// control isn't a real `<input type=email>` the browser itself validates.
#[wasm_bindgen_test]
async fn a_field_override_replaces_the_control_and_its_validator_blocks_submission() {
    let container: Element = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();

    let received: Rc<RefCell<Option<AttrValue>>> = Rc::new(RefCell::new(None));
    let onsubmit = {
        let received = received.clone();
        Callback::from(move |turtle: AttrValue| *received.borrow_mut() = Some(turtle))
    };

    let mut overrides: FieldOverrides = HashMap::new();
    overrides.insert(
        FieldSelector::Path(CoreNamedNode::new("http://example.org/email").unwrap()),
        FieldOverride {
            render: Rc::new(|_field, current, onchange| {
                let value = match current {
                    Some(ValueEntry::Literal(l)) => l.value().to_string(),
                    _ => String::new(),
                };
                let oninput = onchange.reform(|e: InputEvent| {
                    let value = e
                        .target_dyn_into::<HtmlInputElement>()
                        .map(|el| el.value())
                        .unwrap_or_default();
                    ValueEntry::Literal(Literal::new_simple_literal(value))
                });
                html! { <input type="text" class="custom-email" value={value} oninput={oninput} /> }
            }),
            validate: Some(Rc::new(|entry| {
                let ValueEntry::Literal(l) = entry else {
                    return Ok(());
                };
                if l.value().contains('@') {
                    Ok(())
                } else {
                    Err("must contain @".to_string())
                }
            })),
        },
    );

    let props = ShaclFormProps {
        shapes_ttl: AttrValue::from(OVERRIDE_SHAPES),
        target: Target::ShapeIri(AttrValue::from("http://example.org/S")),
        instance_ttl: None,
        instance_subject_iri: Some(AttrValue::from("http://example.org/x")),
        new_subject_iri: None,
        field_overrides: Some(Rc::new(overrides)),
        onsubmit,
    };
    let _handle =
        yew::Renderer::<ShaclForm>::with_root_and_props(container.clone(), props).render();
    settle().await;
    settle().await;

    // The custom control rendered, not the built-in text input.
    let input = container
        .query_selector("input.custom-email")
        .unwrap()
        .expect("the override's own control rendered")
        .dyn_into::<HtmlInputElement>()
        .unwrap();

    let form = container
        .query_selector("form")
        .unwrap()
        .expect("the form rendered")
        .dyn_into::<web_sys::HtmlFormElement>()
        .unwrap();

    // An invalid value: the override's own validator must block submission.
    input.set_value("not-an-email");
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    settle().await;
    form.request_submit().unwrap();
    settle().await;
    settle().await;

    assert!(
        received.borrow().is_none(),
        "onsubmit must not fire while the override's validator rejects the current value"
    );
    let error_text = container
        .query_selector(".shacl-form-error")
        .unwrap()
        .expect("a validation error must be shown")
        .text_content()
        .unwrap_or_default();
    assert!(
        error_text.contains("Email") && error_text.contains('@'),
        "{error_text}"
    );

    // A valid value: submission must now succeed.
    input.set_value("alice@example.org");
    input
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    settle().await;
    form.request_submit().unwrap();
    settle().await;
    settle().await;

    let turtle = received
        .borrow()
        .clone()
        .expect("onsubmit must fire once the override's validator accepts the value");
    assert!(turtle.contains("alice@example.org"), "{turtle}");

    document().body().unwrap().remove_child(&container).unwrap();
}

const SELECT_SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix ex: <http://example.org/> .
ex:S a sh:NodeShape ;
  sh:property [ sh:path ex:choice ; sh:name "Choice" ; sh:in ( "chat"@fr "chat"@en ) ] .
"#;

/// Known-limitations pass: two `sh:in` terms that stringify identically
/// but are different terms (here, the same word tagged `@fr` vs `@en`)
/// used to be indistinguishable to the rendered `<select>` — matched by
/// lexical string alone, so the *second* option could never actually be
/// selected (picking it in the DOM still resolved back to the first). This
/// reproduces the exact user-facing symptom: read an instance whose value
/// is the *second* option, confirm the DOM shows the second `<option>`
/// selected (not the first, which the old lexical-only match would show
/// regardless of the real value), then pick the *first* option and submit,
/// confirming the *first* term's own language tag makes it to the output —
/// proving both reading and writing tell the two options apart.
#[wasm_bindgen_test]
async fn two_sh_in_options_with_the_same_lexical_string_are_told_apart() {
    let container: Element = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();

    let received: Rc<RefCell<Option<AttrValue>>> = Rc::new(RefCell::new(None));
    let onsubmit = {
        let received = received.clone();
        Callback::from(move |turtle: AttrValue| *received.borrow_mut() = Some(turtle))
    };
    let props = ShaclFormProps {
        shapes_ttl: AttrValue::from(SELECT_SHAPES),
        target: Target::ShapeIri(AttrValue::from("http://example.org/S")),
        instance_ttl: Some(AttrValue::from(
            r#"@prefix ex: <http://example.org/> . ex:x ex:choice "chat"@en ."#,
        )),
        instance_subject_iri: Some(AttrValue::from("http://example.org/x")),
        new_subject_iri: None,
        field_overrides: None,
        onsubmit,
    };
    let _handle =
        yew::Renderer::<ShaclForm>::with_root_and_props(container.clone(), props).render();
    settle().await;
    settle().await;

    let select = container
        .query_selector("select")
        .unwrap()
        .expect("the Choice field rendered a select")
        .dyn_into::<web_sys::HtmlSelectElement>()
        .unwrap();
    let option_count = container.query_selector_all("option").unwrap().length();
    assert_eq!(option_count, 3, "blank + two real options");
    assert_eq!(
        select.value(),
        "1",
        "the instance's own value is the @en option (internal index 1) — the @fr option \
         (index 0) must not appear selected just because it stringifies the same"
    );

    // Now pick the FIRST real option (@fr, value "0") and submit.
    select.set_value("0");
    select
        .dispatch_event(&web_sys::Event::new("change").unwrap())
        .unwrap();
    settle().await;

    let form = container
        .query_selector("form")
        .unwrap()
        .expect("the form rendered")
        .dyn_into::<web_sys::HtmlFormElement>()
        .unwrap();
    form.request_submit().unwrap();
    settle().await;
    settle().await;

    let turtle = received.borrow().clone().expect("onsubmit must have fired");
    assert!(
        turtle.contains("\"chat\"@fr"),
        "picking the @fr option must submit the @fr term, not the @en one it used to fall back \
         to:\n{turtle}"
    );
    assert!(
        !turtle.contains("\"chat\"@en"),
        "the old @en value must not still be asserted once @fr was chosen:\n{turtle}"
    );

    document().body().unwrap().remove_child(&container).unwrap();
}

const TWO_FIELD_SHAPES: &str = r#"
@prefix sh: <http://www.w3.org/ns/shacl#> .
@prefix xsd: <http://www.w3.org/2001/XMLSchema#> .
@prefix ex: <http://example.org/> .
ex:S a sh:NodeShape ;
  sh:property [ sh:path ex:first ; sh:name "First" ; sh:datatype xsd:string ; sh:minCount 1 ; sh:order 1 ] ;
  sh:property [ sh:path ex:second ; sh:name "Second" ; sh:datatype xsd:string ; sh:minCount 1 ; sh:order 2 ] .
"#;

/// Audit C4: Yew defers a reducer/state update to a microtask, so
/// dispatching two edits *synchronously* — both `dispatch_event` calls
/// below run in one JS task, before either's resulting re-render happens —
/// used to lose one of them when state was a directly-captured `FormValues`
/// snapshot (`use_state`): the second callback's closure had captured the
/// *pre-edit* snapshot, so `set(snapshot + edit2)` overwrote
/// `set(snapshot + edit1)` instead of building on it. A reducer applies
/// each dispatched action to whatever state is actually current when it is
/// processed, in order, so both edits must survive.
#[wasm_bindgen_test]
async fn two_edits_dispatched_before_any_rerender_both_survive() {
    let container: Element = document().create_element("div").unwrap();
    document().body().unwrap().append_child(&container).unwrap();

    let received: Rc<RefCell<Option<AttrValue>>> = Rc::new(RefCell::new(None));
    let onsubmit = {
        let received = received.clone();
        Callback::from(move |turtle: AttrValue| *received.borrow_mut() = Some(turtle))
    };
    let props = ShaclFormProps {
        shapes_ttl: AttrValue::from(TWO_FIELD_SHAPES),
        target: Target::ShapeIri(AttrValue::from("http://example.org/S")),
        instance_ttl: None,
        instance_subject_iri: Some(AttrValue::from("http://example.org/x")),
        new_subject_iri: None,
        field_overrides: None,
        onsubmit,
    };
    let _handle =
        yew::Renderer::<ShaclForm>::with_root_and_props(container.clone(), props).render();
    settle().await;
    settle().await;

    let inputs = container.query_selector_all("input[type=text]").unwrap();
    assert_eq!(
        inputs.length(),
        2,
        "expected both fields to have rendered before either is edited"
    );
    let first = inputs
        .get(0)
        .unwrap()
        .dyn_into::<HtmlInputElement>()
        .unwrap();
    let second = inputs
        .get(1)
        .unwrap()
        .dyn_into::<HtmlInputElement>()
        .unwrap();

    // Both edits happen here, synchronously, with no `.await` between them —
    // no re-render can have happened yet when the second `dispatch_event`
    // call returns.
    first.set_value("one");
    first
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();
    second.set_value("two");
    second
        .dispatch_event(&web_sys::Event::new("input").unwrap())
        .unwrap();

    settle().await;
    settle().await;

    let form = container
        .query_selector("form")
        .unwrap()
        .expect("the form rendered")
        .dyn_into::<web_sys::HtmlFormElement>()
        .unwrap();
    form.request_submit().unwrap();
    settle().await;
    settle().await;

    let turtle = received.borrow().clone().expect("onsubmit must have fired");
    assert!(turtle.contains("one"), "the first edit was lost:\n{turtle}");
    assert!(
        turtle.contains("two"),
        "the second edit was lost:\n{turtle}"
    );

    document().body().unwrap().remove_child(&container).unwrap();
}
