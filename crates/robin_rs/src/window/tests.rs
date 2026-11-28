use super::{GameRuntimeCompletion, receive_game_exit_code, touch_output_needs_deferred_up};
use crate::touch_input::TouchOutput;

/// In-memory stand-in for the presentation frontend. It accepts exactly the
/// two visibility signals the winit handler forwards (occlusion and a 0x0
/// minimized surface) and exposes the derived state the game thread reads
/// before acquiring a swapchain texture.
#[derive(Default)]
struct FakeFrontend {
    occluded: bool,
    minimized: bool,
}

impl FakeFrontend {
    fn occlude(&mut self, occluded: bool) {
        self.occluded = occluded;
    }

    fn resize(&mut self, width: u32, height: u32) {
        self.minimized = width == 0 || height == 0;
    }

    fn hidden(&self) -> bool {
        super::hidden_from_visibility_signals(self.occluded, self.minimized)
    }
}

#[test]
fn hidden_window_resumes_presentation_when_visible_again() {
    let mut frontend = FakeFrontend::default();
    assert!(!frontend.hidden(), "a mapped window presents");

    frontend.occlude(true);
    assert!(frontend.hidden(), "occlusion must stop presenting");
    frontend.occlude(false);
    assert!(!frontend.hidden(), "uncovering must resume presenting");

    frontend.resize(0, 0);
    assert!(
        frontend.hidden(),
        "a minimized 0x0 surface must not present"
    );
    frontend.resize(1920, 1080);
    assert!(
        !frontend.hidden(),
        "restoring the surface must resume presenting"
    );

    frontend.occlude(true);
    assert!(frontend.hidden());
    frontend.resize(0, 0);
    frontend.occlude(false);
    assert!(frontend.hidden(), "minimized alone still hides the window");
    frontend.resize(1280, 720);
    assert!(!frontend.hidden());
}

#[test]
fn a_present_longer_than_one_monitor_period_is_a_stall_not_a_budget() {
    // 165 Hz monitor period is ~6060 us. A fully occluded FIFO present takes
    // ~1 s; it must not be adopted as the next frame's cost estimate.
    assert!(super::present_cost_is_stall(1_000_000, Some(6_060)));
    assert!(!super::present_cost_is_stall(2_500, Some(6_060)));
    assert!(!super::present_cost_is_stall(6_060, Some(6_060)));
    // No monitor rate (wasm / headless tests) means no bound to apply.
    assert!(!super::present_cost_is_stall(1_000_000, None));
}

#[test]
fn process_visibility_tracks_the_visibility_signals() {
    super::set_window_visibility(false, false);
    assert!(!super::window_hidden());
    super::set_window_visibility(true, false);
    assert!(super::window_hidden());
    super::set_window_visibility(false, true);
    assert!(super::window_hidden());
    super::set_window_visibility(false, false);
    assert!(!super::window_hidden());
}

#[test]
fn failed_window_send_preserves_the_already_queued_message() {
    let (sender, receiver) = async_channel::bounded(1);
    super::report_window_send(sender.try_send(1));
    super::report_window_send(sender.try_send(2));
    assert_eq!(receiver.try_recv().unwrap(), 1);
    assert!(receiver.try_recv().is_err());
    receiver.close();
    super::report_window_send(sender.try_send(3));
    assert!(receiver.try_recv().is_err());
}

#[test]
fn game_exit_code_is_forwarded() {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(17).unwrap();

    assert_eq!(receive_game_exit_code(&rx), Ok(17));
}

#[test]
fn missing_game_exit_code_is_an_error() {
    let (_tx, rx) = std::sync::mpsc::channel();

    assert_eq!(
        receive_game_exit_code(&rx),
        Err("game event loop exited before the game thread published its exit code".to_owned())
    );
}

#[test]
fn disconnected_game_thread_is_an_error() {
    let (tx, rx) = std::sync::mpsc::channel::<i32>();
    drop(tx);

    assert_eq!(
        receive_game_exit_code(&rx),
        Err("game thread terminated without publishing an exit code".to_owned())
    );
}

