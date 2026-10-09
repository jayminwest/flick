//! Forms: a module-owned set of labelled text fields that replaces root search, e.g. the
//! quicklink editor. The module builds the form (`Module::form`) and saves it
//! (`Module::submit`); the controller only edits values and moves focus.

/// One labelled text field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// The module's name for the field; `Form::value` looks it up.
    pub key: &'static str,
    pub label: String,
    pub value: String,
    /// Shown while `value` is empty.
    pub placeholder: String,
    /// Submit refuses while the value is blank.
    pub required: bool,
}

impl Field {
    /// An optional, empty field with no placeholder.
    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    pub fn new(key: &'static str, label: impl Into<String>) -> Field {
        Field {
            key,
            label: label.into(),
            value: String::new(),
            placeholder: String::new(),
            required: false,
        }
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    #[must_use]
    pub fn required(mut self) -> Field {
        self.required = true;
        self
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Field {
        self.value = value.into();
        self
    }

    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    #[must_use]
    pub fn placeholder(mut self, placeholder: impl Into<String>) -> Field {
        self.placeholder = placeholder.into();
        self
    }

    /// Required, and blank after trimming.
    pub fn is_missing(&self) -> bool {
        self.required && self.value.trim().is_empty()
    }
}

/// A module's form, by (`module`, `name`). Form names are their own namespace, apart from
/// list view names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Form {
    /// The module that owns the form and saves it.
    pub module: &'static str,
    pub name: String,
    pub title: String,
    pub fields: Vec<Field>,
    /// The index of the field with keyboard focus.
    pub focused: usize,
    /// What Enter does, shown in the footer, e.g. "Save Quicklink".
    pub submit_label: String,
    /// The last submit's error, shown under the fields.
    pub error: Option<String>,
}

impl Form {
    /// A form with no fields, focus on the first, "Submit" as its label and no error.
    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    pub fn new(module: &'static str, name: impl Into<String>, title: impl Into<String>) -> Form {
        Form {
            module,
            name: name.into(),
            title: title.into(),
            fields: vec![],
            focused: 0,
            submit_label: "Submit".into(),
            error: None,
        }
    }

    /// Same module and name.
    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    pub fn is(&self, module: &str, name: &str) -> bool {
        self.module == module && self.name == name
    }

    /// Move focus to the next field, from the last back to the first.
    pub fn focus_next(&mut self) {
        if !self.fields.is_empty() {
            self.focused = (self.focused + 1) % self.fields.len();
        }
    }

    /// Move focus to the previous field, from the first round to the last.
    pub fn focus_prev(&mut self) {
        if !self.fields.is_empty() {
            self.focused = (self.focused + self.fields.len() - 1) % self.fields.len();
        }
    }

    /// Set field `index`'s value. An index past the last field does nothing.
    pub fn set_value(&mut self, index: usize, text: impl Into<String>) {
        if let Some(field) = self.fields.get_mut(index) {
            field.value = text.into();
        }
    }

    /// The value of the field with `key`, untrimmed. `None` when there is no such field.
    #[cfg_attr(not(test), expect(dead_code, reason = "modules call it from flick-37d5 on"))]
    pub fn value(&self, key: &str) -> Option<&str> {
        self.fields.iter().find(|f| f.key == key).map(|f| f.value.as_str())
    }

    /// The required fields that are still blank, in order.
    pub fn missing_required(&self) -> Vec<&Field> {
        self.fields.iter().filter(|f| f.is_missing()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form() -> Form {
        Form {
            fields: vec![
                Field::new("name", "Name").required(),
                Field::new("url", "URL").required().placeholder("https://…"),
                Field::new("keyword", "Keyword").value("k"),
            ],
            ..Form::new("quicklink", "new", "Create Quicklink")
        }
    }

    #[test]
    fn new_form_and_field_defaults() {
        let f = Form::new("quicklink", "new", "Create Quicklink");
        assert!(f.is("quicklink", "new"));
        assert!(!f.is("quicklink", "edit") && !f.is("clip", "new"));
        assert_eq!((f.title.as_str(), f.submit_label.as_str()), ("Create Quicklink", "Submit"));
        assert!(f.fields.is_empty() && f.focused == 0 && f.error.is_none());
        let field = Field::new("k", "Key");
        assert_eq!(field.label, "Key");
        assert!(field.value.is_empty() && field.placeholder.is_empty() && !field.required);
    }

    #[test]
    fn focus_wraps_both_ways() {
        let mut f = form();
        f.focus_prev();
        assert_eq!(f.focused, 2);
        f.focus_next();
        assert_eq!(f.focused, 0);
        f.focus_next();
        f.focus_next();
        assert_eq!(f.focused, 2);
        f.focus_prev();
        assert_eq!(f.focused, 1);
    }

    #[test]
    fn focus_on_an_empty_form_stays_put() {
        let mut f = Form::new("m", "n", "t");
        f.focus_next();
        f.focus_prev();
        assert_eq!(f.focused, 0);
    }

    #[test]
    fn values_set_and_read_by_key() {
        let mut f = form();
        assert_eq!(f.value("keyword"), Some("k"));
        assert_eq!(f.value("url"), Some(""));
        assert_eq!(f.value("nope"), None);
        f.set_value(1, "https://a.b");
        f.set_value(9, "ignored");
        assert_eq!(f.value("url"), Some("https://a.b"));
        assert_eq!(f.fields[1].placeholder, "https://…");
    }

    #[test]
    fn missing_required_ignores_optional_and_counts_blank() {
        let mut f = form();
        let keys = |f: &Form| f.missing_required().iter().map(|x| x.key).collect::<Vec<_>>();
        assert_eq!(keys(&f), ["name", "url"]);
        f.set_value(0, "  \t");
        assert_eq!(keys(&f), ["name", "url"]);
        f.set_value(0, " Docs ");
        f.set_value(1, "x");
        f.set_value(2, "");
        assert!(keys(&f).is_empty());
    }
}
