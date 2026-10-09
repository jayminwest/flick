//! Span attribution to the running task (`Event::TaskChanged`) and `--by task`.

use super::*;

/// Stored spans as (start, end, task).
fn tasked(cx: &Cx) -> Vec<(i64, i64, Option<i64>)> {
    cx.store.spans(0, i64::MAX).into_iter().map(|s| (s.start, s.end, s.subject.task)).collect()
}

fn task(a: &mut Activity, cx: &mut Cx, t: i64, task: Option<i64>) {
    at(t);
    a.on_event(Event::TaskChanged { task }, cx);
}

#[test]
fn a_task_change_splits_the_open_span() {
    with_cx(|cx| {
        let mut a = activity("");
        task(&mut a, cx, T, Some(1)); // recording off: only remembered
        assert!(rows(cx).is_empty());
        run(&mut a, cx, "on").unwrap();
        task(&mut a, cx, T + 60, Some(3));
        at(T + 100);
        a.on_event(Event::AppActivated { pid: 2 }, cx);
        task(&mut a, cx, T + 120, Some(3)); // the same task: the span goes on
        task(&mut a, cx, T + 150, None);
        assert_eq!(
            tasked(cx),
            [(T, T + 60, Some(1)), (T + 60, T + 100, Some(3)), (T + 100, T + 150, Some(3)), (T + 150, T + 150, None)]
        );
        // Idle: the next span after input carries the task set meanwhile.
        at(T + 200);
        a.on_event(Event::Idle { secs: 30 }, cx);
        task(&mut a, cx, T + 210, Some(4));
        at(T + 300);
        a.on_event(Event::Active, cx);
        // Locked: the task is remembered and used on unlock.
        at(T + 320);
        a.on_event(Event::Locked, cx);
        task(&mut a, cx, T + 330, Some(5));
        at(T + 400);
        a.on_event(Event::Unlocked, cx);
        // Flick in front: the span of the app behind it splits.
        at(T + 450);
        a.on_event(Event::AppActivated { pid: OWN }, cx);
        task(&mut a, cx, T + 460, Some(6));
        FRONT.with(|f| f.set(None));
        a.on_event(Event::Wake, cx); // nothing in front: no span, the task stays
        task(&mut a, cx, T + 470, None);
        assert_eq!(
            tasked(cx)[3..],
            [(T + 150, T + 170, None), (T + 300, T + 320, Some(4)), (T + 400, T + 460, Some(5)), (T + 460, T + 460, Some(6))]
        );
        assert_eq!(a.task, None);
    });
}

#[test]
fn reports_total_by_task_and_spans_carry_it() {
    with_cx(|cx| {
        let mut a = activity("");
        run(&mut a, cx, "on").unwrap();
        task(&mut a, cx, T + 300, Some(7));
        at(T + 900);
        let text = run(&mut a, cx, "today --by task").unwrap();
        assert_eq!(
            text,
            "Activity today by task (recording on)\nRecorded 15m\n       10m   67%  task 7\n        5m   33%  no task"
        );
        assert!(run(&mut a, cx, "week --by task").unwrap().starts_with("Activity week by task"));
        for bad in ["today --by app", "week task"] {
            assert!(run(&mut a, cx, bad).unwrap_err().starts_with("activity: usage: activity "), "{bad}");
        }
        cx.json = true;
        let json: serde_json::Value = serde_json::from_str(&run(&mut a, cx, "today --by task").unwrap()).unwrap();
        assert_eq!((json["by_task"][0]["task"].clone(), json["by_task"][0]["secs"].clone()), (7.into(), 600.into()));
        assert_eq!(json["by_task"][1]["task"], serde_json::Value::Null);
        let spans: serde_json::Value = serde_json::from_str(&run(&mut a, cx, "spans").unwrap()).unwrap();
        assert_eq!((spans[0]["task"].clone(), spans[1]["task"].clone()), (serde_json::Value::Null, 7.into()));
    });
}
