//! Hardware acceptance harness for the production Phase 2 workflow.
//!
//! It deliberately calls `FirmwareWorkspace`, `FirmwareWorkflow`, and the
//! same `SessionController` held by the Tauri application. It does not own a
//! second serial transport and does not send a shell command string.
use std::{
    fs,
    io::{BufRead, BufReader},
    path::PathBuf,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use wvu_bioinstrumentation_studio_lib::{
    arduino_cli::ArduinoCli,
    firmware_workflow::{
        CompileProjectRequest, FirmwareCompatibility, FirmwareFailure, FirmwareWorkflow,
        RestoreReferenceRequest, UploadProjectRequest,
    },
    firmware_workspace::{CreateProjectRequest, FirmwareWorkspace, TemplateKind},
    recording::{BmegReader, RecordingDuration, RecordingMetadata, StopReason},
    session::SessionController,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = std::env::args().skip(1);
    if arguments.next().as_deref() == Some("validate") {
        let bmeg = PathBuf::from(arguments.next().ok_or("provide a .bmeg path to validate")?);
        let metadata = bmeg.with_extension("metadata.json");
        let csv = bmeg.with_extension("csv");
        println!(
            "{}",
            serde_json::to_string_pretty(&validate_recording(
                &bmeg.display().to_string(),
                &metadata.display().to_string(),
                &csv.display().to_string(),
            )?)?
        );
        return Ok(());
    }
    let cli = ArduinoCli::discover(None)?;
    let board = cli
        .boards()?
        .into_iter()
        .next()
        .ok_or("no Arduino UNO R4 WiFi was discovered")?;
    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis();
    let root = std::env::temp_dir().join(format!("wvu_phase2_acceptance_{unique}"));
    fs::create_dir_all(&root)?;
    let workspace = FirmwareWorkspace::new(root.join("workspace_data"));
    let project = workspace.create_project(CreateProjectRequest {
        parent_folder: root.display().to_string(),
        project_name: "A0AsciiDiagnostic".into(),
        template: TemplateKind::A0AcquisitionExample,
        notes: Some("Phase 2 hardware acceptance; UNO alone, no biomedical accessory.".into()),
        overwrite_confirmed: false,
    })?;
    let session = SessionController::default();
    let workflow = FirmwareWorkflow::with_workspace(workspace, session.clone());

    workflow
        .start_compile(CompileProjectRequest {
            project_folder: project.project_folder.clone(),
            unsaved_changes: false,
        })
        .map_err(workflow_error)?;
    let compiled = wait_for_job(&workflow)?;
    let compile = compiled
        .last_compile
        .as_ref()
        .ok_or("compile job did not produce a result")?;
    if compile.failure.is_some() {
        return Err(format!("A0 diagnostic compile failed: {}", compile.message).into());
    }

    workflow
        .start_upload(UploadProjectRequest {
            project_folder: project.project_folder.clone(),
            port: board.port.clone(),
            unsaved_changes: false,
            confirmation: true,
        })
        .map_err(workflow_error)?;
    let non_wvu_uploaded = wait_for_job(&workflow)?;
    let non_wvu = non_wvu_uploaded
        .last_upload
        .as_ref()
        .ok_or("A0 diagnostic upload did not produce a result")?;
    if non_wvu.failure.is_some()
        || non_wvu_uploaded.compatibility != FirmwareCompatibility::NonWvuSketch
        || non_wvu
            .verification
            .as_ref()
            .is_none_or(|verification| verification.compatible)
    {
        return Err(
            "A0 diagnostic upload did not produce the required non-WVU compatibility state".into(),
        );
    }
    let final_non_wvu_port = non_wvu.final_port.clone().unwrap_or(board.port.clone());

    workflow
        .start_restore_reference(RestoreReferenceRequest {
            port: final_non_wvu_port,
            confirmation: true,
        })
        .map_err(workflow_error)?;
    let restored = wait_for_job(&workflow)?;
    let restore = restored
        .last_upload
        .as_ref()
        .ok_or("reference restore did not produce a result")?;
    if restore.failure.is_some()
        || restored.compatibility != FirmwareCompatibility::WvuProtocolCompatible
        || !restore
            .verification
            .as_ref()
            .is_some_and(|verification| verification.compatible)
    {
        return Err(format!("reference restore verification failed: {}", restore.message).into());
    }
    let final_port = restore.final_port.clone().unwrap_or(board.port.clone());
    if !workflow
        .is_acquisition_allowed(&final_port)
        .map_err(workflow_error)?
    {
        return Err(
            "workflow did not re-enable acquisition after verified reference restore".into(),
        );
    }

    let recording_dir = root.join("recordings");
    session.start_serial(
        final_port.clone(),
        RecordingDuration::Timed { seconds: 30 },
        recording_dir,
    )?;
    let summary = wait_for_recording(&session)?;
    if summary.error.is_some() {
        return Err(format!("30-second acquisition faulted: {:?}", summary.error).into());
    }
    let validation = validate_recording(
        &summary.bmeg_path,
        &summary.metadata_path,
        &summary.csv_path,
    )?;
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "board": board,
            "a0_project": project.project_folder,
            "compile": compile,
            "non_wvu_upload": non_wvu,
            "reference_restore": restore,
            "final_port": final_port,
            "acquisition_summary": summary,
            "export_validation": validation,
            "temporary_root": root,
            "acceptance": "passed"
        }))?
    );
    Ok(())
}

fn wait_for_job(
    workflow: &FirmwareWorkflow,
) -> Result<
    wvu_bioinstrumentation_studio_lib::firmware_workflow::FirmwareWorkflowStatus,
    Box<dyn std::error::Error>,
