//! Native automation routing, independent of the listener lifecycle.

use super::request_decode::{self, RequestKind};
use super::*;
use hyper::Method;

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
enum Route {
    Info,
    Natives,
    EngineDump,
    Decompile,
    Screenshot,
    State,
    Events,
    Rpc(RequestKind),
}

/// Route recognition is also the transport's body-acquisition policy. Unknown
/// paths and unsupported methods never acquire a potentially large body.
fn classify(method: &Method, path: &str) -> Option<Route> {
    Some(match (method, path) {
        (&Method::GET, "/") | (&Method::GET, "/info") => Route::Info,
        (&Method::GET, "/natives") => Route::Natives,
        (&Method::GET, "/engine-dump") => Route::EngineDump,
        (&Method::GET, "/script/decompile") => Route::Decompile,
        (&Method::GET, "/screenshot") => Route::Screenshot,
        // `/state` and `/events` answer from the process's own observability
        // surface, never from the mission queue: the mission is exactly what is
        // gone (or deadlocked) when an agent asks.
        (&Method::GET, "/state") => Route::State,
        (&Method::GET, "/events") => Route::Events,
        (&Method::GET, "/host-debug" | "/level-assets" | "/script" | "/get-replay")
        | (
            &Method::POST,
            "/native" | "/batch" | "/console" | "/command" | "/step-forward" | "/step-back"
            | "/go-to-frame" | "/set-paused" | "/load-replay",
        ) => Route::Rpc(RequestKind::from_method(&path[1..]).expect("classified RPC method")),
        _ => return None,
    })
}

fn replay_body_limit() -> usize {
    crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS.max_input_bytes
}

pub(super) fn body_limit(method: &Method, path: &str) -> Option<usize> {
    let route = classify(method, path)?;
    Some(if *method == Method::GET {
        0
    } else {
        match route {
            Route::Rpc(RequestKind::LoadReplay) => replay_body_limit(),
            Route::Rpc(RequestKind::Batch) => 1024 * 1024,
            _ => 64 * 1024,
        }
    })
}

pub(super) async fn dispatch(
    req: NativeRequest,
    queue: &Queue,
    listen_port: u16,
) -> (u16, ReplyBody) {
    if let Some(reason) = browser_rejection_reason(
        req.header("Origin"),
        req.header("Sec-Fetch-Site"),
        req.header("Host"),
        listen_port,
    ) {
        tracing::warn!("script HTTP server: rejected request: {reason}");
        return (403, RpcError::invalid_request(reason).wire_body().into());
    }
    let (path, query) = req.url().split_once('?').unwrap_or((req.url(), ""));
    let payload = match classify(req.method(), path) {
        Some(Route::Info) => return (200, info_json().into()),
        Some(Route::Natives) => return (200, list_natives_json().into()),
        Some(Route::State) => return state_response(),
        Some(Route::Events) => {
            return match crate::http_server::query::events_page(query) {
                Ok(page) => (200, ReplyBody::Json(page)),
                Err(error) => (400, error.wire_body().into()),
            };
        }
        Some(Route::EngineDump) => Ok(HttpPayload::EngineDump),
        Some(Route::Decompile) => crate::http_server::query::decompile_class(query)
            .map(|class| HttpPayload::Decompile { class }),
        Some(Route::Screenshot) => {
            crate::http_server::query::screenshot(query).map(HttpPayload::Screenshot)
        }
        Some(Route::Rpc(kind)) => {
            if matches!(kind, RequestKind::LoadReplay)
                && let Err(error) = validate_replay_headers(&req)
            {
                return (400, error.wire_body().into());
            }
            if matches!(kind, RequestKind::LoadReplay) {
                crate::replay_format::preflight_compact_transport(
                    req.body_bytes(),
                    &crate::replay_format::LOCAL_CUSTOM_REPLAY_ADMISSION_LIMITS,
                )
                .map_err(|error| RpcError::replay_format("invalid binary replay", error))
                .and_then(|_| {
                    let paused = crate::http_server::query::replay_paused(query)?;
                    Ok(HttpPayload::LoadReplay {
                        data: req.body_bytes().to_vec(),
                        paused,
                    })
                })
            } else {
                request_decode::decode_json(kind, req.body_bytes())
            }
        }
        None => {
            return (
                404,
                RpcError::unavailable_capability("not found")
                    .wire_body()
                    .into(),
            );
        }
    };
    match payload {
        Ok(payload) => relay(queue, payload).await,
        Err(error) => (400, error.wire_body().into()),
    }
}

/// `/state` answers from this process's published snapshot. It is served on the
/// listener thread so a mission that ended — or a game thread that is stuck —
/// does not take the answer down with it.
fn state_response() -> (u16, ReplyBody) {
    match crate::http_server::diagnostics::state_reply() {
        Ok(body) => (200, body),
        Err(error) => (503, error.wire_body().into()),
    }
}

