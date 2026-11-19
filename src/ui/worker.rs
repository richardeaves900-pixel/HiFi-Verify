use crate::alsa_proc::read_alsa_hw_params;
use crate::model::{HwParams, Sink};
use crate::pwtop::{self, NodeRow};
use crate::telemetry::{self, PcmPathAssessment, StreamInfo};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Duration;

#[derive(Default)]
pub struct Snapshot {
    pub rows: Vec<NodeRow>,
    pub sinks: Vec<Sink>,
    pub stream: StreamInfo,
    pub path: Option<PcmPathAssessment>,
    pub processing: Vec<String>,
    pub dac_hw: Option<HwParams>,
    pub driver_rate: Option<u32>,
    pub driver_quantum: Option<u32>,
    pub total_err: u64,
    pub wait_us: Option<f64>,
    pub pwtop_updated: bool,
    pub volume: Option<f64>,
}

pub fn spawn_worker(tx: Sender<super::UiEvent>, stop: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let mut tick: u64 = 0;
        let mut last_rows: Vec<NodeRow> = Vec::new();
        let mut last_err: u64 = 0;
        let mut last_node_err: HashMap<String, u64> = HashMap::new();
        let mut xrun_log_started = false;
        let mut graph = telemetry::PwGraph::default();
        let mut stream = StreamInfo::default();
        let mut volume = None;

        while !stop.load(Ordering::Relaxed) {
            let mut pwtop_updated = false;

            // pw-top is expensive (~200 ms). Refresh every 4th tick ≈ 2 s.
            if tick.is_multiple_of(4) {
                // Only update counters when pw-top succeeds; transient timeouts
                // must never reset last_err to 0 and cause a false XRUN spike.
                if let Some(pw) = pwtop::parse_pw_top() {
                    if xrun_log_started {
                        let mut logged_any = false;
                        for row in &pw.rows {
                            let prev = last_node_err.get(&row.name).copied().unwrap_or(0);
                            if row.err > prev {
                                crate::history::log_node_xruns(&row.name, row.err - prev);
                                logged_any = true;
                            }
                        }
                        if !logged_any && pw.total_err > last_err {
                            crate::history::log_node_xruns("system", pw.total_err - last_err);
                        }
                    }
                    last_node_err.clear();
                    for row in &pw.rows {
                        last_node_err.insert(row.name.clone(), row.err);
                    }
                    xrun_log_started = true;
                    last_rows = pw.rows;
                    last_err = pw.total_err;
                    pwtop_updated = true;
                }
            }

            let hw = read_alsa_hw_params();
            if tick.is_multiple_of(4) {
                graph = telemetry::PwGraph::load();
            }
            if tick.is_multiple_of(2) {
                stream = telemetry::detect_active_stream(&graph).unwrap_or_default();
                volume = crate::volume::try_get_volume();
            }

            let processing = graph.processing.clone();
            let wait_us = last_rows
                .iter()
                .find(|r| r.status == 'R' && r.wait_us.is_some())
                .and_then(|r| r.wait_us);

            let (node_rate, node_quantum) = telemetry::observed_output_clock(&last_rows);
            let path = graph.assess_pcm_path(&last_rows, &hw, &stream);
            let active_rate = path.output_rate.or(node_rate);
            let active_quantum = path.output_quantum.or(node_quantum);

            let snap = Snapshot {
                rows: last_rows.clone(),
                sinks: graph.sinks.clone(),
                stream: stream.clone(),
                path: Some(path),
                processing,
                dac_hw: graph.hardware_for_route(&hw).cloned(),
                driver_rate: active_rate,
                driver_quantum: active_quantum,
                total_err: last_err,
                wait_us,
                pwtop_updated,
                volume,
            };

            if tx.send(super::UiEvent::Snapshot(Box::new(snap))).is_err() {
                break;
            }
            tick = tick.wrapping_add(1);

            for _ in 0..5 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    });
}
