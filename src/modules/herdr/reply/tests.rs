use super::*;

/// A Claude Code screen, shaped like a real one: an earlier turn, a tool call, the reply,
/// a spinner line and the input box with a draft in it.
const CLAUDE: &str = "\
❯ first question
⏺ First answer.

❯ Check the mail
⏺ Bash(mail --check)
  ⎿  0 new
⏺ Nothing new this heartbeat.

  - Mail: no new email since the last check,
    and the queue is empty.
  - Waiting on the user.

✻ Sautéed for 6s · done 1:15 PM
              ✔ Update installed · Restart to update
──────────────────────────────────────────────────────
❯ Yes, start the next task
──────────────────────────────────────────────────────
  ⏵⏵ bypass permissions on (shift+tab to cycle)";

#[test]
fn the_reply_is_the_last_bullet_block_of_the_last_turn() {
    assert_eq!(
        last_reply(CLAUDE, 6),
        "Nothing new this heartbeat.\n\n- Mail: no new email since the last check,\n  and the queue is empty.\n- Waiting on the user."
    );
}

#[test]
fn an_empty_prompt_line_gives_way_to_the_turn_before_it() {
    // Codex: no box rules; the input line is a prompt with a placeholder and nothing after.
    let codex = "› fix the build\n• Ran cargo build\n• Fixed it.\n\tAll green.\n\n› Ask Codex to do anything\n  gpt-5 · 80% left";
    assert_eq!(last_reply(codex, 6), "Fixed it.\nAll green.");
    // The last turn has no bullet: all of it.
    assert_eq!(last_reply("> hi\nplain answer\n  more\n", 6), "plain answer\n  more");
    assert_eq!(last_reply("> hi\nplain answer\n\n✻ Baked for 2s\n", 6), "plain answer");
    // The last turn has only a spinner so far: the turn before it.
    assert_eq!(last_reply("❯ q1\nanswer\n❯ q2\n✻ Thinking…", 6), "answer");
    // A bullet with nothing after it on its line.
    assert_eq!(last_reply("● \n  body\n", 6), "body");
}

#[test]
fn without_a_prompt_or_bullet_it_keeps_the_last_lines() {
    let text = "$ cargo test\n\n   Compiling demo v0.1.0\n\trunning 3 tests  \n\ntest a ... ok\ntest b ... ok\ntest c ... ok\n\ntest result: ok. 3 passed\n\n";
    assert_eq!(last_reply(text, 3), "test b ... ok\ntest c ... ok\ntest result: ok. 3 passed");
    assert_eq!(last_reply("", 6), "");
    assert_eq!(last_reply("a\r\nb\u{7}c\n  \n", 6), "a\nbc");
    // A prompt with no reply anywhere.
    assert_eq!(last_reply("❯ only a question", 6), "");
}

#[test]
fn a_lone_rule_or_far_apart_rules_cut_only_at_the_last() {
    let rule = "─".repeat(30);
    let text = format!("⏺ one\n{rule}\n{}\n{rule}\nstatus", "x\n".repeat(13));
    assert_eq!(last_reply(&text, 6), format!("one\n{rule}\n{}", "x\n".repeat(13).trim_end()));
    assert_eq!(last_reply(&format!("⏺ two\n{rule}\nchrome"), 6), "two");
    // An indented rule is part of the reply.
    assert_eq!(last_reply("⏺ a\n  ──────────────────────────\n  b", 6), "a\n──────────────────────────\nb");
}

#[test]
fn wrapping_keeps_indent_and_splits_long_words() {
    assert_eq!(wrap("", 10), [""]);
    assert_eq!(wrap("aaa bbb ccc", 7), ["aaa bbb", "ccc"]);
    assert_eq!(wrap("  aaa bbb ccc", 8), ["  aaa", "  bbb", "  ccc"]);
    assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
    assert_eq!(wrap("ab cdefghij", 4), ["ab", "cdef", "ghij"]);
    assert_eq!(wrap("é é", 3), ["é é"]);
}

#[test]
fn the_whole_reply_is_wrapped_and_nothing_is_cut() {
    assert_eq!(wrapped("one\ntwo\nthree", 10), "one\ntwo\nthree");
    assert_eq!(wrapped("aaa bbb ccc\nd", 3), "aaa\nbbb\nccc\nd");
    assert_eq!(wrapped("", 10), "");
}
