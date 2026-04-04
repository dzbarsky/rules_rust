// Copyright 2024 The Bazel Authors. All rights reserved.
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//    http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Bazel JSON worker wire-format helpers.
//!
//! Covers parsing, construction, and transmission of worker protocol messages.

use std::io;
use std::sync::{Arc, Mutex};

#[cfg(not(unix))]
use std::io::Write;

use tinyjson::JsonValue;

use crate::ProcessWrapperError;

use super::logging::{append_worker_lifecycle_log, current_pid, current_thread_label};
use super::request::WorkRequest;
use super::types::{RequestId, SandboxDir};

/// Thread-safe stdout guard for serializing worker responses.
pub(super) type SharedStdout = Arc<Mutex<()>>;

#[cfg(unix)]
unsafe extern "C" {
    fn write(fd: i32, buf: *const std::ffi::c_void, count: usize) -> isize;
}

pub(super) fn write_worker_response(
    stdout: &SharedStdout,
    response: &str,
) -> Result<(), ProcessWrapperError> {
    let _guard = stdout
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    write_all_stdout_fd(response.as_bytes())
        .and_then(|_| write_all_stdout_fd(b"\n"))
        .map_err(|e| ProcessWrapperError(format!("failed to write WorkResponse: {e}")))?;
    Ok(())
}

#[cfg(unix)]
fn write_all_stdout_fd(mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = unsafe { write(1, bytes.as_ptr().cast(), bytes.len()) };
        if written < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        let written = written as usize;
        if written == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "short write to worker stdout",
            ));
        }
        bytes = &bytes[written..];
    }
    Ok(())
}

#[cfg(not(unix))]
fn write_all_stdout_fd(bytes: &[u8]) -> io::Result<()> {
    let mut out = io::stdout().lock();
    out.write_all(bytes)?;
    out.flush()
}

/// Parses a single JSON work request line, sending an error response if parsing fails.
pub(super) fn parse_request_line(line: &str, stdout: &SharedStdout) -> Option<WorkRequest> {
    let request: JsonValue = match line.parse::<JsonValue>() {
        Ok(request) => request,
        Err(e) => {
            let request_id = (|| {
                let after_key = line.split_once("\"requestId\"")?.1;
                let after_colon = after_key.split_once(':')?.1.trim_start();
                let end = after_colon
                    .find(|ch: char| !ch.is_ascii_digit())
                    .unwrap_or(after_colon.len());
                after_colon[..end].parse().ok().map(super::types::RequestId)
            })();
            if let Some(request_id) = request_id {
                append_worker_lifecycle_log(&format!(
                    "pid={} thread={} request_parse_error request_id={} bytes={} error={}",
                    current_pid(),
                    current_thread_label(),
                    request_id,
                    line.len(),
                    e
                ));
                let response =
                    build_response(1, &format!("worker protocol parse error: {e}"), request_id);
                if let Err(we) = write_worker_response(stdout, &response) {
                    append_worker_lifecycle_log(&format!(
                        "pid={} event=response_write_failed thread={} request_id={} error={}",
                        current_pid(),
                        current_thread_label(),
                        request_id,
                        we,
                    ));
                }
            }
            return None;
        }
    };

    match extract_sandbox_dir(&request) {
        Ok(sandbox_dir) => Some(WorkRequest {
            request_id: extract_request_id(&request),
            arguments: extract_arguments(&request),
            sandbox_dir,
            cancel: extract_cancel(&request),
        }),
        Err(e) => {
            let request_id = extract_request_id(&request);
            let response = build_response(1, &e, request_id);
            if let Err(we) = write_worker_response(stdout, &response) {
                append_worker_lifecycle_log(&format!(
                    "pid={} event=response_write_failed thread={} request_id={} error={}",
                    current_pid(),
                    current_thread_label(),
                    request_id,
                    we,
                ));
            }
            None
        }
    }
}

/// Extracts the `requestId` field from a WorkRequest (defaults to 0).
pub(super) fn extract_request_id(request: &JsonValue) -> RequestId {
    if let JsonValue::Object(map) = request
        && let Some(JsonValue::Number(id)) = map.get("requestId")
    {
        return RequestId(*id as i64);
    }
    RequestId(0)
}

/// Extracts the `arguments` array from a WorkRequest.
pub(super) fn extract_arguments(request: &JsonValue) -> Vec<String> {
    if let JsonValue::Object(map) = request
        && let Some(JsonValue::Array(args)) = map.get("arguments")
    {
        return args
            .iter()
            .filter_map(|v| {
                if let JsonValue::String(s) = v {
                    Some(s.clone())
                } else {
                    None
                }
            })
            .collect();
    }
    vec![]
}

/// Extracts `sandboxDir` and rejects unusable sandbox directories.
///
/// An unusable directory usually means multiplex sandboxing is enabled on a
/// platform without sandbox support.
///
/// Safety: sandboxDir is constructed by Bazel as `__sandbox/<worker_id>/<execroot_basename>`.
/// It is a relative path from controlled integer and string components — no user input, no
/// path traversal possible. See `SandboxedWorkerProxy.java` in the Bazel source.
pub(super) fn extract_sandbox_dir(request: &JsonValue) -> Result<Option<SandboxDir>, String> {
    if let JsonValue::Object(map) = request
        && let Some(JsonValue::String(dir)) = map.get("sandboxDir")
    {
        if dir.is_empty() {
            return Ok(None);
        }
        if std::fs::read_dir(dir).is_ok_and(|mut entries| entries.next().is_some()) {
            return Ok(Some(SandboxDir(dir.clone())));
        }
        return Err(format!(
            "Bazel sent sandboxDir=\"{}\" but the directory {}. \
             This typically means --experimental_worker_multiplex_sandboxing is enabled \
             on a platform without sandbox support (e.g. Windows). \
             Remove this flag or make it platform-specific \
             (e.g. build:linux --experimental_worker_multiplex_sandboxing).",
            dir,
            if std::path::Path::new(dir).exists() {
                "is empty (no symlinks to execroot)"
            } else {
                "does not exist"
            },
        ));
    }
    Ok(None)
}

/// Extracts the `cancel` field from a WorkRequest (false if absent).
pub(super) fn extract_cancel(request: &JsonValue) -> bool {
    if let JsonValue::Object(map) = request
        && let Some(JsonValue::Boolean(cancel)) = map.get("cancel")
    {
        return *cancel;
    }
    false
}

/// Builds a JSON WorkResponse string.
pub(super) fn build_response(exit_code: i32, output: &str, request_id: RequestId) -> String {
    let output: String = output
        .chars()
        .map(|ch| match ch {
            '\n' | '\r' | '\t' => ch,
            ch if ch.is_control() => ' ',
            ch => ch,
        })
        .collect();
    format!(
        "{{\"exitCode\":{},\"output\":{},\"requestId\":{}}}",
        exit_code,
        json_string_literal(&output),
        request_id.0
    )
}

/// Builds a JSON WorkResponse with `wasCancelled: true`.
pub(super) fn build_cancel_response(request_id: RequestId) -> String {
    format!(
        "{{\"exitCode\":0,\"output\":{},\"requestId\":{},\"wasCancelled\":true}}",
        json_string_literal(""),
        request_id.0
    )
}

pub(super) fn json_string_literal(value: &str) -> String {
    JsonValue::String(value.to_owned())
        .stringify()
        .unwrap_or_else(|_| "\"\"".to_string())
}
