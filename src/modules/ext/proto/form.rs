//! A form in a reply to `form`: `{"title": "New Task", "submit": "Create", "fields": [ … ]}`.
//!
//! Field: `{"key": "title", "label": "Title", "value": "…", "placeholder": "…",
//! "required": true, "multiline": false}`. Only `key` and `label` are required; only a
//! multiline field's `value` may hold newlines and tabs.

use serde_json::{Map, Value};

use super::{
    FIELDS_MAX, SHORT_MAX, TITLE_MAX, VALUE_MAX, flag, hidden_multiline, list, required, text,
    text_where,
};
use crate::core::card::valid_id;

/// The submit button label when the form names none.
pub const SUBMIT: &str = "Submit";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtForm {
    pub title: String,
    pub submit: String,
    /// At least one.
    pub fields: Vec<ExtField>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtField {
    /// The key of its text in `submit`'s `values`: 1-64 of A-Z a-z 0-9 . _ -
    pub key: String,
    pub label: String,
    pub value: String,
    pub placeholder: String,
    pub required: bool,
    pub multiline: bool,
}

/// The form of the reply `o`.
pub(super) fn parse(o: &Map<String, Value>, w: &mut Vec<String>) -> Result<ExtForm, String> {
    let title = required(o, "title", TITLE_MAX, "form", w)?;
    let submit = text(o, "submit", SHORT_MAX, "form", w)?.filter(|s| !s.trim().is_empty());
    let fields = list(o.get("fields"), "fields", FIELDS_MAX, |f| &f.key, field, w)?;
    if fields.is_empty() {
        return Err("form has no valid field".into());
    }
    Ok(ExtForm { title, submit: submit.unwrap_or_else(|| SUBMIT.into()), fields })
}

fn field(v: &Value, at: &str, w: &mut Vec<String>) -> Result<ExtField, String> {
    let o = v.as_object().ok_or("not an object")?;
    let key = o.get("key").and_then(Value::as_str).filter(|k| valid_id(k));
    let key = key.ok_or("key must be 1-64 of A-Z a-z 0-9 . _ -")?.to_string();
    let label = required(o, "label", SHORT_MAX, at, w)?;
    let multiline = flag(o, "multiline", at, w);
    let bad: fn(char) -> bool = if multiline { hidden_multiline } else { super::hidden };
    let value = text_where(o, "value", VALUE_MAX, at, w, bad)?.unwrap_or_default();
    let placeholder = text(o, "placeholder", SHORT_MAX, at, w)?.unwrap_or_default();
    Ok(ExtField { key, label, value, placeholder, required: flag(o, "required", at, w), multiline })
}

#[cfg(test)]
mod tests;
