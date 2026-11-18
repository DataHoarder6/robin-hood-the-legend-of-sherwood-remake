//! Native automation routing, independent of the listener lifecycle.

use super::request_decode::{self, RequestKind};
use super::*;
use hyper::Method;

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
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
    match crate::observability::state_document() {
        Some(document) => (200, ReplyBody::Json(document)),
        // No mission has reached an RPC boundary yet; there is no last-known
        // state to report, and inventing one would be a lie with a timestamp.
        None => (
            503,
            RpcError::unavailable_capability("engine not ready — no mission state yet")
                .wire_body()
                .into(),
        ),
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

    /// The live failure: a terminal operation answered `/state` with
    /// `{"error":"mission ended before RPC request completed"}`. The listener
    /// must answer from the last published snapshot and name what stopped.
    #[test]
    fn state_answers_at_a_terminal_operation() {
        let _process = crate::observability::lock_process_surface();
        let home = tempfile::tempdir().unwrap();
        // Safety: the process-surface guard holds the install-home env lock.
        unsafe { std::env::set_var("ROBINHOOD_INSTALL_HOME", home.path()) };
        crate::observability::install_run_log();
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
        crate::observability::record_terminal(Some("Quit".into()), "mission ended");
        let (status, body) = state_response();
        assert_eq!(status, 200);
        let ReplyBody::Json(document) = body else {
            panic!("state answers with a JSON document");
        };
        assert_eq!(document["frame"], 240);
        assert_eq!(document["multiplayer"]["admission"], "running");
        assert_eq!(document["terminal"]["operation"], "Quit");
        assert_eq!(document["terminal"]["reason"], "mission ended");
        assert_eq!(
            document["observability"]["trace_log"]
                .as_str()
                .expect("trace log path")
                .rsplit_once("/logs/")
                .expect("log lives under the install home's logs")
                .0,
            home.path().to_str().unwrap()
        );
        unsafe { std::env::remove_var("ROBINHOOD_INSTALL_HOME") };
    }
}
