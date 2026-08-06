//! Acceptance harness for the same production SessionController used by Tauri commands.
use std::{
    env,
    fs::File,
    io::{Read, Write},
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};
use wvu_bioinstrumentation_studio_lib::{
    arduino_cli::ArduinoCli,
    protocol::{encode_frame, Frame, FrameParser, MessageType},
    recording::{BmegReader, RecordingDuration, RecordingMetadata, StopReason},
    session::{ResetTarget, SessionController},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mode = env::args()
        .nth(1)
        .unwrap_or_else(|| "simulator".to_string());
    let duration_argument = env::args().nth(2).unwrap_or_else(|| "10".to_string());
    let until_stopped = duration_argument.eq_ignore_ascii_case("until");
    let seconds = duration_argument
        .parse::<u64>()
        .unwrap_or(RecordingDuration::MIN_TIMED_SECONDS)
        .max(RecordingDuration::MIN_TIMED_SECONDS);
    let manual_stop_after = env::args()
        .nth(3)
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(15)
        .max(1);
    let duration = if until_stopped {
        RecordingDuration::UntilStopped
    } else {
        RecordingDuration::Timed { seconds }
    };
    let session = SessionController::default();
    let output_dir = PathBuf::from("recordings");
    let capture_started = Instant::now();
    if mode == "validate" {
        let bmeg_path = PathBuf::from(&duration_argument);
        let metadata_path = bmeg_path.with_extension("metadata.json");
        let csv_path = bmeg_path.with_extension("csv");
        let metadata_file = File::open(&metadata_path)?;
        let metadata: RecordingMetadata = serde_json::from_reader(metadata_file)?;
        let mut reader = BmegReader::open(&bmeg_path)?;
        let mut records = 0u64;
        let mut first_sequence: Option<u32> = None;
        let mut first_timestamp: Option<u64> = None;
        let mut previous_sequence: Option<u32> = None;
        let mut previous_timestamp: Option<u64> = None;
        while let Some(sample) = reader.next_sample()? {
            if let Some(previous) = previous_sequence {
                if sample.sequence != previous.wrapping_add(1) {
                    return Err(format!("noncontiguous BMEG sequence at record {records}").into());
                }
            } else {
                first_sequence = Some(sample.sequence);
            }
            if let Some(previous) = previous_timestamp {
                if sample.timestamp_us <= previous {
                    return Err(format!("nonmonotonic BMEG timestamp at record {records}").into());
                }
            } else {
                first_timestamp = Some(sample.timestamp_us);
            }
            previous_sequence = Some(sample.sequence);
            previous_timestamp = Some(sample.timestamp_us);
            records += 1;
        }
        let csv_file = File::open(&csv_path)?;
        let mut csv_reader = std::io::BufReader::new(csv_file);
        let mut header = String::new();
        std::io::BufRead::read_line(&mut csv_reader, &mut header)?;
        if header.trim_end()
            != "sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags"
        {
            return Err("unexpected CSV header".into());
        }
        let mut csv_rows = 0u64;
        let mut row = String::new();
        loop {
            row.clear();
            if std::io::BufRead::read_line(&mut csv_reader, &mut row)? == 0 {
                break;
            }
            let fields: Vec<_> = row.trim_end().split(',').collect();
            if fields.len() != 7 || fields[3] != "A0" || fields[6] != "1" {
                return Err(format!("malformed CSV row {}", csv_rows + 1).into());
            }
            let counts = fields[4].parse::<u16>()?;
            let volts = fields[5].parse::<f64>()?;
            let expected_volts = f64::from(counts) * 5.0 / 4095.0;
            if (volts - expected_volts).abs() > 0.000_000_6 {
                return Err(
                    format!("CSV voltage conversion mismatch at row {}", csv_rows + 1).into(),
                );
            }
            csv_rows += 1;
        }
        let first_timestamp = first_timestamp.ok_or("BMEG contains no samples")?;
        let last_timestamp = previous_timestamp.ok_or("BMEG contains no samples")?;
        let measured_rate_hz = (records.saturating_sub(1)) as f64 * 1_000_000.0
            / (last_timestamp.saturating_sub(first_timestamp)) as f64;
        let metadata_valid = metadata.total_samples == records
            && metadata.duration_mode.as_deref() == Some("until_stopped")
            && metadata.requested_duration_seconds.is_none()
            && metadata.stop_reason == Some(StopReason::User)
            && metadata.completion_status == "complete";
        if !metadata_valid || csv_rows != records {
            return Err("metadata or CSV row count does not match BMEG".into());
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "bmeg_path": bmeg_path,
                "metadata_path": metadata_path,
                "csv_path": csv_path,
                "bmeg_records": records,
                "csv_rows": csv_rows,
                "first_sequence": first_sequence,
                "last_sequence": previous_sequence,
                "first_timestamp_us": first_timestamp,
                "last_timestamp_us": last_timestamp,
                "measured_rate_hz_from_timestamps": measured_rate_hz,
                "metadata_total_samples": metadata.total_samples,
                "duration_mode": metadata.duration_mode,
                "stop_reason": metadata.stop_reason,
                "completion_status": metadata.completion_status,
                "integrity": metadata.integrity,
                "validation": "passed"
            }))?
        );
        return Ok(());
    }
    if mode == "reset" {
        let cli = ArduinoCli::discover(None)?;
        let board = cli
            .boards()?
            .into_iter()
            .next()
            .ok_or("no UNO R4 WiFi discovered")?;
        let result = session.reset_and_retry(ResetTarget {
            port: board.port,
            serial_number: board.serial_number,
        })?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return if result.handshake_succeeded {
            Ok(())
        } else {
            Err("reset/retry did not complete a protocol handshake".into())
        };
    }
    if mode == "verify" {
        let cli = ArduinoCli::discover(None)?;
        let board = cli
            .boards()?
            .into_iter()
            .next()
            .ok_or("no UNO R4 WiFi discovered")?;
        let result = session.retry_handshake(ResetTarget {
            port: board.port,
            serial_number: board.serial_number,
        })?;
        println!("{}", serde_json::to_string_pretty(&result)?);
        return if result.handshake_succeeded {
            Ok(())
        } else {
            Err("controlled firmware did not complete a protocol handshake".into())
        };
    }
    if mode == "probe" {
        let cli = ArduinoCli::discover(None)?;
        let board = cli
            .boards()?
            .into_iter()
            .next()
            .ok_or("no UNO R4 WiFi discovered")?;
        let mut port = serialport::new(&board.port, 115_200)
            .timeout(Duration::from_millis(50))
            .open()?;
        port.clear(serialport::ClearBuffer::Input)?;
        port.write_data_terminal_ready(true)?;
        port.write_request_to_send(true)?;
        let ping = encode_frame(&Frame {
            message_type: MessageType::Ping,
            flags: 0,
            sequence: 0,
            payload: vec![],
        })
        .map_err(|error| std::io::Error::other(format!("could not encode PING: {error:?}")))?;
        port.write_all(&ping)?;
        port.flush()?;
        let deadline = Instant::now() + Duration::from_secs(3);
        let mut parser = FrameParser::default();
        let mut raw = Vec::new();
        let mut frames = Vec::new();
        let mut buffer = [0u8; 256];
        while Instant::now() < deadline {
            match port.read(&mut buffer) {
                Ok(count) if count > 0 => {
                    raw.extend_from_slice(&buffer[..count]);
                    frames.extend(parser.push(&buffer[..count]));
                }
                Ok(_) => {}
                Err(error) if error.kind() == std::io::ErrorKind::TimedOut => {}
                Err(error) => return Err(error.into()),
            }
        }
        let parsed: Vec<_> = frames
            .iter()
            .map(|frame| {
                serde_json::json!({
                    "message_type": format!("{:?}", frame.message_type),
                    "sequence": frame.sequence,
                    "payload_hex": frame.payload.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" "),
                })
            })
            .collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "port": board.port,
                "bytes_received": raw.len(),
                "raw_hex": raw.iter().map(|byte| format!("{byte:02X}")).collect::<Vec<_>>().join(" "),
                "frames": parsed,
                "crc_failures": parser.stats.crc_failures,
                "invalid_frames": parser.stats.invalid_frames,
                "skipped_noise_bytes": parser.stats.skipped_noise_bytes,
            }))?
        );
        return Ok(());
    }
    if mode == "hardware" {
        let cli = ArduinoCli::discover(None)?;
        let board = cli
            .boards()?
            .into_iter()
            .next()
            .ok_or("no UNO R4 WiFi discovered")?;
        session.start_serial(board.port, duration.clone(), output_dir)?;
    } else {
        session.start_simulator(duration.clone(), output_dir)?;
    }
    let deadline = Instant::now()
        + Duration::from_secs(if until_stopped {
            manual_stop_after + 20
        } else {
            seconds + 20
        });
    let mut stop_requested = false;
    loop {
        let status = session.status()?;
        if let Some(summary) = status.last_summary {
            println!("{}", serde_json::to_string_pretty(&summary)?);
            return if summary.error.is_some() {
                Err("session faulted".into())
            } else {
                Ok(())
            };
        }
        if status.state == wvu_bioinstrumentation_studio_lib::session::SessionState::Faulted {
            return Err(status
                .last_error
                .unwrap_or_else(|| "session faulted before recording finalization".into())
                .into());
        }
        if Instant::now() >= deadline {
            session.request_stop()?;
            session.wait_for_worker()?;
            return Err("timed out waiting for session worker".into());
        }
        if until_stopped
            && !stop_requested
            && capture_started.elapsed() >= Duration::from_secs(manual_stop_after)
        {
            session.request_stop()?;
            stop_requested = true;
        }
        thread::sleep(Duration::from_millis(40));
    }
}
