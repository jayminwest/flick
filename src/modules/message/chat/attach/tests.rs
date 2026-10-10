use super::*;

#[test]
fn remote_words_are_plain_paths() {
    for ok in [".dotfiles/home/.local/bin/kota-ask", "~/bin/x", "a_b.c-d"] {
        assert!(remote_word(ok), "{ok}");
    }
    for bad in ["", "-x", "a b", "a;b", "$(x)", "a'b", "a\"b", "a\nb", &"a".repeat(PATH_MAX + 1)] {
        assert!(!remote_word(bad), "{bad:?}");
    }
    assert!(remote_word(&"a".repeat(PATH_MAX)));
}

#[test]
fn attach_dirs_stay_under_the_remote_home() {
    assert_eq!(dir(DEFAULT_DIR), Ok(".cache/flick/attach".into()));
    assert_eq!(dir("shots/"), Ok("shots".into()));
    assert_eq!(dir("a/b.c/d_e-f"), Ok("a/b.c/d_e-f".into()));
    for bad in ["", "/", "/tmp/x", "~/x", "a/../b", "..", "./a", "a//b", "a/-x", "-a", "a b", "a;rm"] {
        let e = dir(bad).unwrap_err();
        assert!(e.starts_with("[message] attach_dir"), "{bad:?}: {e}");
    }
}

#[test]
fn screenshots_are_named_by_request_and_number() {
    assert_eq!(path(DEFAULT_DIR, "k1abc", 2), Ok(".cache/flick/attach/k1abc-2.png".into()));
    assert!(path("/abs", "k", 1).is_err());
    assert!(path(DEFAULT_DIR, "a/b", 1).unwrap_err().contains("a request id"));
    assert!(path(DEFAULT_DIR, "", 1).is_err());
}

#[test]
fn the_upload_writes_and_prunes_in_one_call() {
    let argv = upload_argv("jaymin@mbp-server", "shots/", "k1", 1).unwrap();
    let want = [
        "/usr/bin/ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=8", "jaymin@mbp-server",
        "mkdir -p shots && cat > shots/k1-1.png && find shots -type f -name '*.png' -mtime +7 -delete",
    ];
    assert_eq!(argv, want);
    assert!(upload_argv("-bad", "shots", "k1", 1).is_err());
    assert!(upload_argv("h", "../x", "k1", 1).is_err());
    assert!(upload_argv("h", "x", "k 1", 1).is_err());
}

#[test]
fn sizes_are_checked() {
    assert!(BUDGET > super::ask::BUDGET, "a PNG takes longer than a question");
    assert!(check_size(1).is_ok());
    assert!(check_size(PNG_MAX).is_ok());
    assert_eq!(check_size(0), Err("The screenshot is empty".into()));
    assert_eq!(check_size(PNG_MAX + 1), Err("The screenshot is over 32 MB".into()));
}
