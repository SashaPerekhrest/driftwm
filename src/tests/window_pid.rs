//! Every live window in the `state` reply carries the PID of the process that
//! opened its Wayland connection, whichever list reports it: canvas, pinned or
//! fullscreen. The fixture's clients connect over a socketpair made in this
//! process, so that PID is the test's own.

use crate::ipc::dispatch;
use crate::ipc::protocol::{Request, Response, StateInfo, WindowSelector};

use super::{Fixture, map_window, window_by_app_id};

fn state(f: &mut Fixture) -> StateInfo {
    match dispatch(Request::State, f.state()) {
        Ok(Response::State(state)) => *state,
        other => panic!("expected a State reply, got {other:?}"),
    }
}

#[test]
fn canvas_pinned_and_fullscreen_entries_carry_the_client_pid() {
    let mut f = Fixture::new();
    let output = f.add_output(1, (1920, 1080));
    let id = f.add_client();
    map_window(&mut f, id, "canvas", (400, 300));
    map_window(&mut f, id, "pinned", (400, 300));
    map_window(&mut f, id, "fullscreen", (400, 300));

    dispatch(
        Request::Pin {
            window: Some(WindowSelector::AppId("pinned".into())),
            value: Some(true),
        },
        f.state(),
    )
    .unwrap();
    let fullscreen = window_by_app_id(&mut f, "fullscreen").unwrap();
    f.state()
        .enter_fullscreen(&fullscreen, Some(output.clone()));

    let state = state(&mut f);
    let pid = Some(std::process::id());
    let canvas = state.windows.iter().find(|w| w.app_id == "canvas");
    assert_eq!(canvas.map(|w| w.pid), Some(pid), "canvas entry");
    let pinned = state.pinned.iter().find(|w| w.app_id == "pinned");
    assert_eq!(pinned.map(|w| w.pid), Some(pid), "pinned entry");
    let fullscreen = state.fullscreen.iter().find(|w| w.app_id == "fullscreen");
    assert_eq!(fullscreen.map(|w| w.pid), Some(pid), "fullscreen entry");

    f.state().exit_fullscreen_on(&output);
}
