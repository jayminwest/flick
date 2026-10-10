use super::*;
use crate::config::parse;
use crate::modules::sys::ID;
use crate::modules::sys::act::Place;
use crate::modules::sys::settings::Settings;

const CONFIG: &str = r#"
[[sys.machine]]
name = "pro"
via = "ssh"
ssh = "me@pro"
vnc = "vnc://pro"
dash = "https://pro:8080/"
[[sys.machine.service]]
name = "ollama"
kind = "launchd"
target = "com.ollama"
log = "/opt/ollama.log"
restart = true
[[sys.machine.service]]
name = "port"
kind = "tcp"
target = "pro:1"
[[sys.machine]]
name = "bare"
via = "flick"
"#;

fn machines() -> Vec<Machine> {
    let s: Settings = parse(CONFIG).unwrap().section(ID).unwrap().unwrap().get::<Settings>().unwrap().check().unwrap();
    s.machine
}

fn target(m: &Machine, i: usize) -> Target {
    Target { machine: m.name.clone(), service: m.service[i].clone(), place: Place::Ssh("me@pro".into()) }
}

fn labels(card: &Card) -> Vec<(&str, &str, Style)> {
    card.actions.iter().map(|a| (a.id.as_str(), a.label.as_str(), a.style)).collect()
}

#[test]
fn a_card_holds_the_machine_and_service_actions() {
    let ms = machines();
    let pro = &ms[0];
    let cmd = "ssh me@pro launchctl kickstart -k gui/$(id -u)/com.ollama".to_string();
    let card = card(pro, &[(target(pro, 0), Ok(cmd.clone())), (target(pro, 1), Err("unused".into()))]).unwrap();
    assert_eq!((card.id.as_str(), card.title.as_str(), card.state), ("actions/pro", "pro · actions", State::Open));
    assert_eq!(
        labels(&card),
        [
            ("vnc", "Screen Sharing", Style::Default),
            ("dash", "Open Dash", Style::Default),
            ("tail/ollama", "Tail ollama", Style::Default),
            ("restart/ollama", "Restart ollama…", Style::Destructive),
        ]
    );
    // Restart asks first: a shell action showing the exact command, which never runs as such.
    assert_eq!(card.actions[3].kind, Kind::Local { run: Do::Shell(cmd), reply: false });
    assert_eq!(card.actions[0].kind, Kind::Reply);
    // A restart that cannot run is disabled with why.
    let card = super::card(pro, &[(target(pro, 0), Err("sys: no uid for the launchd domain".into()))]).unwrap();
    let why = Kind::Disabled { raw: Value::Null, reply: false, reason: "sys: no uid for the launchd domain".into() };
    assert_eq!(card.actions[3].kind, why);
    // Nothing to offer: no card.
    assert_eq!(super::card(&ms[1], &[]), None);
}

#[test]
fn presses_name_what_to_do() {
    let cancel = "__cancel__";
    assert_eq!(press("actions/pro", "vnc", cancel), Some(Press::Open { machine: "pro", vnc: true }));
    assert_eq!(press("actions/pro", "dash", cancel), Some(Press::Open { machine: "pro", vnc: false }));
    assert_eq!(press("actions/a/b", "tail/x/y", cancel), Some(Press::Tail { machine: "a/b", service: "x/y" }));
    assert_eq!(press("actions/pro", "restart/ollama", cancel), Some(Press::Restart { machine: "pro", service: "ollama" }));
    assert_eq!(press("actions/pro", cancel, cancel), Some(Press::Cancel));
    for (card, action) in [("deploy-42", "vnc"), ("actions/pro", "reboot"), ("actions/pro", "reboot/now")] {
        assert_eq!(press(card, action, cancel), None, "{card} {action}");
    }
    assert_eq!(card_id("pro"), "actions/pro");
}