/// Reject unsupported replay framing before acquiring the request body.
/// Returns the maximum accepted body size for the transport collector.
pub(super) fn validate_replay_headers(req: &NativeRequest) -> Result<usize, RpcError> {
    let limit = replay_body_limit();
    if req.header("Transfer-Encoding").is_some() {
        return Err(RpcError::invalid_request(
            "load-replay does not accept Transfer-Encoding",
        ));
    }
    if req.header("Content-Encoding").is_some() {
        return Err(RpcError::invalid_request(
            "load-replay does not accept Content-Encoding",
        ));
    }
    let content_type = req.header("Content-Type").ok_or_else(|| {
        RpcError::invalid_request("load-replay requires Content-Type: application/x-robin-rhrec")
    })?;
    let media_type = content_type.split(';').next().unwrap_or_default().trim();
    if !media_type.eq_ignore_ascii_case("application/x-robin-rhrec") {
        return Err(RpcError::invalid_request(
            "load-replay requires Content-Type: application/x-robin-rhrec",
        ));
    }
    let declared = req.body_length().ok_or_else(|| {
        RpcError::invalid_request("load-replay requires a bounded Content-Length")
    })?;
    if declared > limit {
        return Err(RpcError::capacity(format!(
            "load-replay binary body observed {declared} bytes, limit is {limit}"
        )));
    }
    Ok(limit)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two queue-free endpoints are a routing fact, not a property of the
    /// handler they call: re-classifying either as `Rpc` would put it back on
    /// the mission queue while every response builder stayed green.
    #[test]
    fn state_and_events_are_routed_off_the_mission_queue() {
        assert_eq!(classify(&Method::GET, "/state"), Some(Route::State));
        assert_eq!(classify(&Method::GET, "/events"), Some(Route::Events));
        assert_eq!(body_limit(&Method::GET, "/state"), Some(0));
        assert_eq!(body_limit(&Method::GET, "/events"), Some(0));
    }

    /// `/events` is served from the process ring even while no mission session
    /// exists to drain a queue: this is the answer a poller needs after the
    /// mission died.
    #[test]
    fn an_events_page_is_served_with_an_empty_queue() {
        let since =
            crate::observability::events_page(None, crate::observability::MAX_PAGE).next_seq;
        crate::observability::event(
            crate::observability::EventKind::TransportReconnected,
            "multiplayer: transport reconnected",
        )
        .record();
        let queue: Queue = Arc::new(Mutex::new(RequestRouter::default()));
        let port = 7717;
        let request = NativeRequest::test_request(
            Method::GET,
            format!("/events?since={since}&limit=1000"),
            port,
        );
        let (status, body) = futures::executor::block_on(dispatch(request, &queue, port));
        assert_eq!(status, 200);
        let ReplyBody::Json(page) = body else {
            panic!("/events answers with a JSON page");
        };
        let events = page["events"].as_array().unwrap();
        let recorded = events
            .iter()
            .find(|event| event["kind"] == "transport_reconnected")
            .expect("the page carries the event recorded after the cursor");
        assert!(
            recorded["seq"].as_u64().unwrap() >= since,
            "the page starts at the cursor it was given: {page}"
        );
        assert!(
            page["next_seq"].as_u64().is_some_and(|next| next > since),
            "a page that returned events advances the cursor: {page}"
        );
        assert!(page["run"].is_string(), "{page}");
        assert!(
            queue.lock().unwrap().take_idle().is_empty(),
            "the page never entered the mission queue"
        );
    }

    #[test]
    fn route_body_policy_preserves_native_aliases_and_rejects_unknown_routes() {
        for path in [
            "/",
            "/info",
            "/natives",
            "/state",
            "/events",
            "/host-debug",
            "/engine-dump",
            "/level-assets",
            "/script",
            "/script/decompile",
            "/screenshot",
            "/get-replay",
        ] {
            assert_eq!(body_limit(&Method::GET, path), Some(0), "{path}");
            assert_eq!(body_limit(&Method::POST, path), None, "{path}");
        }
        for path in [
            "/native",
            "/console",
            "/command",
            "/step-forward",
            "/step-back",
            "/go-to-frame",
            "/set-paused",
        ] {
            assert_eq!(body_limit(&Method::POST, path), Some(64 * 1024), "{path}");
            assert_eq!(body_limit(&Method::GET, path), None, "{path}");
        }
        assert_eq!(body_limit(&Method::POST, "/batch"), Some(1024 * 1024));
        assert_eq!(
            body_limit(&Method::POST, "/load-replay"),
            Some(replay_body_limit())
        );
        assert_eq!(body_limit(&Method::GET, "/decompile"), None);
        assert_eq!(body_limit(&Method::POST, "/unknown"), None);
        assert_eq!(body_limit(&Method::PUT, "/native"), None);
    }

    /// The live failure: `/state` answered a terminal operation with a queue
    /// error. The listener builds its answer from the process surface, so it
    /// answers whatever the queue is doing — and names the evidence a failure
    /// bundle needs.
    #[test]
    fn state_answers_from_the_process_surface() {
        let home = tempfile::tempdir().unwrap();
        crate::observability::install_run_log_at(home.path());
        crate::observability::publish_state(serde_json::json!({
            "frame": 240,
            "map": "Dem_Lei_MP",
            "replay": null,
            "multiplayer": {
                "local_seat": 0,
                "expected_players": 2,
                "admission": "running",
                "begin_sim_frame": 133,
                "pending_transition": null,
                "last_compared_hash_frame": 225,
                "desync_count": 0,
                "final_operation": "LevelInProgress",
                "connect_string": null,
            },
        }));
        let (status, body) = state_response();
        assert_eq!(status, 200);
        let ReplyBody::Json(document) = body else {
            panic!("state answers with a JSON document");
        };
        let object = document.as_object().expect("state answers with an object");
        for key in ["frame", "map", "multiplayer", "terminal"] {
            assert!(
                object.contains_key(key),
                "the document names {key} in every attempt: {document}"
            );
        }
        let observability = &document["observability"];
        assert!(observability["trace_log"].is_string(), "{document}");
        assert!(observability["event_log"].is_string(), "{document}");
        assert!(observability["event_run"].is_string(), "{document}");
    }
}
