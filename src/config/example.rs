//! `config.example.toml`: every table and key, commented out, with its default. `flick
//! config example` prints it. A setting line is `#` then a letter or `[`; a note is `# `.
//! Each module's tests call `assert_documents` with its settings type, so the file lists
//! exactly the keys the module reads.

/// The example file, embedded.
pub const EXAMPLE: &str = include_str!("../../config.example.toml");

/// `text` with every setting line's `#` removed. Notes stay comments.
#[cfg(test)]
pub fn uncommented(text: &str) -> String {
    text.lines()
        .map(|l| match l.strip_prefix('#') {
            Some(rest) if rest.starts_with(|c: char| c.is_ascii_alphabetic() || c == '[') => rest,
            _ => l,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Assert that the example's table at `path` (`"herdr"`, or `"keys.chord"` for the first
/// entry of an array of tables) has exactly the fields of `T`, besides `enabled`, and that
/// it deserializes into `T`.
#[cfg(test)]
pub fn assert_documents<T: serde::de::DeserializeOwned>(path: &str) {
    use toml::{Table, Value};

    let mut value = Value::Table(toml::from_str::<Table>(&uncommented(EXAMPLE)).unwrap());
    for part in path.split('.') {
        value = value.get(part).cloned().unwrap_or_else(|| panic!("example lacks [{path}]"));
        if let Value::Array(items) = value {
            value = items.into_iter().next().unwrap_or_else(|| panic!("[[{path}]] is empty"));
        }
    }
    let Value::Table(mut table) = value else { panic!("example: {path} is not a table") };
    table.remove("enabled");
    let mut documented: Vec<&str> = table.keys().map(String::as_str).collect();
    let mut fields = fields::<T>().to_vec();
    documented.sort_unstable();
    fields.sort_unstable();
    assert_eq!(documented, fields, "config.example.toml [{path}] against its settings type");
    if let Err(e) = Value::Table(table).try_into::<T>() {
        panic!("example [{path}]: {e}");
    }
}

/// The field names a derived `Deserialize` asks `deserialize_struct` for.
#[cfg(test)]
fn fields<T: serde::de::DeserializeOwned>() -> &'static [&'static str] {
    use serde::de::{Deserializer, Error as _, Visitor, value::Error};

    struct Fields<'a>(&'a mut &'static [&'static str]);

    impl<'de> Deserializer<'de> for Fields<'_> {
        type Error = Error;

        fn deserialize_any<V: Visitor<'de>>(self, _: V) -> Result<V::Value, Error> {
            Err(Error::custom("not a struct"))
        }

        fn deserialize_struct<V: Visitor<'de>>(
            self,
            _: &'static str,
            fields: &'static [&'static str],
            _: V,
        ) -> Result<V::Value, Error> {
            *self.0 = fields;
            Err(Error::custom("fields taken"))
        }

        serde::forward_to_deserialize_any! {
            bool i8 i16 i32 i64 i128 u8 u16 u32 u64 u128 f32 f64 char str string bytes
            byte_buf option unit unit_struct newtype_struct seq tuple tuple_struct map enum
            identifier ignored_any
        }
    }

    let mut fields: &'static [&'static str] = &[];
    let _ = T::deserialize(Fields(&mut fields));
    assert!(!fields.is_empty(), "not a struct with named fields");
    fields
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_setting_lines_lose_their_mark() {
        let text = "# a note\n#\n#[t]\n#key = 1\n## x\nreal = 2";
        assert_eq!(uncommented(text), "# a note\n#\n[t]\nkey = 1\n## x\nreal = 2");
    }

    #[test]
    fn fields_reads_a_derived_struct() {
        #[derive(serde::Deserialize)]
        #[expect(dead_code, reason = "only the field names are read")]
        struct S {
            a: u8,
            b: String,
        }
        assert_eq!(fields::<S>(), ["a", "b"]);
    }
}
