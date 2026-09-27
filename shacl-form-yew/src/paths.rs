//! Every edit the rendered form makes goes through a [`yew::functional::Reducible`]
//! ([`FormState`]/[`FormAction`]), not a directly-captured `FormValues`
//! snapshot: Yew defers a `use_state`/`use_reducer` update to a microtask,
//! so two events handled synchronously in one JS task (a script filling
//! several inputs, a test driver) would each build on whatever `FormValues`
//! was current when *their own* closure was created — the second overwrites
//! the first's edit rather than building on it. A reducer's `reduce` is
//! instead applied to the actual current state at the moment each action is
//! processed, in the order dispatched, so this can't happen. [`Loc`] names
//! *where* an action applies; [`update_at`] is the one function that knows
//! how to get there.
use shacl_form_core::{FieldKind, FormSchema, FormValues, ValueEntry, default_entry};
use std::rc::Rc;
use yew::functional::Reducible;

/// Where one field lives, however deeply nested. `parents` descends through
/// `Nested` entries to reach the [`FormSchema`]/[`FormValues`] pair the
/// field actually belongs to; `field` is that field's index there.
///
/// `(field index, repetition index)` at each level of `parents`: a `Nested`
/// field can itself be repeated (`sh:maxCount` > 1), so reaching "the third
/// field of the second repetition of this nested field" needs both numbers,
/// not just which field.
#[derive(Clone, PartialEq, Debug, Default)]
pub struct Loc {
    pub parents: Vec<(usize, usize)>,
    pub field: usize,
}

impl Loc {
    pub fn root(field: usize) -> Self {
        Loc {
            parents: Vec::new(),
            field,
        }
    }

    pub fn child(&self, repetition: usize, field: usize) -> Self {
        let mut parents = self.parents.clone();
        parents.push((self.field, repetition));
        Loc { parents, field }
    }
}

/// The form's whole live state: the schema it was built from (`Rc`, since
/// it never changes for the lifetime of one parse — see `crate::parse_all`)
/// and the current values. One `Reducible` state rather than two separate
/// hooks so a `Reset` action (a fresh shapes graph parsed, or a different
/// instance loaded) replaces both atomically — there is never a render
/// where `values` answers for a `schema` that hasn't been swapped in yet.
#[derive(Clone, PartialEq)]
pub struct FormState {
    pub schema: Rc<FormSchema>,
    pub values: FormValues,
}

pub enum FormAction {
    Set {
        loc: Loc,
        repetition: usize,
        entry: ValueEntry,
    },
    Clear {
        loc: Loc,
        repetition: usize,
    },
    Add {
        loc: Loc,
    },
    Remove {
        loc: Loc,
        repetition: usize,
    },
    Reset {
        schema: Rc<FormSchema>,
        values: FormValues,
    },
}

impl Reducible for FormState {
    type Action = FormAction;

    fn reduce(self: Rc<Self>, action: Self::Action) -> Rc<Self> {
        match action {
            FormAction::Reset { schema, values } => Rc::new(FormState { schema, values }),
            FormAction::Set {
                loc,
                repetition,
                entry,
            } => {
                let values = set_leaf(&self.schema, &self.values, &loc, repetition, entry);
                Rc::new(FormState {
                    schema: self.schema.clone(),
                    values,
                })
            }
            FormAction::Clear { loc, repetition } => {
                let values = remove_entry(&self.schema, &self.values, &loc, repetition);
                Rc::new(FormState {
                    schema: self.schema.clone(),
                    values,
                })
            }
            FormAction::Add { loc } => {
                let Some(field) = field_at(&self.schema, &loc) else {
                    return self;
                };
                let values = add_entry(&self.schema, &self.values, &loc, default_entry(field));
                Rc::new(FormState {
                    schema: self.schema.clone(),
                    values,
                })
            }
            FormAction::Remove { loc, repetition } => {
                let values = remove_entry(&self.schema, &self.values, &loc, repetition);
                Rc::new(FormState {
                    schema: self.schema.clone(),
                    values,
                })
            }
        }
    }
}

