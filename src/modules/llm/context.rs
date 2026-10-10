//! What of a normal chat goes to the model (flick-5dfe), pure: the newest turns that fit
//! `[llm] context_chars`, counted in characters with the system prompt. The prompt just sent
//! always goes, even alone past the budget, and what is sent never starts with a reply (chat
//! templates expect a user turn first). The thread itself keeps every message.

use super::openai::{Role, Turn};

/// The turns of `turns` sent with `system` under `budget` characters (0: all of them).
pub fn fit<'a, 'b>(turns: &'a [Turn<'b>], system: &str, budget: usize) -> &'a [Turn<'b>] {
    if budget == 0 {
        return turns;
    }
    let mut used = system.chars().count();
    let mut start = turns.len();
    for (i, t) in turns.iter().enumerate().rev() {
        used = used.saturating_add(t.content.chars().count());
        if used > budget && start < turns.len() {
            break;
        }
        start = i;
    }
    while start + 1 < turns.len() && turns[start].role == Role::Assistant {
        start += 1;
    }
    &turns[start..]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turns(texts: &[&'static str]) -> Vec<Turn<'static>> {
        let role = |i: usize| if i.is_multiple_of(2) { Role::User } else { Role::Assistant };
        texts.iter().enumerate().map(|(i, t)| Turn { role: role(i), content: t }).collect()
    }

    fn sent(texts: &[&'static str], system: &str, budget: usize) -> Vec<&'static str> {
        fit(&turns(texts), system, budget).iter().map(|t| t.content).collect()
    }

    #[test]
    fn zero_sends_the_whole_chat() {
        assert_eq!(sent(&["aaaa", "bbbb", "cccc"], "sys", 0), ["aaaa", "bbbb", "cccc"]);
        assert!(sent(&[], "sys", 0).is_empty());
        assert!(sent(&[], "sys", 5).is_empty());
    }

    #[test]
    fn the_newest_turns_that_fit_go_from_a_user_turn() {
        let chat = ["aaaa", "bbbb", "cccc", "dddd", "eeee"];
        assert_eq!(sent(&chat, "", 100), chat);
        assert_eq!(sent(&chat, "", 20), chat);
        // 12 fits "cccc dddd eeee", which starts with a user turn.
        assert_eq!(sent(&chat, "", 12), ["cccc", "dddd", "eeee"]);
        // 8 fits "dddd eeee", but "dddd" is a reply: it is left out too.
        assert_eq!(sent(&chat, "", 8), ["eeee"]);
        // The system prompt counts.
        assert_eq!(sent(&chat, "ss", 12), ["eeee"]);
    }

    #[test]
    fn the_prompt_goes_even_past_the_budget() {
        assert_eq!(sent(&["aaaa", "bbbb", "a long prompt"], "a long system prompt", 3), ["a long prompt"]);
    }

    #[test]
    fn characters_not_bytes_are_counted() {
        assert_eq!(sent(&["éé", "üü", "øø"], "", 6), ["éé", "üü", "øø"]);
    }
}
