use crate::cli::{stream_json_resume_command, CommandSpec};
use crate::logging::log_to_file;
use crate::models::{mark_output_activity, OutputActivity, SharedAppState};
use crate::resume::StreamResumeCommand;
use crate::watcher::scan::{active_rate_limit_from_message, ActiveRateLimitInfo};
use anyhow::Result;
use chrono::{DateTime, Local};
use serde_json::Value;
use std::io;
use std::sync::{Arc, Mutex};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::mpsc;

#[derive(Debug)]
pub struct StreamJsonSignal {
    pub session_id: Option<String>,
    pub rate_limit: Option<ActiveRateLimitInfo>,
}

#[derive(Debug)]
pub enum StreamLineResult {
    Signal(StreamJsonSignal),
    Ignored,
    InvalidJson,
}

#[derive(Debug, PartialEq, Eq)]
enum StreamProcessAction {
    Exit,
    Restart,
}

pub(crate) async fn pump_raw_output<R, W>(
    mut reader: R,
    mut writer: W,
    line_tx: mpsc::Sender<String>,
    activity: OutputActivity,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    let mut buf = [0_u8; 8192];
    let mut pending = Vec::new();

    loop {
        let n = reader.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }

        writer.write_all(&buf[..n]).await?;
        writer.flush().await?;
        mark_output_activity(&activity);

        pending.extend_from_slice(&buf[..n]);
        while let Some(newline) = pending.iter().position(|byte| *byte == b'\n') {
            let line_bytes: Vec<u8> = pending.drain(..=newline).collect();
            if let Ok(line) = String::from_utf8(line_bytes) {
                let line = line.trim_end_matches(['\r', '\n']).to_string();
                let _ = line_tx.try_send(line);
            }
        }
    }
}

pub async fn run_stream_json_print(
    command: CommandSpec,
    state: SharedAppState,
    mut resume_rx: mpsc::UnboundedReceiver<StreamResumeCommand>,
) -> Result<()> {
    let latest_session_id = Arc::new(Mutex::new(None::<String>));
    let initial_program = command.program.clone();
    let mut next_command = command;

    loop {
        let action = run_one_stream_process(
            next_command.clone(),
            Arc::clone(&state),
            Arc::clone(&latest_session_id),
            &mut resume_rx,
        )
        .await?;

        if action == StreamProcessAction::Exit {
            return Ok(());
        }

        let Some(session_id) = latest_session_id
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            log_to_file(
                "[Stream JSON] Cannot resume: no session id was observed in stream output.",
            );
            return Ok(());
        };

        next_command = resume_command_with_program(&initial_program, &session_id);
    }
}

