use serde_json::{Value, json};

use super::{ExtField, ExtForm, SUBMIT};
use crate::modules::ext::proto::{FIELDS_MAX, Op, Reply, VALUE_MAX, parse};

fn form(v: &Value) -> Result<(ExtForm, Vec<String>), String> {
    let p = parse(Op::Form, v.to_string().as_bytes())?;
    let Reply::Form(f) = p.reply else { panic!("not a form: {:?}", p.reply) };
    Ok((f, p.warnings))
}

#[test]
fn a_form_parses() {
    let v = json!({ "title": "New Task", "submit": "Create", "fields": [
        { "key": "title", "label": "Title", "value": "x", "placeholder": "What", "required": true },
        { "key": "notes", "label": "Notes", "value": "a\nb\tc", "multiline": true },
    ]});
    let (got, w) = form(&v).unwrap();
    assert!(w.is_empty(), "{w:?}");
    let title = ExtField {
        key: "title".into(),
        label: "Title".into(),
        value: "x".into(),
        placeholder: "What".into(),
        required: true,
        multiline: false,
    };
    let notes = ExtField {
        key: "notes".into(),
        label: "Notes".into(),
        value: "a\nb\tc".into(),
        placeholder: String::new(),
        required: false,
        multiline: true,
    };
    let want =
        ExtForm { title: "New Task".into(), submit: "Create".into(), fields: vec![title, notes] };
    assert_eq!(got, want);
    let (got, _) =
        form(&json!({ "title": "T", "submit": " ", "fields": [{ "key": "a", "label": "A" }] }))
            .unwrap();
    assert_eq!(got.submit, SUBMIT);
}

#[test]
fn bad_forms_and_fields() {
    let errors = [
        (json!({ "fields": [{ "key": "a", "label": "A" }] }), "no title"),
        (json!({ "title": "T" }), "form has no valid field"),
        (json!({ "title": "T", "fields": "a" }), "fields is not a list"),
    ];
    for (v, want) in errors {
        assert_eq!(form(&v).unwrap_err(), want, "{v}");
    }
    let skipped = [
        (json!("a"), "not an object"),
        (json!({ "label": "A" }), "key must be 1-64 of A-Z a-z 0-9 . _ -"),
        (json!({ "key": "a" }), "no label"),
        (json!({ "key": "a", "label": "A", "value": "a\nb" }), "value has a control character"),
        (
            json!({ "key": "a", "label": "A", "value": "\u{1}", "multiline": true }),
            "value has a control character",
        ),
        (
            json!({ "key": "a", "label": "A", "placeholder": "\u{2028}" }),
            "placeholder has a control character",
        ),
    ];
    for (f, why) in skipped {
        let (got, w) =
            form(&json!({ "title": "T", "fields": [f, { "key": "ok", "label": "Ok" }] })).unwrap();
        assert_eq!(w, [format!("fields[0]: {why}; skipped")], "{f}");
        assert_eq!(got.fields.len(), 1);
    }
    let many: Vec<Value> =
        (0..=FIELDS_MAX).map(|i| json!({ "key": format!("f{i}"), "label": "F" })).collect();
    let (got, w) = form(&json!({ "title": "T", "fields": many })).unwrap();
    assert_eq!(got.fields.len(), FIELDS_MAX);
    assert_eq!(w, ["more than 6 fields; fields[6] and later dropped"]);
    let (got, w) =
        form(&json!({ "title": "T", "fields": [{ "key": "a", "label": "A", "required": 1 }] }))
            .unwrap();
    assert!(!got.fields[0].required);
    assert_eq!(w, ["fields[0]: required is not true or false; false used"]);
    let long = "v".repeat(VALUE_MAX + 1);
    let (got, _) =
        form(&json!({ "title": "T", "fields": [{ "key": "a", "label": "A", "value": long }] }))
            .unwrap();
    assert_eq!(got.fields[0].value.chars().count(), VALUE_MAX);
}
