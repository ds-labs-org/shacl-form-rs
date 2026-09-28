//! Turns one [`FormSchema`] level into `Html`, recursing into every
//! `Nested` field's own level. Every rendered control's callback captures
//! only a [`Loc`], a repetition index, and `dispatch` (a
//! `Callback<FormAction>` — cheap to clone, and cloning it does not touch
//! `FormSchema`/`FormValues` at all) — the actual edit happens once, in
//! `FormState::reduce` (`crate::paths`), not once per callback capture.
use crate::controls::render_control;
use crate::overrides::{self, FieldOverrides};
use crate::paths::{FormAction, Loc};
use shacl_form_core::{Field, FieldKind, FormSchema, FormValues, ValueEntry};
use yew::prelude::*;

pub fn render_fields(
    schema: &FormSchema,
    values: &FormValues,
    make_loc: &dyn Fn(usize) -> Loc,
    dispatch: Callback<FormAction>,
    overrides: Option<&FieldOverrides>,
) -> Html {
    html! {
        <>
        { for schema.fields.iter().enumerate().map(|(idx, field)|
            render_one_field(values, idx, field, make_loc, dispatch.clone(), overrides)
        ) }
        </>
    }
}

fn render_one_field(
    values: &FormValues,
    idx: usize,
    field: &Field,
    make_loc: &dyn Fn(usize) -> Loc,
    dispatch: Callback<FormAction>,
    overrides: Option<&FieldOverrides>,
) -> Html {
    let loc = make_loc(idx);
    let entries = values.get(idx);
    let can_add = field
        .max_count
        .is_none_or(|max| (entries.len() as u32) < max);
    let can_remove = (entries.len() as u32) > field.min_count;

    let rows: Vec<Html> = entries
        .iter()
        .enumerate()
        .map(|(rep, entry)| {
            render_one_entry(
                field,
                rep,
                entry,
                &loc,
                dispatch.clone(),
                can_remove,
                overrides,
            )
        })
        .collect();

    let add_button = can_add.then(|| {
        let loc = loc.clone();
        let dispatch = dispatch.clone();
        let onclick = Callback::from(move |_| dispatch.emit(FormAction::Add { loc: loc.clone() }));
        html! { <button type="button" class="shacl-form-add" onclick={onclick}>{ "+ Add" }</button> }
    });

    html! {
        <div class="shacl-form-field">
            <label>{ &field.label }
                { if field.min_count > 0 { html!{<span class="shacl-form-required">{" *"}</span>} } else { html!{} } }
            </label>
            { for field.description.as_ref().map(|d| html!{ <p class="shacl-form-description">{ d }</p> }) }
            { for rows }
            { for add_button }
            { for field.unsupported.as_ref().map(|u| html!{ <p class="shacl-form-unsupported" title={u.clone()}>{ "⚠ not fully supported by this generated form" }</p> }) }
        </div>
    }
}

fn render_one_entry(
    field: &Field,
    rep: usize,
    entry: &ValueEntry,
    loc: &Loc,
    dispatch: Callback<FormAction>,
    can_remove: bool,
    overrides: Option<&FieldOverrides>,
) -> Html {
    let remove_button = can_remove.then(|| {
        let loc = loc.clone();
        let dispatch = dispatch.clone();
        let onclick = Callback::from(move |_| {
            dispatch.emit(FormAction::Remove {
                loc: loc.clone(),
                repetition: rep,
            })
        });
        html! { <button type="button" class="shacl-form-remove" onclick={onclick}>{ "✕" }</button> }
    });

    match (&field.kind, entry) {
        (
            FieldKind::Nested {
                schema: nested_schema,
            },
            ValueEntry::Nested {
                values: nested_values,
                ..
            },
        ) => {
            let child_loc = loc.clone();
            let make_child_loc = move |inner_idx: usize| child_loc.child(rep, inner_idx);
            html! {
                <fieldset class="shacl-form-nested" key={rep}>
                    { render_fields(nested_schema, nested_values, &make_child_loc, dispatch.clone(), overrides) }
                    { remove_button }
                </fieldset>
            }
        }
        _ => {
            let loc_set = loc.clone();
            let dispatch_set = dispatch.clone();
            let onchange = Callback::from(move |v: ValueEntry| {
                dispatch_set.emit(FormAction::Set {
                    loc: loc_set.clone(),
                    repetition: rep,
                    entry: v,
                })
            });
            let loc_clear = loc.clone();
            let on_clear = Callback::from(move |()| {
                dispatch.emit(FormAction::Clear {
                    loc: loc_clear.clone(),
                    repetition: rep,
                })
            });
            // A host-supplied override takes over just the control itself
            // (this crate still rendered the label/description/required-
            // marker above, and still renders the remove button below) —
            // see `overrides`'s own doc comment for why.
            let control = match overrides.and_then(|o| overrides::lookup(o, field)) {
                Some(over) => (over.render)(field, Some(entry), onchange),
                None => render_control(field, Some(entry), onchange, on_clear),
            };
            html! {
                <span class="shacl-form-entry" key={rep}>
                    { control }
                    { remove_button }
                </span>
            }
        }
    }
}