async fn run_one_stream_process(
    command: CommandSpec,
    state: SharedAppState,
    latest_session_id: Arc<Mutex<Option<String>>>,
    resume_rx: &mut mpsc::UnboundedReceiver<StreamResumeCommand>,
) -> Result<StreamProcessAction> {
    let starting_lockout_revision = state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .lockout_revision;
    let mut child = Command::new(&command.program)
        .args(&command.args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    let stdout = child.stdout.take().expect("stdout was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let activity = {
        state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .last_output_activity
            .clone()
    };

    let (line_tx, line_rx) = mpsc::channel(1024);
    let stdout_task = tokio::spawn(pump_raw_output(
        stdout,
        tokio::io::stdout(),
        line_tx,
        activity.clone(),
    ));
    let stderr_task = tokio::spawn(pump_raw_output(
        stderr,
        tokio::io::stderr(),
        mpsc::channel(1).0,
        activity,
    ));
    let parser_task = tokio::spawn(parse_stream_lines(
        line_rx,
        Arc::clone(&state),
        latest_session_id,
    ));

    let (live_restart_requested, child_status) = tokio::select! {
        status = child.wait() => {
            let status = status?;
            log_to_file(&format!(
                "[Stream JSON] Claude exited with status {status}."
            ));
            (false, Some(status))
        }
        command = resume_rx.recv() => {
            match command {
                Some(StreamResumeCommand::Continue) => {
                    log_to_file("[Stream JSON] Continue requested. Killing print-mode child for restart with --resume.");
                    restart_running_child(&mut child).await?;
                    (true, None)
                }
                None => {
                    let status = child.wait().await?;
                    log_to_file(&format!(
                        "[Stream JSON] Claude exited with status {status}."
                    ));
                    (false, Some(status))
                }
            }
        }
    };

    stdout_task.await??;
    stderr_task.await??;
    parser_task.await?;

    if let Some(status) = child_status {
        if !status.success() {
            anyhow::bail!("stream child exited with status {status}");
        }
    }

    if live_restart_requested {
        return Ok(StreamProcessAction::Restart);
    }

    Ok(await_resume_after_exit(
        lockout_recorded_since(&state, starting_lockout_revision),
        resume_rx,
    )
    .await)
}

fn lockout_recorded_since(state: &SharedAppState, starting_revision: u64) -> bool {
    state
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .lockout_revision
        != starting_revision
}

async fn await_resume_after_exit(
    resume_pending: bool,
    resume_rx: &mut mpsc::UnboundedReceiver<StreamResumeCommand>,
) -> StreamProcessAction {
    if let Ok(StreamResumeCommand::Continue) = resume_rx.try_recv() {
        return StreamProcessAction::Restart;
    }

    if !resume_pending {
        return StreamProcessAction::Exit;
    }

    while let Some(command) = resume_rx.recv().await {
        if matches!(command, StreamResumeCommand::Continue) {
            return StreamProcessAction::Restart;
        }
    }

    StreamProcessAction::Exit
}

async fn restart_running_child(child: &mut tokio::process::Child) -> Result<()> {
    match child.start_kill() {
        Ok(()) => {
            let _ = child.wait().await?;
            Ok(())
        }
        Err(error) => {
            if child.try_wait()?.is_some() {
                Ok(())
            } else {
                Err(error.into())
            }
        }
    }
}

fn resume_command_with_program(program: &str, session_id: &str) -> CommandSpec {
    let mut command = stream_json_resume_command(session_id);
    command.program = program.to_string();
    command
}

async fn parse_stream_lines(
    mut line_rx: mpsc::Receiver<String>,
    state: SharedAppState,
    latest_session_id: Arc<Mutex<Option<String>>>,
) {
    while let Some(line) = line_rx.recv().await {
        match parse_stream_line(&line) {
            StreamLineResult::Signal(signal) => {
                if let Some(session_id) = signal.session_id {
                    *latest_session_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(session_id);
                }
                if let Some(limit) = signal.rate_limit {
                    crate::monitor::record_lockout(&state, limit, "stream-json");
                }
            }
            StreamLineResult::InvalidJson => {
                log_to_file("[Stream JSON] Invalid NDJSON line; raw output was preserved.");
            }
            StreamLineResult::Ignored => {}
        }
    }
}

pub fn parse_stream_line(line: &str) -> StreamLineResult {
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        return StreamLineResult::InvalidJson;
    };

    let session_id = find_string_field(&value, &["session_id", "sessionId"]);
    let rate_limit = extract_rate_limit(&value);

    if session_id.is_some() || rate_limit.is_some() {
        StreamLineResult::Signal(StreamJsonSignal {
            session_id,
            rate_limit,
        })
    } else {
        StreamLineResult::Ignored
    }
}

fn extract_rate_limit(value: &Value) -> Option<ActiveRateLimitInfo> {
    if value.get("error").and_then(Value::as_str) != Some("rate_limit") {
        return None;
    }

    let timestamp = find_string_field(value, &["timestamp"])?;
    let log_time = DateTime::parse_from_rfc3339(&timestamp)
        .ok()?
        .with_timezone(&Local);
    let message = find_rate_limit_text(value)?;

    active_rate_limit_from_message(log_time, &message)
}

fn find_rate_limit_text(value: &Value) -> Option<String> {
    find_string_value(value, &|text| {
        let lower = text.to_ascii_lowercase();
        (lower.contains("rate_limit") || lower.contains("limit")) && lower.contains("reset")
    })
}

fn find_string_field(value: &Value, names: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for name in names {
                if let Some(Value::String(text)) = map.get(*name) {
                    return Some(text.clone());
                }
            }
            map.values()
                .find_map(|child| find_string_field(child, names))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_field(child, names)),
        _ => None,
    }
}

