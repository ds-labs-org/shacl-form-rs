//! Turns one [`FormSchema`] level into `Html`, recursing into every
//! `Nested` field's own level. The only thing every rendered control
//! ultimately does is call `dispatch` with a brand-new root [`FormValues`]
//! (see `crate::paths`) — this module never mutates anything in place.
use crate::controls::render_control;
use crate::paths::{Loc, add_entry, remove_entry, set_leaf};
use shacl_form_core::{Field, FieldKind, FormSchema, FormValues, ValueEntry, default_entry};
use yew::prelude::*;

pub fn render_fields(
    schema: &FormSchema,
    values: &FormValues,
    make_loc: &dyn Fn(usize) -> Loc,
    root_schema: &FormSchema,
    root_values: &FormValues,
    dispatch: Callback<FormValues>,
) -> Html {
    html! {
        <>
        { for schema.fields.iter().enumerate().map(|(idx, field)|
            render_one_field(schema, values, idx, field, make_loc, root_schema, root_values, dispatch.clone())
        ) }
        </>
    }
}

#[allow(clippy::too_many_arguments)]
fn render_one_field(
    schema: &FormSchema,
    values: &FormValues,
    idx: usize,
    field: &Field,
    make_loc: &dyn Fn(usize) -> Loc,
    root_schema: &FormSchema,
    root_values: &FormValues,
    dispatch: Callback<FormValues>,
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
                schema,
                values,
                idx,
                field,
                rep,
                entry,
                &loc,
                make_loc,
                root_schema,
                root_values,
                dispatch.clone(),
                can_remove,
            )
        })
        .collect();

    let add_button = can_add.then(|| {
        let root_schema = root_schema.clone();
        let root_values = root_values.clone();
        let loc = loc.clone();
        let field = field.clone();
        let dispatch = dispatch.clone();
        let onclick = Callback::from(move |_| dispatch.emit(add_entry(&root_schema, &root_values, &loc, default_entry(&field))));
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

#[allow(clippy::too_many_arguments)]
fn render_one_entry(
    schema: &FormSchema,
    values: &FormValues,
    field_idx: usize,
    field: &Field,
    rep: usize,
    entry: &ValueEntry,
    loc: &Loc,
    make_loc: &dyn Fn(usize) -> Loc,
    root_schema: &FormSchema,
    root_values: &FormValues,
    dispatch: Callback<FormValues>,
    can_remove: bool,
) -> Html {
    let remove_button = can_remove.then(|| {
        let root_schema = root_schema.clone();
        let root_values = root_values.clone();
        let loc = loc.clone();
        let dispatch = dispatch.clone();
        let onclick = Callback::from(move |_| {
            dispatch.emit(remove_entry(&root_schema, &root_values, &loc, rep))
        });
        html! { <button type="button" class="shacl-form-remove" onclick={onclick}>{ "✕" }</button> }
    });

    match (&field.kind, entry) {
        (
            FieldKind::Nested {
                schema: nested_schema,
            },
            ValueEntry::Nested(nested_values),
        ) => {
            let child_loc = loc.clone();
            let make_child_loc = move |inner_idx: usize| child_loc.child(rep, inner_idx);
            let _ = (schema, values, field_idx, make_loc); // this level's own coordinates aren't needed once we've descended
            html! {
                <fieldset class="shacl-form-nested">
                    { render_fields(nested_schema, nested_values, &make_child_loc, root_schema, root_values, dispatch.clone()) }
                    { remove_button }
                </fieldset>
            }
        }
        _ => {
            let root_schema = root_schema.clone();
            let root_values = root_values.clone();
            let loc = loc.clone();
            let dispatch2 = dispatch.clone();
            let onchange = Callback::from(move |v: ValueEntry| {
                dispatch2.emit(set_leaf(&root_schema, &root_values, &loc, rep, v))
            });
            html! {
                <span class="shacl-form-entry">
                    { render_control(field, Some(entry), onchange) }
                    { remove_button }
                </span>
            }
        }
    }
}