#[test]
#[cfg(not(target_os = "android"))]
fn normal_close_delivers_quit_before_waiting_for_actual_success() {
    let (events_tx, events_rx) = async_channel::unbounded();
    let (exit_tx, exit_rx) = std::sync::mpsc::channel();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    let completion = GameRuntimeCompletion {
        sender: Some(exit_tx),
        wake: Some(move || wake_tx.send(receive_game_exit_code(&exit_rx)).unwrap()),
    };

    super::request_window_close(&events_tx).unwrap();
    assert!(matches!(
        events_rx.try_recv(),
        Ok(super::HostMsg::Event(crate::gfx_types::GameEvent::Quit))
    ));
    // Queuing close is not completion, even if the game is still loading
    // or unwinding a modal. The UI stays alive without blocking on it.
    assert_eq!(
        wake_rx.try_recv(),
        Err(std::sync::mpsc::TryRecvError::Empty)
    );
    completion.publish(0);
    assert_eq!(wake_rx.try_recv().unwrap(), Ok(0));
}

#[test]
fn startup_and_game_failures_are_published_before_waking_the_loop() {
    for code in [1, 17] {
        let (tx, rx) = std::sync::mpsc::channel();
        let completion = GameRuntimeCompletion {
            sender: Some(tx),
            wake: Some(move || assert_eq!(receive_game_exit_code(&rx), Ok(code))),
        };
        completion.publish(code);
    }
}

#[test]
fn abnormal_future_drop_disconnects_before_waking_the_loop() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    let completion = GameRuntimeCompletion {
        sender: Some(tx),
        wake: Some(move || wake_tx.send(receive_game_exit_code(&rx)).unwrap()),
    };
    drop(completion);
    assert_eq!(
        wake_rx.try_recv().unwrap(),
        Err("game thread terminated without publishing an exit code".to_owned())
    );
}

#[test]
#[ignore = "requires LLVM unwind support; see docs/TESTING.md"]
fn unwinding_panic_disconnects_before_waking_the_loop() {
    let (tx, rx) = std::sync::mpsc::channel();
    let (wake_tx, wake_rx) = std::sync::mpsc::channel();
    let result = std::panic::catch_unwind(move || {
        let _completion = GameRuntimeCompletion {
            sender: Some(tx),
            wake: Some(move || wake_tx.send(receive_game_exit_code(&rx)).unwrap()),
        };
        panic!("simulated game-thread failure");
    });
    assert!(result.is_err());
    assert_eq!(
        wake_rx.try_recv().unwrap(),
        Err("game thread terminated without publishing an exit code".to_owned())
    );
}

#[test]
fn game_thread_panic_fails_the_child_process() {
    const CHILD: &str = "ROBIN_WINDOW_PANIC_PROBE";
    if std::env::var_os(CHILD).is_some() {
        // Cranelift may abort instead of unwinding this thread. Keep that
        // real native behavior isolated from the main test process.
        let _ = std::thread::spawn(|| panic!("simulated native game panic")).join();
        std::process::exit(1);
    }
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "window::tests::game_thread_panic_fails_the_child_process",
        ])
        .env(CHILD, "1")
        .spawn()
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(!status.success(), "a game panic must not become success");
            break;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("panic-probe child failed to terminate within ten seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn unstarted_factory_and_never_polled_future_report_missing_exit_code() {
    for create_future in [false, true] {
        let (tx, rx) = std::sync::mpsc::channel();
        let (wake_tx, wake_rx) = std::sync::mpsc::channel();
        let completion = GameRuntimeCompletion {
            sender: Some(tx),
            wake: Some(move || wake_tx.send(receive_game_exit_code(&rx)).unwrap()),
        };
        let make_future = move || async move {
            std::future::pending::<()>().await;
            completion.publish(0);
        };
        if create_future {
            drop(make_future());
        } else {
            drop(make_future);
        }
        assert_eq!(
            wake_rx.try_recv().unwrap(),
            Err("game thread terminated without publishing an exit code".to_owned())
        );
    }
}

#[test]
#[cfg(not(target_os = "android"))]
fn close_request_delivery_failure_is_explicit() {
    let (tx, rx) = async_channel::unbounded();
    drop(rx);
    assert!(
        super::request_window_close(&tx)
            .unwrap_err()
            .contains("could not deliver")
    );
}