fn find_string_value(value: &Value, matches: &dyn Fn(&str) -> bool) -> Option<String> {
    match value {
        Value::String(text) if matches(text) => Some(text.clone()),
        Value::Array(items) => items
            .iter()
            .find_map(|child| find_string_value(child, matches)),
        Value::Object(map) => map
            .values()
            .find_map(|child| find_string_value(child, matches)),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        await_resume_after_exit, lockout_recorded_since, parse_stream_line, pump_raw_output,
        resume_command_with_program, StreamLineResult, StreamProcessAction,
    };
    use crate::cli::CommandSpec;
    use crate::models::{output_is_hot, AppState};
    use crate::watcher::scan::ActiveRateLimitInfo;
    use chrono::{Duration as ChronoDuration, Local};
    use std::collections::VecDeque;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll};
    use std::time::Duration;
    use tokio::io::{AsyncRead, ReadBuf};
    use tokio::sync::mpsc;

    struct FragmentedReader {
        chunks: VecDeque<Vec<u8>>,
    }

    impl FragmentedReader {
        fn new(chunks: &[&[u8]]) -> Self {
            Self {
                chunks: chunks.iter().map(|chunk| chunk.to_vec()).collect(),
            }
        }
    }

    impl AsyncRead for FragmentedReader {
        fn poll_read(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &mut ReadBuf<'_>,
        ) -> Poll<std::io::Result<()>> {
            if let Some(chunk) = self.chunks.pop_front() {
                buf.put_slice(&chunk);
            }
            Poll::Ready(Ok(()))
        }
    }

    #[test]
    fn invalid_json_is_reported_without_failing_raw_output() {
        assert!(matches!(
            parse_stream_line("not json"),
            StreamLineResult::InvalidJson
        ));
    }

    #[test]
    fn ignores_valid_signal_free_event() {
        assert!(matches!(
            parse_stream_line(r#"{"type":"result","result":"ok"}"#),
            StreamLineResult::Ignored
        ));
    }

    #[test]
    fn extracts_session_id_from_stream_event() {
        let result = parse_stream_line(
            r#"{"type":"system","session_id":"11111111-1111-1111-1111-111111111111"}"#,
        );

        let StreamLineResult::Signal(signal) = result else {
            panic!("expected signal");
        };
        assert_eq!(
            signal.session_id.as_deref(),
            Some("11111111-1111-1111-1111-111111111111")
        );
        assert!(signal.rate_limit.is_none());
    }

    #[test]
    fn extracts_rate_limit_from_nested_stream_event() {
        let result = parse_stream_line(
            r#"{"type":"error","timestamp":"2099-07-09T10:00:00-04:00","error":"rate_limit","message":{"content":[{"type":"text","text":"Claude limit reached; resets 5:30pm"}]}}"#,
        );

        let StreamLineResult::Signal(signal) = result else {
            panic!("expected signal");
        };
        let limit = signal.rate_limit.expect("rate limit");
        assert_eq!(limit.display_str, "5:30pm");
        assert_eq!(limit.raw_message, "Claude limit reached; resets 5:30pm");
    }

    #[test]
    fn ignores_assistant_text_that_only_mentions_a_limit_reset() {
        assert!(matches!(
            parse_stream_line(
                r#"{"type":"assistant","timestamp":"2099-07-09T10:00:00-04:00","message":{"content":[{"type":"text","text":"Claude limit reached; resets 5:30pm"}]}}"#,
            ),
            StreamLineResult::Ignored
        ));
    }

    #[test]
    fn ignores_unknown_json_shape() {
        assert!(matches!(
            parse_stream_line(r#"{"type":"assistant","message":{"content":[]}}"#),
            StreamLineResult::Ignored
        ));
    }

    #[test]
    fn waits_only_for_lockouts_recorded_during_current_process() {
        let state = Arc::new(Mutex::new(AppState::new()));
        state.lock().unwrap().lockout_target_time = Some(Local::now() + ChronoDuration::hours(1));
        let starting_revision = state.lock().unwrap().lockout_revision;

        assert!(!lockout_recorded_since(&state, starting_revision));

        crate::monitor::record_lockout(
            &state,
            ActiveRateLimitInfo {
                target_time: Local::now() + ChronoDuration::hours(2),
                display_str: "later".to_string(),
                raw_message: "rate limit".to_string(),
            },
            "stream-json",
        );

        assert!(lockout_recorded_since(&state, starting_revision));
    }

    #[tokio::test]
    async fn pump_preserves_raw_bytes_and_emits_complete_lines() {
        let state = AppState::new();
        let (line_tx, mut line_rx) = mpsc::channel(8);
        let input = b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n".as_slice();
        let mut output = Vec::new();

        pump_raw_output(
            input,
            &mut output,
            line_tx,
            state.last_output_activity.clone(),
        )
        .await
        .expect("pump succeeds");

        assert_eq!(output, b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n");
        assert_eq!(line_rx.recv().await.as_deref(), Some("{\"type\":\"one\"}"));
        assert_eq!(line_rx.recv().await.as_deref(), Some("{\"type\":\"two\"}"));
        assert!(output_is_hot(
            &state.last_output_activity,
            Duration::from_secs(2)
        ));
    }

    #[tokio::test]
    async fn pump_keeps_incomplete_tail_raw_without_parsing_it() {
        let state = AppState::new();
        let (line_tx, mut line_rx) = mpsc::channel(8);
        let input = b"{\"partial\":true}".as_slice();
        let mut output = Vec::new();

        pump_raw_output(
            input,
            &mut output,
            line_tx,
            state.last_output_activity.clone(),
        )
        .await
        .expect("pump succeeds");

        assert_eq!(output, b"{\"partial\":true}");
        assert!(line_rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn pump_never_blocks_when_line_channel_is_full() {
        let state = AppState::new();
        let (line_tx, _line_rx) = mpsc::channel(1);
        line_tx.try_send("{\"filled\":true}".to_string()).unwrap();
        let input = b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n".as_slice();
        let mut output = Vec::new();

        let result = tokio::time::timeout(
            Duration::from_secs(1),
            pump_raw_output(
                input,
                &mut output,
                line_tx,
                state.last_output_activity.clone(),
            ),
        )
        .await;

        assert!(result.is_ok());
        assert_eq!(output, b"{\"type\":\"one\"}\n{\"type\":\"two\"}\n");
    }

    #[tokio::test]
    async fn fragmented_overloaded_input_preserves_every_raw_byte() {
        let state = AppState::new();
        let (line_tx, mut line_rx) = mpsc::channel(1);
        line_tx.try_send("already full".to_string()).unwrap();
        let chunks: &[&[u8]] = &[
            b"{\"type\":\"ass",
            b"istant\"}\nnot json\n{\"unknown\":",
            b"true}\n{\"partial\":true}",
        ];
        let expected = chunks.concat();
        let mut output = Vec::new();

        tokio::time::timeout(
            Duration::from_secs(1),
            pump_raw_output(
                FragmentedReader::new(chunks),
                &mut output,
                line_tx,
                state.last_output_activity.clone(),
            ),
        )
        .await
        .expect("pump deadlocked")
        .expect("pump failed");

        assert_eq!(output, expected);
        assert_eq!(line_rx.try_recv().as_deref(), Ok("already full"));
    }

    #[tokio::test]
    async fn waits_for_continue_after_child_exit_when_lockout_is_active() {
        let (resume_tx, mut resume_rx) = mpsc::unbounded_channel();

        let wait_task =
            tokio::spawn(async move { await_resume_after_exit(true, &mut resume_rx).await });

        tokio::time::sleep(Duration::from_millis(25)).await;
        assert!(!wait_task.is_finished());

        resume_tx
            .send(crate::resume::StreamResumeCommand::Continue)
            .expect("send continue");

        assert!(matches!(
            wait_task.await.expect("join wait task"),
            StreamProcessAction::Restart
        ));
    }

    #[tokio::test]
    async fn consumes_queued_continue_after_child_exit_without_lockout() {
        let (resume_tx, mut resume_rx) = mpsc::unbounded_channel();
        resume_tx
            .send(crate::resume::StreamResumeCommand::Continue)
            .expect("queue continue");

        assert!(matches!(
            await_resume_after_exit(false, &mut resume_rx).await,
            StreamProcessAction::Restart
        ));
    }

    #[test]
    fn resume_command_preserves_original_program() {
        assert_eq!(
            resume_command_with_program("/opt/bin/claude", "session-123"),
            CommandSpec {
                program: "/opt/bin/claude".to_string(),
                args: vec![
                    "--resume".to_string(),
                    "session-123".to_string(),
                    "-p".to_string(),
                    "--output-format".to_string(),
                    "stream-json".to_string(),
                    "--verbose".to_string(),
                    "--include-partial-messages".to_string(),
                    "continue".to_string(),
                ],
            }
        );
    }
}
