use crate::domain::{ProjectId, StageCommand, StageCompleted, StageId, StageState};
use crate::error::{AutoForgeError, Result};
#[cfg(test)]
use crate::services::quality::MAX_DEBUG_CYCLES;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// 품질 게이트 상태 — Verify ↔ Debug 루프 + SecurityPatch
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QualityGate {
    pub verify_passed: bool,
    pub debug_cycles: u8,
    pub awaiting_debug: bool,
    pub security_done: bool,
    pub max_debug_cycles: u8,
}

impl QualityGate {
    pub fn with_max_cycles(max: u8) -> Self {
        Self {
            max_debug_cycles: max,
            ..Default::default()
        }
    }

    pub fn record_verify_failed(&mut self) {
        self.verify_passed = false;
        self.awaiting_debug = self.debug_cycles < self.max_debug_cycles;
    }

    pub fn record_verify_passed(&mut self) {
        self.verify_passed = true;
        self.awaiting_debug = false;
    }

    pub fn record_debug_done(&mut self) {
        self.debug_cycles += 1;
        self.awaiting_debug = false;
    }

    pub fn verify_exhausted(&self) -> bool {
        !self.verify_passed && self.debug_cycles >= self.max_debug_cycles && !self.awaiting_debug
    }
}

/// 아키텍처 설계 Q&A 게이트 — draft 질문 생성 → 사용자 답변 → finalize
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ArchitectureGate {
    pub draft_done: bool,
    pub awaiting_answers: bool,
    pub finalized: bool,
}

impl ArchitectureGate {
    pub fn record_draft(&mut self) {
        self.draft_done = true;
        self.awaiting_answers = true;
    }

    pub fn record_answers_submitted(&mut self) {
        self.awaiting_answers = false;
    }

    pub fn record_finalized(&mut self) {
        self.finalized = true;
        self.awaiting_answers = false;
    }
}

/// DAG 스케줄러 — 순수 로직, I/O 없음
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DagScheduler {
    completed: HashSet<StageId>,
    running: HashSet<StageId>,
    failed: Option<StageId>,
    project_id: ProjectId,
    pub quality: QualityGate,
    pub architecture: ArchitectureGate,
}

impl DagScheduler {
    pub fn new() -> Self {
        Self {
            completed: HashSet::new(),
            running: HashSet::new(),
            failed: None,
            project_id: ProjectId::new(),
            quality: QualityGate::default(),
            architecture: ArchitectureGate::default(),
        }
    }

    pub fn with_project(project_id: ProjectId) -> Self {
        Self {
            project_id,
            ..Self::new()
        }
    }

    pub fn with_quality(project_id: ProjectId, max_debug_cycles: u8) -> Self {
        Self {
            project_id,
            quality: QualityGate::with_max_cycles(max_debug_cycles),
            ..Self::new()
        }
    }

    pub fn mark_running(&mut self, stage: StageId) {
        self.running.insert(stage);
    }

    pub fn mark_completed(&mut self, event: &StageCompleted) {
        self.running.remove(&event.stage);
        self.completed.insert(event.stage);
    }

    pub fn mark_failed(&mut self, stage: StageId) {
        self.running.remove(&stage);
        self.failed = Some(stage);
    }

    /// Verify 결과 기록 — 통과 시 completed, 실패 시 Debug 루프 또는 실패
    pub fn record_verify_result(&mut self, passed: bool) {
        self.running.remove(&StageId::Verify);
        if passed {
            self.quality.record_verify_passed();
            self.completed.insert(StageId::Verify);
        } else if self.quality.verify_exhausted()
            || self.quality.debug_cycles >= self.quality.max_debug_cycles
        {
            self.failed = Some(StageId::Verify);
        } else {
            self.quality.record_verify_failed();
            // Verify는 통과할 때까지 completed에 넣지 않음
        }
    }

    pub fn record_debug_done(&mut self) {
        self.running.remove(&StageId::Debug);
        self.quality.record_debug_done();
    }