/// Resolves the `Field` a `Loc` points at, walking through nested schemas
/// (every repetition of a `Nested` field shares the same nested schema, so
/// only the field index at each level matters here — the repetition index
/// only ever selects a *value*, which this needs none of). Needed by the
/// `Add` action, which has no existing entry to copy a `FieldKind` from and
/// must look the field up to build a blank one.
pub fn field_at<'a>(schema: &'a FormSchema, loc: &Loc) -> Option<&'a shacl_form_core::Field> {
    resolve_schema(schema, &loc.parents)?.fields.get(loc.field)
}

fn resolve_schema<'a>(
    schema: &'a FormSchema,
    parents: &[(usize, usize)],
) -> Option<&'a FormSchema> {
    match parents.split_first() {
        None => Some(schema),
        Some(((field_idx, _rep), rest)) => match schema.fields.get(*field_idx).map(|f| &f.kind) {
            Some(FieldKind::Nested { schema: nested }) => resolve_schema(nested, rest),
            _ => None,
        },
    }
}

/// Rebuilds `values` with `edit` applied to the `(schema, values)` pair
/// `parents` points at. Clones `FormValues` at each level on the way down
/// (cheap: see `shacl_form_core::FormValues`'s own doc comment — each
/// field's row is an `Rc`, so this is a handful of pointer bumps per level,
/// not a copy of the form's contents) and reuses every untouched field's
/// row and every untouched repetition's nested value unchanged.
pub fn update_at(
    schema: &FormSchema,
    values: &FormValues,
    parents: &[(usize, usize)],
    edit: impl FnOnce(&mut FormValues),
) -> FormValues {
    let mut new_values = values.clone();
    match parents.split_first() {
        None => edit(&mut new_values),
        Some(((field_idx, rep_idx), rest)) => {
            let mut entries = new_values.get(*field_idx).to_vec();
            let Some(ValueEntry::Nested {
                subject,
                values: inner_values,
            }) = entries.get(*rep_idx)
            else {
                // The path names a slot that no longer exists (its entry was
                // removed by an earlier action in the same batch) — nothing
                // to update.
                return new_values;
            };
            let Some(FieldKind::Nested {
                schema: inner_schema,
            }) = schema.fields.get(*field_idx).map(|f| &f.kind)
            else {
                return new_values;
            };
            let updated_inner = update_at(inner_schema, inner_values, rest, edit);
            entries[*rep_idx] = ValueEntry::Nested {
                subject: subject.clone(),
                values: Rc::new(updated_inner),
            };
            new_values.set(*field_idx, entries);
        }
    }
    new_values
}

/// Edits one repetition of one field — never appends. `repetition` is
/// captured at render time by whichever control's `onchange` fired, so it
/// answers for the list as it stood *then*; if a `Remove`/`Clear` from an
/// earlier action in the same dispatch batch (no re-render in between —
/// see `FormState::reduce`'s own doc comment on why a batch can contain
/// more than one action against the same pre-batch render) has since
/// shifted or shortened that list, `repetition` may no longer name the
/// entry the user actually meant to edit. Silently falling back to
/// "append a new entry" — this function's own behaviour until an audit
/// caught it — is worse than doing nothing: it fabricates a value nobody
/// asked to add, on top of leaving the entry the user actually edited
/// untouched. A no-op at least does not invent data.
pub fn set_leaf(
    schema: &FormSchema,
    values: &FormValues,
    loc: &Loc,
    repetition: usize,
    entry: ValueEntry,
) -> FormValues {
    update_at(schema, values, &loc.parents, move |vs| {
        let mut entries = vs.get(loc.field).to_vec();
        if repetition < entries.len() {
            entries[repetition] = entry;
            vs.set(loc.field, entries);
        }
    })
}

pub fn add_entry(
    schema: &FormSchema,
    values: &FormValues,
    loc: &Loc,
    default: ValueEntry,
) -> FormValues {
    update_at(schema, values, &loc.parents, move |vs| {
        let mut entries = vs.get(loc.field).to_vec();
        entries.push(default);
        vs.set(loc.field, entries);
    })
}

pub fn remove_entry(
    schema: &FormSchema,
    values: &FormValues,
    loc: &Loc,
    repetition: usize,
) -> FormValues {
    update_at(schema, values, &loc.parents, move |vs| {
        let mut entries = vs.get(loc.field).to_vec();
        if repetition < entries.len() {
            entries.remove(repetition);
        }
        vs.set(loc.field, entries);
    })
}
