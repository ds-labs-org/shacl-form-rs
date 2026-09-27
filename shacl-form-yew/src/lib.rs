//! `<ShaclForm>`: given a SHACL shapes graph and a target shape, renders a
//! real HTML form for it and emits the filled-in RDF instance data on
//! submit. All the SHACL reading lives in [`shacl_form_core`]; this crate
//! only renders [`shacl_form_core::FormSchema`] and turns DOM events back
//! into [`shacl_form_core::ValueEntry`] values.
mod controls;
mod paths;
mod render;

use paths::{FormAction, FormState, Loc};
use shacl_form_core::oxrdf::{BlankNode, NamedNode, NamedOrBlankNode};
use shacl_form_core::{
    FormSchema, FormValues, ShapesError, from_shape_iri, from_target_class, parse_instance_turtle,
    parse_turtle,
};
use std::rc::Rc;
use yew::prelude::*;

/// Whether `target` names a shape directly or a class every matching
/// `sh:NodeShape` targets — see `shacl_form_core::from_shape_iri` vs.
/// `from_target_class` for what each does with more than one candidate
/// shape.
#[derive(Clone, PartialEq)]
pub enum Target {
    ShapeIri(AttrValue),
    TargetClass(AttrValue),
}

#[derive(Properties, PartialEq, Clone)]
pub struct ShaclFormProps {
    pub shapes_ttl: AttrValue,
    pub target: Target,
    /// An existing instance to edit, and the subject (as a full IRI) it is
    /// asserted about. Omit both to create a new instance instead, whose
    /// subject is a freshly minted blank node unless `new_subject_iri` says
    /// otherwise.
    #[prop_or_default]
    pub instance_ttl: Option<AttrValue>,
    #[prop_or_default]
    pub instance_subject_iri: Option<AttrValue>,
    #[prop_or_default]
    pub new_subject_iri: Option<AttrValue>,
    /// Called with the produced Turtle when the form is submitted. This
    /// component never sends it anywhere itself — see the workspace
    /// README's scope note on why persistence is a host concern.
    pub onsubmit: Callback<AttrValue>,
}

/// What actually varies `parse_all`'s result — `use_memo`/`use_effect_with`
/// key on this small, cheaply-`PartialEq`-comparable tuple rather than on
/// the `Parsed` result itself: `FormSchema`/`FormValues` derive `PartialEq`
/// structurally, so comparing two `Rc<Parsed>` would walk the *whole*
/// (shared, but still large-once-traversed — see `shacl_form_core`'s own
/// `ShapeCache` doc comment) schema tree on every render just to answer
/// "did the props change", which is exactly the cost the cache exists to
/// avoid paying twice.
type Deps = (
    AttrValue,
    Target,
    Option<AttrValue>,
    Option<AttrValue>,
    Option<AttrValue>,
);

fn deps_of(props: &ShaclFormProps) -> Deps {
    (
        props.shapes_ttl.clone(),
        props.target.clone(),
        props.instance_ttl.clone(),
        props.instance_subject_iri.clone(),
        props.new_subject_iri.clone(),
    )
}

type Parsed = Result<(FormSchema, FormValues, NamedOrBlankNode), String>;

fn parse_all(props: &ShaclFormProps) -> Parsed {
    let shapes = parse_turtle(&props.shapes_ttl).map_err(|e| e.to_string())?;
    let schema = match &props.target {
        Target::ShapeIri(iri) => {
            let iri = NamedNode::new(iri.as_str()).map_err(|e| e.to_string())?;
            from_shape_iri(&shapes, &iri)
        }
        Target::TargetClass(iri) => {
            let iri = NamedNode::new(iri.as_str()).map_err(|e| e.to_string())?;
            from_target_class(&shapes, &iri)
        }
    }
    .map_err(|e: ShapesError| e.to_string())?;

    let subject = match &props.instance_subject_iri {
        Some(iri) => {
            NamedOrBlankNode::NamedNode(NamedNode::new(iri.as_str()).map_err(|e| e.to_string())?)
        }
        None => match &props.new_subject_iri {
            Some(iri) => NamedOrBlankNode::NamedNode(
                NamedNode::new(iri.as_str()).map_err(|e| e.to_string())?,
            ),
            None => NamedOrBlankNode::BlankNode(BlankNode::default()),
        },
    };

    let values = match &props.instance_ttl {
        Some(ttl) => {
            let instance = parse_instance_turtle(ttl).map_err(|e| e.to_string())?;
            FormValues::read_from_instance(&schema, &instance, subject.as_ref().into())
        }
        None => FormValues::new_for(&schema),
    };

    Ok((schema, values, subject))
}

#[function_component(ShaclForm)]
pub fn shacl_form(props: &ShaclFormProps) -> Html {
    let deps = deps_of(props);
    let parsed = use_memo(deps.clone(), {
        let props = props.clone();
        move |_| parse_all(&props)
    });

    let state = use_reducer(|| FormState {
        schema: Rc::new(FormSchema::default()),
        values: FormValues::default(),
    });
    let subject = use_state(|| None::<NamedOrBlankNode>);
    {
        let state = state.clone();
        let parsed = parsed.clone();
        let subject = subject.clone();
        // Keyed on `deps`, not on `parsed` itself — see `Deps`'s own doc
        // comment. Runs once per genuine input change (a new shapes graph,
        // a different target, a different instance to edit), never once
        // per keystroke: keystrokes only ever dispatch `Set`/`Add`/`Remove`
        // against the reducer, which this effect has no part in.
        use_effect_with(deps, move |_| {
            if let Ok((schema, initial, subj)) = parsed.as_ref() {
                subject.set(Some(subj.clone()));
                state.dispatch(FormAction::Reset {
                    schema: Rc::new(schema.clone()),
                    values: initial.clone(),
                });
            }
        });
    }

    match parsed.as_ref() {
        Err(message) => html! {
            <p class="shacl-form-error">{ format!("could not build a form from this shapes graph: {message}") }</p>
        },
        Ok(_) => {
            let dispatch = {
                let state = state.clone();
                Callback::from(move |action: FormAction| state.dispatch(action))
            };
            let onsubmit = {
                let state = state.clone();
                let subject = subject.clone();
                let cb = props.onsubmit.clone();
                Callback::from(move |e: SubmitEvent| {
                    e.prevent_default();
                    let Some(subject) = subject.as_ref() else {
                        return;
                    };
                    let turtle = state.values.to_turtle(&state.schema, subject, &[]);
                    cb.emit(AttrValue::from(turtle));
                })
            };
            let root_loc = Loc::root;
            html! {
                <form class="shacl-form" onsubmit={onsubmit}>
                    { render::render_fields(&state.schema, &state.values, &root_loc, dispatch) }
                    <button type="submit">{ "Submit" }</button>
                </form>
            }
        }
    }
}