    pub fn record_security_done(&mut self) {
        self.running.remove(&StageId::SecurityPatch);
        self.quality.security_done = true;
        self.completed.insert(StageId::SecurityPatch);
    }

    pub fn record_architect_draft(&mut self) {
        self.running.remove(&StageId::Architect);
        self.architecture.record_draft();
    }

    pub fn record_architect_answers_submitted(&mut self) {
        self.architecture.record_answers_submitted();
    }

    pub fn record_architect_finalized(&mut self) {
        self.running.remove(&StageId::Architect);
        self.architecture.record_finalized();
        self.completed.insert(StageId::Architect);
    }

    pub fn is_awaiting_architecture_input(&self) -> bool {
        self.architecture.awaiting_answers
    }

    pub fn ready_stages(&self) -> Vec<StageCommand> {
        if self.failed.is_some() {
            return vec![];
        }

        let mut ready = Vec::new();

        if !self.completed.contains(&StageId::Ingest) && self.running.is_empty() {
            return vec![StageCommand {
                project_id: self.project_id.clone(),
                stage: StageId::Ingest,
                attempt: 0,
            }];
        }

        if self.completed.contains(&StageId::Ingest)
            && !self.completed.contains(&StageId::Summarize)
            && !self.running.contains(&StageId::Summarize)
        {
            ready.push(StageId::Summarize);
        }

        if self.completed.contains(&StageId::Summarize) {
            if !self.architecture.finalized
                && !self.architecture.awaiting_answers
                && !self.running.contains(&StageId::Architect)
            {
                ready.push(StageId::Architect);
            }

            if !self.completed.contains(&StageId::Design)
                && !self.running.contains(&StageId::Design)
            {
                ready.push(StageId::Design);
            }
        }

        let arch_done = self.architecture.finalized;
        let design_done = self.completed.contains(&StageId::Design);
        if arch_done
            && design_done
            && !self.completed.contains(&StageId::Implement)
            && !self.running.contains(&StageId::Implement)
        {
            ready.push(StageId::Implement);
        }

        // ── 품질 게이트: Verify ↔ Debug 루프 → SecurityPatch ──
        if self.completed.contains(&StageId::Implement) && !self.quality.verify_passed {
            if self.quality.awaiting_debug
                && !self.running.contains(&StageId::Debug)
                && !self.running.contains(&StageId::Verify)
            {
                ready.push(StageId::Debug);
            } else if !self.quality.awaiting_debug
                && !self.running.contains(&StageId::Verify)
                && !self.running.contains(&StageId::Debug)
            {
                ready.push(StageId::Verify);
            }
        }

        if self.quality.verify_passed
            && !self.quality.security_done
            && !self.running.contains(&StageId::SecurityPatch)
        {
            ready.push(StageId::SecurityPatch);
        }

        if self.quality.security_done
            && !self.completed.contains(&StageId::Deliver)
            && !self.running.contains(&StageId::Deliver)
        {
            ready.push(StageId::Deliver);
        }

        ready
            .into_iter()
            .map(|stage| StageCommand {
                project_id: self.project_id.clone(),
                stage,
                attempt: 0,
            })
            .collect()
    }

    pub fn is_pipeline_complete(&self) -> bool {
        self.completed.contains(&StageId::Deliver)
    }

    pub fn has_failed(&self) -> bool {
        self.failed.is_some()
    }
}

impl Default for DagScheduler {
    fn default() -> Self {
        Self::new()
    }
}

pub struct ProjectTracker {
    stages: HashMap<StageId, StageState>,
}

impl Default for ProjectTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectTracker {
    pub fn new() -> Self {
        Self {
            stages: HashMap::new(),
        }
    }

    pub fn transition(&mut self, stage: StageId, to: StageState) -> Result<()> {
        if let Some(current) = self.stages.get(&stage) {
            if !is_valid_transition(*current, to) {
                return Err(AutoForgeError::Orchestrator(format!(
                    "invalid transition for {stage:?}: {current:?} -> {to:?}"
                )));
            }
        }
        self.stages.insert(stage, to);
        Ok(())
    }
}

