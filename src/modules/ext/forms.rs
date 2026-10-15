//! Forms: an action `{"form": "<form>"}` opens the extension's form `<name>/<form>`. It shows
//! at once from its last answer, or empty and `Loading…` until the first (`op: form`, asked
//! each time it opens; a later answer only updates the cache, so it never clobbers what the
//! user typed). Forms are `background` (core `Form`): submit sends `op: submit` on the worker
//! and the form stays open, `Saving…`, until the reply. A reply's `status` closes it to root
//! search with that status in the footer, or toasts when the launcher is hidden; an error is
//! logged, always toasted, and shown in the form while it is still on the screen. A form the
//! user left (Escape) is never opened again: its reply only toasts.

use super::items::{self, LOADING};
use super::proto::form::ExtForm;
use super::proto::{Done, Reply, Request};
use super::Ext;
use crate::core::{Field, Form};

/// A form's footer while its submit waits for the reply.
pub const SAVING: &str = "Saving…";
/// The status of a submit whose reply names none.
pub const SAVED: &str = "Saved";

/// Where the ext form on the screen is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    /// Asked for, with nothing cached to show.
    Loading,
    /// Its first answer is in the cache: fill the form.
    Loaded,
    /// Its `form` call failed: show this in it.
    LoadFailed(String),
    /// The user edits it.
    Open,
    /// Submitted; the reply is not in.
    Saving,
    /// The reply: its status, or its error.
    Saved(Result<Option<String>, String>),
}

/// Extension form `name` (`<ext>/<form>`) as the launcher shows it.
fn build(name: &str, f: &ExtForm) -> Form {
    let field = |x: &super::proto::form::ExtField| Field {
        key: x.key.clone().into(),
        label: x.label.clone(),
        value: x.value.clone(),
        placeholder: x.placeholder.clone(),
        required: x.required,
        multiline: x.multiline,
    };
    Form {
        fields: f.fields.iter().map(field).collect(),
        submit_label: f.submit.clone(),
        background: true,
        ..Form::new("ext", name, f.title.clone())
    }
}

impl Ext {
    /// Form `name` (`<ext>/<form>`): from its cache, else empty and loading; asked again.
    pub(super) fn open_form(&mut self, name: &str) -> Option<Form> {
        let (ext, inner) = items::split(name)?;
        self.spec(ext)?;
        let ext = ext.to_string();
        self.fetch(&ext, Request::Form { form: inner.to_string() });
        let (form, phase) = if let Some(cached) = self.forms.get(name) {
            (build(name, cached), Phase::Open)
        } else {
            let pending = Some(LOADING.into());
            (Form { background: true, pending, ..Form::new("ext", name, ext) }, Phase::Loading)
        };
        self.form = Some((name.to_string(), phase));
        Some(form)
    }

    /// Extension `ext`'s answer to `form`: cached, and the form on the screen fills from it if
    /// it is still loading. True: that form changed.
    pub(super) fn loaded(&mut self, ext: &str, form: &str, result: Result<Reply, String>) -> bool {
        let name = format!("{ext}/{form}");
        let phase = match result {
            Ok(Reply::Form(f)) => {
                self.forms.insert(name.clone(), f);
                Phase::Loaded
            }
            Ok(_) => return false,
            Err(e) => {
                self.report(ext, &e);
                Phase::LoadFailed(e)
            }
        };
        match &mut self.form {
            Some((n, p)) if *n == name && *p == Phase::Loading => *p = phase,
            _ => return false,
        }
        true
    }

    /// Send `form`'s values; its `Ok` is the pending footer.
    pub(super) fn submit_form(&mut self, form: &Form) -> Result<String, String> {
        let (ext, inner) = items::split(&form.name).ok_or("ext: not an extension form")?;
        if !matches!(&self.form, Some((n, Phase::Open)) if *n == form.name) {
            return Err(format!("{ext}: {inner} is not ready"));
        }
        let values = form.fields.iter().map(|f| (f.key.to_string(), f.value.clone())).collect();
        let ext = ext.to_string();
        self.call(&ext, Request::Submit { form: inner.to_string(), values })?;
        self.form = Some((form.name.clone(), Phase::Saving));
        Ok(SAVING.into())
    }

    /// Extension `ext`'s reply to submitting `form`. True: the form on the screen waits on it.
    pub(super) fn saved(&mut self, ext: &str, form: &str, result: Result<Reply, String>) -> bool {
        let name = format!("{ext}/{form}");
        let result = match result {
            Ok(Reply::Done(Done { status, refresh })) => {
                if refresh {
                    self.refetch(ext);
                }
                Ok(status)
            }
            Ok(_) => Ok(None),
            Err(e) => {
                self.report(ext, &e);
                Err(e)
            }
        };
        let waiting = matches!(&self.form, Some((n, Phase::Saving)) if *n == name);
        if !(waiting && (self.hooks.visible)())
            && let Ok(status) = &result
        {
            (self.hooks.toast)(ext, status.as_deref().unwrap_or(SAVED));
        }
        if waiting {
            self.form = Some((name, Phase::Saved(result)));
        }
        waiting
    }

    /// Bring `form` (on the screen) up to date; `Some` closes it with a status.
    pub(super) fn fill_form(&mut self, form: &mut Form) -> Option<String> {
        let (name, phase) = self.form.as_mut().filter(|(n, _)| *n == form.name)?;
        match std::mem::replace(phase, Phase::Open) {
            Phase::Loaded => *form = build(name, self.forms.get(name.as_str())?),
            Phase::LoadFailed(e) => {
                // It never submits: there is nothing to send.
                (form.pending, form.error) = (None, Some(e.clone()));
                *phase = Phase::LoadFailed(e);
            }
            Phase::Saved(Err(e)) => (form.pending, form.error) = (None, Some(e)),
            Phase::Saved(Ok(status)) => {
                self.form = None;
                return Some(status.unwrap_or_else(|| SAVED.into()));
            }
            other => *phase = other,
        }
        None
    }

    /// `form` left the screen; a reply it still waits on only toasts.
    pub(super) fn close_form(&mut self, form: &Form) {
        self.form.take_if(|(n, _)| *n == form.name);
    }
}

#[cfg(test)]
mod tests;
