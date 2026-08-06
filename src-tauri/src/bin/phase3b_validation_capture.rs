//! Bench-only Phase 3B simulator acceptance harness.
//!
//! It invokes the same `SessionController` methods exposed to the Tauri
//! validation commands. No serial port, person, electrode system, or biomedical
//! module is opened by this binary.
use std::{
    collections::BTreeMap,
    env, fs,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};
use wvu_bioinstrumentation_studio_lib::{
    profiles::{built_in_profiles, ProfileMode},
    recording::{BmegReader, RecordingDuration, ValidationRunContext},
    session::SessionController,
    validation::{
        evaluate_criteria, metrics_for_validation_run, AcceptanceCriterion, CriterionOperator,
        EquipmentItem, ProfileValidationStatus, ValidationEvidenceStatus, ValidationHardware,
        ValidationRun, ValidationStore, ValidationTestType, METRIC_ALGORITHM_VERSION,
    },
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = env::args().nth(1).map(PathBuf::from).unwrap_or_else(|| {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or_default();
        env::temp_dir().join(format!("wvu_phase3b_simulator_{stamp}"))
    });
    fs::create_dir_all(&root)?;
    let profile = built_in_profiles()?
        .into_iter()
        .find(|profile| profile.category == "ecg")
        .ok_or("ECG profile unavailable")?;
    let store = ValidationStore::with_root(root.join("evidence"))?;
    let validation_id = "wvu.bmeg420l.ecg.interface.validation.simulator.001";
    let hardware = ValidationHardware {
        board: "Arduino UNO R4 WiFi".into(),
        board_serial: "SIMULATOR".into(),
        com_port: "SIM".into(),
        firmware_build: profile.required_firmware.build.clone(),
        firmware_device: profile.required_firmware.device.clone(),
        module_name: "Simulator only — no physical module".into(),
        module_identifier: "SIM".into(),
        module_revision: String::new(),
        module_serial: String::new(),
    };
    store.create_draft(
        ProfileMode::InstructorAuthoring,
        &profile,
        validation_id.into(),
        hardware.clone(),
    )?;
    store.update_draft_details(ProfileMode::InstructorAuthoring, validation_id, hardware, vec![EquipmentItem { name: "Deterministic protocol simulator".into(), identifier: "phase3b-deterministic-v1".into(), calibration_or_notes: "Synthetic, nonphysiological source used only for software-path acceptance.".into() }], BTreeMap::from([("safety".into(), "Bench-only; simulator; no person or electrode system connected.".into())]), "Simulator acceptance only; it does not substitute for physical ECG/EMG interface validation.".into())?;

    let plan = [
        (
            ValidationTestType::Baseline,
            2.5,
            None,
            None,
            "deterministic 2.5 V baseline",
            criterion(
                "measured_sample_rate_hz",
                CriterionOperator::GreaterThanOrEqual,
                999.0,
                "Hz",
            ),
        ),
        (
            ValidationTestType::DcSweep,
            2.5,
            Some(2.5),
            None,
            "deterministic 2.5 V DC",
            criterion(
                "absolute_voltage_error",
                CriterionOperator::LessThanOrEqual,
                0.001,
                "V",
            ),
        ),
        (
            ValidationTestType::SineWave,
            2.5,
            None,
            Some(50.0),
            "deterministic 50 Hz sine, 2.5 V offset, 1.0 Vpp",
            criterion(
                "absolute_frequency_error_hz",
                CriterionOperator::LessThanOrEqual,
                0.2,
                "Hz",
            ),
        ),
        (
            ValidationTestType::SaturationMargin,
            2.5,
            None,
            None,
            "intentional 0/5 V clipping test",
            criterion(
                "clipping_percentage",
                CriterionOperator::GreaterThanOrEqual,
                99.0,
                "%",
            ),
        ),
        (
            ValidationTestType::Repeatability,
            2.5,
            None,
            None,
            "repeatability 1: deterministic 2.5 V",
            criterion(
                "mean_volts",
                CriterionOperator::GreaterThanOrEqual,
                2.49,
                "V",
            ),
        ),
        (
            ValidationTestType::Repeatability,
            2.5,
            None,
            None,
            "repeatability 2: deterministic 2.5 V",
            criterion(
                "mean_volts",
                CriterionOperator::GreaterThanOrEqual,
                2.49,
                "V",
            ),
        ),
        (
            ValidationTestType::Repeatability,
            2.5,
            None,
            None,
            "repeatability 3: deterministic 2.5 V",
            criterion(
                "mean_volts",
                CriterionOperator::GreaterThanOrEqual,
                2.49,
                "V",
            ),
        ),
    ];
    let mut all_criteria = Vec::new();
    let mut summaries = Vec::new();
    for (index, (test_type, offset_v, setpoint_v, frequency_hz, description, criterion)) in
        plan.into_iter().enumerate()
    {
        let run_number = index as u32 + 1;
        let context = ValidationRunContext {
            validation_id: validation_id.into(),
            test_type: test_type.label().into(),
            run_number,
            bench_only: true,
            source_description: description.into(),
            source_setpoint_v: setpoint_v,
            source_offset_v: Some(offset_v),
            source_frequency_hz: frequency_hz,
            source_peak_to_peak_v: (test_type == ValidationTestType::SineWave).then_some(1.0),
            equipment_metadata: BTreeMap::from([("source".into(), description.into())]),
            simulator_parameters: BTreeMap::from([(
                "seed".into(),
                "phase3b-deterministic-v1".into(),
            )]),
        };
        let summary = SessionController::default().capture_simulator_validation(
            profile.snapshot(true),
            RecordingDuration::Timed { seconds: 10 },
            &root.join("recordings"),
            context.clone(),
        )?;
        let reader = BmegReader::open(&PathBuf::from(&summary.bmeg_path))?;
        if reader.metadata.validation_context.as_ref() != Some(&context) {
            return Err("validation context was not preserved in the BMEG header".into());
        }
        let (_sample_metrics, metrics) = metrics_for_validation_run(
            &PathBuf::from(&summary.bmeg_path),
            &test_type,
            setpoint_v,
            frequency_hz,
        )?;
        let criteria = evaluate_criteria(&metrics, &[criterion]);
        if criteria.iter().any(|result| !result.passed) {
            return Err(format!("simulator criterion failed for {}", test_type.label()).into());
        }
        all_criteria.extend(criteria.clone());
        store.add_run(
            ProfileMode::InstructorAuthoring,
            validation_id,
            ValidationRun {
                run_number,
                test_type,
                source_description: description.into(),
                source_setpoint_v: setpoint_v,
                source_frequency_hz: frequency_hz,
                source_peak_to_peak_v: (frequency_hz.is_some()).then_some(1.0),
                bmeg_path: summary.bmeg_path.clone(),
                metadata_path: summary.metadata_path.clone(),
                csv_path: summary.csv_path.clone(),
                raw_sample_count: summary.samples,
                algorithm_version: METRIC_ALGORITHM_VERSION.into(),
                metrics,
                criteria,
                notes: "Raw BMEG, metadata, and CSV retained separately.".into(),
            },
        )?;
        summaries.push(summary);
    }
    store.set_acceptance_summary(
        ProfileMode::InstructorAuthoring,
        validation_id,
        all_criteria,
        true,
    )?;
    let finalized = store.finalize(ProfileMode::InstructorAuthoring, validation_id, &profile)?;
    if finalized.status != ValidationEvidenceStatus::Finalized {
        return Err("evidence did not finalize".into());
    }
    let profile_status = store.profile_status(&profile)?;
    if profile_status.status != ProfileValidationStatus::Unvalidated {
        return Err(
            "simulator evidence must not promote a profile to physical bench validation".into(),
        );
    }
    let package = store.export_package(
        ProfileMode::InstructorAuthoring,
        validation_id,
        &root.join("packages"),
    )?;
    let imported = ValidationStore::with_root(root.join("imported"))?.import_package(
        ProfileMode::InstructorAuthoring,
        &package,
        &profile,
    )?;
    let output = serde_json::json!({
        "result": "passed", "scope": "simulator only; no hardware or human-connected validation",
        "validation_id": finalized.validation_id, "validation_hash": finalized.integrity.canonical_hash,
        "profile_status": profile_status.status, "profile_status_message": profile_status.explanation, "package": package, "imported_hash": imported.integrity.canonical_hash,
        "runs": summaries.iter().map(|summary| serde_json::json!({"samples": summary.samples, "packets": summary.packets, "rate_hz": summary.measured_rate_hz, "bmeg": summary.bmeg_path, "csv": summary.csv_path, "metadata": summary.metadata_path, "integrity": summary.integrity})).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn criterion(
    metric: &str,
    operator: CriterionOperator,
    threshold: f64,
    units: &str,
) -> AcceptanceCriterion {
    AcceptanceCriterion {
        metric: metric.into(),
        operator,
        threshold,
        units: units.into(),
    }
}
