//! The demo: a shapes-graph textarea, a target-shape field, the live
//! `<ShaclForm>` those two produce, and the Turtle it emits on submit.
mod fixtures;

use shacl_form_yew::{ShaclForm, Target};
use web_sys::{HtmlInputElement, HtmlTextAreaElement};
use yew::prelude::*;

fn textarea_value(e: InputEvent) -> String {
    e.target_dyn_into::<HtmlTextAreaElement>()
        .map(|el| el.value())
        .unwrap_or_default()
}

fn input_value(e: InputEvent) -> String {
    e.target_dyn_into::<HtmlInputElement>()
        .map(|el| el.value())
        .unwrap_or_default()
}

#[function_component(DemoApp)]
pub fn demo_app() -> Html {
    let shapes_ttl = use_state(|| AttrValue::from(fixtures::SIMPLE.shapes_ttl));
    let target_shape_iri = use_state(|| AttrValue::from(fixtures::SIMPLE.target_shape_iri));
    let submitted = use_state(|| Option::<AttrValue>::None);
    // Bumped every time the shapes graph or target changes on purpose (a
    // preset button, or the target-shape field), so `<ShaclForm>` remounts
    // with fresh internal state instead of trying to reconcile old form
    // state against a shape it was never built from.
    let generation = use_state(|| 0u32);

    let on_shapes_input = {
        let shapes_ttl = shapes_ttl.clone();
        let generation = generation.clone();
        Callback::from(move |e: InputEvent| {
            shapes_ttl.set(AttrValue::from(textarea_value(e)));
            generation.set(*generation + 1);
        })
    };
    let on_target_input = {
        let target_shape_iri = target_shape_iri.clone();
        let generation = generation.clone();
        Callback::from(move |e: InputEvent| {
            target_shape_iri.set(AttrValue::from(input_value(e)));
            generation.set(*generation + 1);
        })
    };
    let onsubmit = {
        let submitted = submitted.clone();
        Callback::from(move |turtle: AttrValue| submitted.set(Some(turtle)))
    };

    let preset_buttons = fixtures::ALL.iter().map(|preset| {
        let shapes_ttl = shapes_ttl.clone();
        let target_shape_iri = target_shape_iri.clone();
        let generation = generation.clone();
        let onclick = Callback::from(move |_| {
            shapes_ttl.set(AttrValue::from(preset.shapes_ttl));
            target_shape_iri.set(AttrValue::from(preset.target_shape_iri));
            generation.set(*generation + 1);
        });
        html! { <button type="button" onclick={onclick}>{ preset.label }</button> }
    });

    html! {
        <main>
            <h1>{ "shacl-form-rs demo" }</h1>
            <p>{ "Paste a SHACL shapes graph (Turtle), name the sh:NodeShape to render, and edit the form it produces below. \
                   Submitting shows the RDF instance data the form built — nothing is sent anywhere." }</p>
            <div class="presets">{ for preset_buttons }</div>
            <div class="columns">
                <section>
                    <label for="shapes">{ "Shapes graph (Turtle)" }</label>
                    <textarea id="shapes" rows="20" value={(*shapes_ttl).clone()} oninput={on_shapes_input} />
                    <label for="target">{ "Target sh:NodeShape IRI" }</label>
                    <input id="target" type="text" value={(*target_shape_iri).clone()} oninput={on_target_input} />
                </section>
                <section>
                    <h2>{ "Rendered form" }</h2>
                    <ShaclForm
                        key={*generation}
                        shapes_ttl={(*shapes_ttl).clone()}
                        target={Target::ShapeIri((*target_shape_iri).clone())}
                        onsubmit={onsubmit}
                    />
                </section>
            </div>
            { for submitted.as_ref().map(|turtle| html! {
                <section>
                    <h2>{ "Submitted Turtle" }</h2>
                    <pre>{ turtle }</pre>
                </section>
            }) }
        </main>
    }
}