> {
    let deadline = Instant::now() + Duration::from_secs(180);
    loop {
        let status = workflow.status().map_err(workflow_error)?;
        if status.job.is_none() {
            return Ok(status);
        }
        if Instant::now() >= deadline {
            return Err("timed out waiting for firmware workflow job".into());
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn wait_for_recording(
    session: &SessionController,
) -> Result<wvu_bioinstrumentation_studio_lib::session::SessionSummary, Box<dyn std::error::Error>>
{
    let deadline = Instant::now() + Duration::from_secs(55);
    loop {
        let status = session.status()?;
        if let Some(summary) = status.last_summary {
            return Ok(summary);
        }
        if Instant::now() >= deadline {
            let _ = session.request_stop();
            let _ = session.wait_for_worker();
            return Err("timed out waiting for 30-second acquisition finalization".into());
        }
        thread::sleep(Duration::from_millis(40));
    }
}

fn validate_recording(
    bmeg: &str,
    metadata: &str,
    csv: &str,
) -> Result<serde_json::Value, Box<dyn std::error::Error>> {
    let recording_metadata: RecordingMetadata = serde_json::from_slice(&fs::read(metadata)?)?;
    let mut reader = BmegReader::open(&PathBuf::from(bmeg))?;
    let mut csv_reader = BufReader::new(fs::File::open(csv)?);
    let mut header = String::new();
    csv_reader.read_line(&mut header)?;
    if header.trim_end()
        != "sample_sequence,timestamp_us,elapsed_seconds,channel,adc_counts,volts,status_flags"
    {
        return Err("unexpected CSV header".into());
    }
    let mut records = 0u64;
    let mut previous_sequence: Option<u32> = None;
    let mut previous_timestamp: Option<u64> = None;
    let mut first_timestamp: Option<u64> = None;
    while let Some(sample) = reader.next_sample()? {
        if let Some(previous) = previous_sequence {
            if sample.sequence != previous.wrapping_add(1) {
                return Err("BMEG sample sequence was not contiguous".into());
            }
        }
        if let Some(previous) = previous_timestamp {
            if sample.timestamp_us <= previous {
                return Err("BMEG timestamp was not monotonic".into());
            }
        }
        if first_timestamp.is_none() {
            first_timestamp = Some(sample.timestamp_us);
        }
        let mut row = String::new();
        if csv_reader.read_line(&mut row)? == 0 {
            return Err("CSV ended before BMEG records".into());
        }
        let fields: Vec<_> = row.trim_end().split(',').collect();
        if fields.len() != 7 || fields[3] != "A0" || fields[6] != "1" {
            return Err("CSV row did not match the documented Phase 1 format".into());
        }
        if fields[0].parse::<u32>()? != sample.sequence
            || fields[1].parse::<u64>()? != sample.timestamp_us
            || fields[4].parse::<u16>()? != sample.counts
        {
            return Err("CSV sample data did not match BMEG".into());
        }
        let volts = fields[5].parse::<f64>()?;
        let expected_volts = f64::from(sample.counts) * 5.0 / 4095.0;
        if (volts - expected_volts).abs() > 0.000_000_6 {
            return Err("CSV voltage conversion did not match counts * 5.0 / 4095.0".into());
        }
        previous_sequence = Some(sample.sequence);
        previous_timestamp = Some(sample.timestamp_us);
        records += 1;
    }
    let mut trailing = String::new();
    if csv_reader.read_line(&mut trailing)? != 0 {
        return Err("CSV contained records beyond BMEG".into());
    }
    let csv_rows = records;
    if records == 0 || records != csv_rows || records != recording_metadata.total_samples {
        return Err("BMEG, CSV, and metadata record counts do not match".into());
    }
    if recording_metadata.duration_mode.as_deref() != Some("timed")
        || recording_metadata.requested_duration_seconds != Some(30)
        || recording_metadata.stop_reason != Some(StopReason::TimedComplete)
        || recording_metadata.completion_status != "complete"
        || recording_metadata.adc_bits != 12
        || recording_metadata.requested_sample_rate_hz != 1000
    {
        return Err(
            "recording metadata did not match the Phase 2 30-second acceptance configuration"
                .into(),
        );
    }
    let integrity = &recording_metadata.integrity;
    if integrity.crc_failures != 0
        || integrity.invalid_frames != 0
        || integrity.missing_packet_sequences != 0
        || integrity.missing_sample_sequences != 0
        || integrity.duplicate_packets != 0
        || integrity.out_of_order_packets != 0
        || integrity.firmware_overflows != 0
        || integrity.host_channel_overflows != 0
        || integrity.disconnect_events != 0
        || integrity.reconnects != 0
    {
        return Err("recording metadata reported an unexpected integrity failure".into());
    }
    let first_timestamp = first_timestamp.ok_or("BMEG contains no records")?;
    let last_timestamp = previous_timestamp.ok_or("BMEG contains no records")?;
    let measured_rate_from_timestamps = (records.saturating_sub(1)) as f64 * 1_000_000.0
        / (last_timestamp.saturating_sub(first_timestamp)) as f64;
    Ok(serde_json::json!({
        "bmeg_records": records,
        "csv_rows": csv_rows,
        "metadata_total_samples": recording_metadata.total_samples,
        "first_timestamp_us": first_timestamp,
        "last_timestamp_us": last_timestamp,
        "measured_rate_hz_from_timestamps": measured_rate_from_timestamps,
        "metadata_measured_rate_hz": recording_metadata.measured_sample_rate_hz,
        "completion_status": recording_metadata.completion_status,
        "stop_reason": recording_metadata.stop_reason,
        "integrity": recording_metadata.integrity,
        "validation": "passed"
    }))
}

fn workflow_error(error: FirmwareFailure) -> std::io::Error {
    std::io::Error::other(format!("firmware workflow error: {error:?}"))
}
