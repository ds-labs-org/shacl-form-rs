//! Mounts the real `<ShaclForm>` in a real browser (see `.cargo/config.toml`
//! for the `wasm-bindgen-test-runner` wiring this needs) against a tiny
//! shapes graph, types into the rendered `<input>`, submits, and reads the
//! Turtle `onsubmit` actually received — proving the whole pipeline (parse
//! shapes -> FormSchema -> rendered control -> typed edit -> FormValues ->
//! serialised Turtle) end to end, not just that each piece compiles.
#![cfg(target_arch = "wasm32")]

use shacl_form_yew::{ShaclForm, ShaclFormProps, Target};
use std::cell::RefCell;
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