#[test]
fn close_survives_loading_and_nested_modal_event_drains() {
    use crate::gfx_types::GameEvent;
    let mut events = Vec::new();
    super::preserve_close_request(false, &mut events);
    assert!(events.is_empty());
    for _ in 0..3 {
        events.clear(); // A loading/modal loop consumed the previous batch.
        super::preserve_close_request(true, &mut events);
        super::preserve_close_request(true, &mut events);
        assert!(matches!(events.as_slice(), [GameEvent::Quit]));
    }
}

#[test]
fn only_release_classified_pointer_sequences_defer_their_up() {
    assert!(touch_output_needs_deferred_up(&[
        TouchOutput::PointerDown {
            x: 10.0,
            y: 20.0,
            button: 1,
            clicks: 1,
        },
        TouchOutput::PointerUp {
            x: 10.0,
            y: 20.0,
            button: 1,
        },
    ]));
    assert!(!touch_output_needs_deferred_up(&[
        TouchOutput::PointerMove { x: 30.0, y: 40.0 },
        TouchOutput::PointerUp {
            x: 30.0,
            y: 40.0,
            button: 1,
        },
    ]));
}

fn touch_handler() -> (super::AppHandler, async_channel::Receiver<super::HostMsg>) {
    let (events_tx, events_rx) = async_channel::unbounded();
    let (_cmd_tx, cmd_rx) = async_channel::unbounded();
    let handler = super::AppHandler::new(
        "touch test",
        1280,
        720,
        true,
        events_tx,
        cmd_rx,
        Box::new(|_| {}),
    );
    (handler, events_rx)
}

#[test]
fn touch_pointer_outputs_preserve_the_recognized_button() {
    use crate::gfx_types::GameEvent;

    for button in [1, 3] {
        let (mut handler, events) = touch_handler();
        handler.emit_touch_outputs(vec![
            TouchOutput::PointerDown {
                x: 10.0,
                y: 20.0,
                button,
                clicks: 1,
            },
            TouchOutput::PointerUp {
                x: 10.0,
                y: 20.0,
                button,
            },
        ]);
        assert!(matches!(
            events.try_recv(),
            Ok(super::HostMsg::Event(GameEvent::MouseDown(10, 20, actual, 1))) if actual == button
        ));
        assert!(matches!(
            events.try_recv(),
            Ok(super::HostMsg::DeferredEvent(GameEvent::MouseUp(10, 20, actual))) if actual == button
        ));
    }
}

#[test]
fn touch_tap_defers_release_feedback_with_its_pointer_up() {
    use crate::gfx_types::GameEvent;

    let (mut handler, events) = touch_handler();
    handler.emit_touch_outputs(vec![
        TouchOutput::PointerDown {
            x: 10.0,
            y: 20.0,
            button: 3,
            clicks: 1,
        },
        TouchOutput::PointerUp {
            x: 10.0,
            y: 20.0,
            button: 3,
        },
        TouchOutput::PressFeedback {
            contact: 7,
            x: 10.0,
            y: 20.0,
            pressed: false,
        },
    ]);
    assert!(matches!(
        events.try_recv(),
        Ok(super::HostMsg::Event(GameEvent::MouseDown(10, 20, 3, 1)))
    ));
    assert!(matches!(
        events.try_recv(),
        Ok(super::HostMsg::DeferredEvent(GameEvent::MouseUp(10, 20, 3)))
    ));
    assert!(matches!(
        events.try_recv(),
        Ok(super::HostMsg::DeferredEvent(
            GameEvent::TouchPressFeedback {
                contact: 7,
                pressed: false,
                ..
            }
        ))
    ));
}

#[test]
fn touch_press_feedback_maps_to_a_host_only_game_event() {
    use crate::gfx_types::GameEvent;

    let (mut handler, events) = touch_handler();
    handler.emit_touch_outputs(vec![TouchOutput::PressFeedback {
        contact: 7,
        x: 10.5,
        y: 20.5,
        pressed: true,
    }]);
    assert!(matches!(
        events.try_recv(),
        Ok(super::HostMsg::Event(GameEvent::TouchPressFeedback {
            contact: 7,
            x: 10.5,
            y: 20.5,
            pressed: true,
        }))
    ));
}