fn is_valid_transition(from: StageState, to: StageState) -> bool {
    matches!(
        (from, to),
        (StageState::Queued, StageState::Running)
            | (StageState::Running, StageState::Completed)
            | (StageState::Running, StageState::Failed)
            | (StageState::Failed, StageState::Queued)
            | (StageState::Completed, StageState::Queued) // Debug 루프 재시도
            | (_, StageState::Skipped)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_through(sched: &mut DagScheduler, stage: StageId) {
        sched.mark_completed(&StageCompleted {
            project_id: ProjectId::new(),
            stage,
            output_artifacts: vec![],
        });
    }

    #[test]
    fn parallel_architect_and_design_after_summarize() {
        let mut sched = DagScheduler::new();
        complete_through(&mut sched, StageId::Ingest);
        complete_through(&mut sched, StageId::Summarize);

        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::Architect));
        assert!(ready.contains(&StageId::Design));
    }

    #[test]
    fn architect_awaiting_answers_blocks_implement() {
        let mut sched = DagScheduler::new();
        for stage in [StageId::Ingest, StageId::Summarize] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_draft();
        complete_through(&mut sched, StageId::Design);

        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(!ready.contains(&StageId::Architect));
        assert!(!ready.contains(&StageId::Implement));
        assert!(sched.is_awaiting_architecture_input());
    }

    #[test]
    fn architect_finalize_enables_implement() {
        let mut sched = DagScheduler::new();
        for stage in [StageId::Ingest, StageId::Summarize, StageId::Design] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_draft();
        sched.record_architect_answers_submitted();
        sched.record_architect_finalized();

        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::Implement));
    }

    #[test]
    fn implement_waits_for_both_architect_and_design() {
        let mut sched = DagScheduler::new();
        for stage in [StageId::Ingest, StageId::Summarize] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_finalized();

        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(!ready.contains(&StageId::Implement));
        assert!(ready.contains(&StageId::Design));
    }

    #[test]
    fn verify_runs_after_implement() {
        let mut sched = DagScheduler::with_quality(ProjectId::new(), MAX_DEBUG_CYCLES);
        for stage in [StageId::Ingest, StageId::Summarize, StageId::Design] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_finalized();
        complete_through(&mut sched, StageId::Implement);

        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::Verify));
    }

    #[test]
    fn verify_fail_triggers_debug_then_reverify() {
        let mut sched = DagScheduler::with_quality(ProjectId::new(), 3);
        for stage in [StageId::Ingest, StageId::Summarize, StageId::Design] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_finalized();
        complete_through(&mut sched, StageId::Implement);

        sched.record_verify_result(false);
        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::Debug));

        sched.mark_running(StageId::Debug);
        sched.record_debug_done();
        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::Verify));
    }

    #[test]
    fn verify_pass_enables_security_patch() {
        let mut sched = DagScheduler::with_quality(ProjectId::new(), 3);
        for stage in [StageId::Ingest, StageId::Summarize, StageId::Design] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_finalized();
        complete_through(&mut sched, StageId::Implement);

        sched.record_verify_result(true);
        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::SecurityPatch));
        assert!(!ready.contains(&StageId::Deliver));
    }

    #[test]
    fn security_patch_then_deliver() {
        let mut sched = DagScheduler::with_quality(ProjectId::new(), 3);
        for stage in [StageId::Ingest, StageId::Summarize, StageId::Design] {
            complete_through(&mut sched, stage);
        }
        sched.record_architect_finalized();
        complete_through(&mut sched, StageId::Implement);
        sched.record_verify_result(true);
        sched.record_security_done();

        let ready: HashSet<_> = sched.ready_stages().into_iter().map(|c| c.stage).collect();
        assert!(ready.contains(&StageId::Deliver));
    }
}
