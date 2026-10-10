//! The text and JSON of `llm ping` and `llm models`. Pure.

use std::time::Duration;

use serde_json::{Value, json};

use super::openai::Model;

fn ms(took: Duration) -> u128 {
    took.as_millis()
}

/// `llm ping`: the server answered with `models`.
pub fn ping_text(server: &str, models: &[Model], took: Duration) -> String {
    let n = models.len();
    let s = if n == 1 { "" } else { "s" };
    format!("llm: {server} answers in {} ms ({n} model{s})", ms(took))
}

/// `llm models`: one line per model, `id  state  ctx N  capabilities`.
pub fn models_text(server: &str, models: &[Model]) -> String {
    if models.is_empty() {
        return format!("llm: {server} lists no models");
    }
    let width = models.iter().map(|m| m.id.chars().count()).max().unwrap_or(0);
    let lines: Vec<String> = models
        .iter()
        .map(|m| {
            let mut parts = vec![format!("{:width$}", m.id)];
            parts.extend(m.state.clone());
            parts.extend(m.context_length.map(|c| format!("ctx {c}")));
            if !m.capabilities.is_empty() {
                parts.push(m.capabilities.join(","));
            }
            parts.join("  ").trim_end().to_string()
        })
        .collect();
    lines.join("\n")
}

/// `llm models --json`: `{"server":..,"ms":..,"models":[{id,state,context_length,capabilities}]}`.
pub fn models_json(server: &str, models: &[Model], took: Duration) -> Value {
    json!({ "server": server, "ms": ms(took) as u64, "models": models })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str, state: Option<&str>, ctx: Option<u64>, caps: &[&str]) -> Model {
        let capabilities = caps.iter().map(|c| (*c).to_string()).collect();
        Model { id: id.into(), state: state.map(str::to_string), context_length: ctx, capabilities }
    }

    #[test]
    fn ping_counts_models() {
        let one = [model("a", None, None, &[])];
        assert_eq!(ping_text("mlx", &one, Duration::from_millis(42)), "llm: mlx answers in 42 ms (1 model)");
        assert_eq!(ping_text("mlx", &[], Duration::ZERO), "llm: mlx answers in 0 ms (0 models)");
    }

    #[test]
    fn models_line_up() {
        let list = [model("qwen3", Some("loaded"), Some(40_960), &["chat", "reasoning"]), model("g", None, None, &[])];
        assert_eq!(models_text("mlx", &list), "qwen3  loaded  ctx 40960  chat,reasoning\ng");
        assert_eq!(models_text("mlx", &[]), "llm: mlx lists no models");
        let v = models_json("mlx", &list[1..], Duration::from_millis(7));
        let want = json!({"server":"mlx","ms":7,"models":[{"id":"g","state":null,"context_length":null,"capabilities":[]}]});
        assert_eq!(v, want);
    }
}
