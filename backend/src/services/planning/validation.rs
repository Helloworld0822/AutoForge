use super::model::PlanningBundle;
use crate::services::ai::{parse_json, ProjectSpec};
use std::collections::{HashMap, HashSet, VecDeque};

pub fn parse_and_validate(content: &str) -> std::result::Result<PlanningBundle, String> {
    let bundle = parse_json(content).map_err(|error| error.to_string())?;
    validate(&bundle)?;
    Ok(bundle)
}

pub fn validate(bundle: &PlanningBundle) -> std::result::Result<(), String> {
    validate_document("architecture", &bundle.architecture.content)?;
    validate_document("spec", &bundle.spec.content)?;
    validate_confidence(bundle.planning_meta.confidence)?;
    validate_task_ids(&bundle.tasks)?;
    validate_dependencies(&bundle.tasks)
}

pub fn escalation_needed(
    project_spec: &ProjectSpec,
    bundle: Option<&PlanningBundle>,
    validation_exhausted: bool,
) -> bool {
    validation_exhausted
        || project_spec
            .contradictions
            .iter()
            .any(|contradiction| !contradiction.trim().is_empty())
        || bundle.is_some_and(|value| {
            value
                .planning_meta
                .complexity
                .eq_ignore_ascii_case("extreme")
                || value.planning_meta.confidence < 0.7
        })
}

fn validate_document(name: &str, content: &str) -> std::result::Result<(), String> {
    if content.trim().is_empty() {
        return Err(format!("{name} document must not be blank"));
    }
    Ok(())
}

fn validate_confidence(confidence: f64) -> std::result::Result<(), String> {
    if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
        return Err("planning_meta.confidence must be between 0.0 and 1.0".into());
    }
    Ok(())
}

fn validate_task_ids(tasks: &[super::model::PlanningTask]) -> std::result::Result<(), String> {
    if tasks.is_empty() {
        return Err("plan must contain at least one task".into());
    }
    let mut ids = HashSet::new();
    for task in tasks {
        if task.title.trim().is_empty()
            || task.acceptance_criteria.is_empty()
            || task.status != "pending"
        {
            return Err("each task needs a title, acceptance criteria and pending status".into());
        }
        if task.id.trim().is_empty() {
            return Err("task id must not be blank".into());
        }
        if !ids.insert(task.id.as_str()) {
            return Err(format!("duplicate task id: {}", task.id));
        }
    }
    Ok(())
}

fn validate_dependencies(tasks: &[super::model::PlanningTask]) -> std::result::Result<(), String> {
    let ids: HashSet<_> = tasks.iter().map(|task| task.id.as_str()).collect();
    let mut remaining: HashMap<_, _> = tasks
        .iter()
        .map(|task| (task.id.as_str(), task.dependencies.len()))
        .collect();
    let mut dependents: HashMap<&str, Vec<&str>> = HashMap::new();

    for task in tasks {
        for dependency in &task.dependencies {
            if dependency.trim().is_empty() || !ids.contains(dependency.as_str()) {
                return Err(format!(
                    "task {} has a missing dependency: {dependency}",
                    task.id
                ));
            }
            if dependency == &task.id {
                return Err(format!("task {} depends on itself", task.id));
            }
            dependents
                .entry(dependency.as_str())
                .or_default()
                .push(task.id.as_str());
        }
    }

    let mut ready: VecDeque<_> = remaining
        .iter()
        .filter_map(|(id, count)| (*count == 0).then_some(*id))
        .collect();
    let mut completed = 0;
    while let Some(id) = ready.pop_front() {
        completed += 1;
        if let Some(waiting) = dependents.get(id) {
            for task_id in waiting {
                let count = remaining
                    .get_mut(task_id)
                    .ok_or_else(|| format!("task dependency index missing: {task_id}"))?;
                *count -= 1;
                if *count == 0 {
                    ready.push_back(task_id);
                }
            }
        }
    }
    if completed != tasks.len() {
        return Err("task dependency graph contains a cycle".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::planning::{PlanningDocument, PlanningMeta, PlanningTask};

    fn valid_bundle() -> PlanningBundle {
        PlanningBundle {
            architecture: PlanningDocument {
                title: "Architecture".into(),
                content: "Service boundaries".into(),
            },
            spec: PlanningDocument {
                title: "Specification".into(),
                content: "Build the service".into(),
            },
            tasks: vec![PlanningTask {
                id: "api".into(),
                title: "Create API".into(),
                task_type: "implementation".into(),
                dependencies: vec![],
                acceptance_criteria: vec!["endpoint responds".into()],
                risk: "low".into(),
                estimated_context: vec!["src/api.rs".into()],
                status: "pending".into(),
            }],
            planning_meta: PlanningMeta {
                complexity: "normal".into(),
                confidence: 0.8,
                requires_escalation: false,
                reason: None,
            },
        }
    }

    #[test]
    fn validation_rejects_blank_document() {
        // Given: an otherwise valid plan.
        let mut blank_document = valid_bundle();
        // When: the architecture document is blank.
        blank_document.architecture.content.clear();
        // Then: validation rejects it.
        assert!(validate(&blank_document).is_err());
    }

    #[test]
    fn validation_rejects_blank_task_id() {
        // Given: an otherwise valid plan.
        let mut blank_task_id = valid_bundle();
        // When: its only task has no ID.
        blank_task_id.tasks[0].id.clear();
        // Then: validation rejects it.
        assert!(validate(&blank_task_id).is_err());
    }

    #[test]
    fn validation_rejects_invalid_confidence() {
        // Given: an otherwise valid plan.
        let mut invalid_confidence = valid_bundle();
        // When: confidence exceeds its valid range.
        invalid_confidence.planning_meta.confidence = 1.1;
        // Then: validation rejects it.
        assert!(validate(&invalid_confidence).is_err());
    }

    #[test]
    fn validation_rejects_missing_dependency() {
        // Given: an otherwise valid plan.
        let mut missing_dependency = valid_bundle();
        // When: a task references no known dependency.
        missing_dependency.tasks[0].dependencies = vec!["missing".into()];
        // Then: validation rejects it.
        assert!(validate(&missing_dependency).is_err());
    }

    #[test]
    fn validation_rejects_cyclic_dependency_graph() {
        // Given: an otherwise valid plan with a second task.
        let mut cyclic = valid_bundle();
        cyclic.tasks.push(PlanningTask {
            id: "worker".into(),
            title: "Create worker".into(),
            task_type: "implementation".into(),
            dependencies: vec!["api".into()],
            acceptance_criteria: vec!["worker runs".into()],
            risk: "low".into(),
            estimated_context: vec!["src/worker.rs".into()],
            status: "pending".into(),
        });
        // When: the tasks depend on one another.
        cyclic.tasks[0].dependencies = vec!["worker".into()];
        // Then: validation rejects the cycle.
        assert!(validate(&cyclic).is_err());
    }

    #[test]
    fn escalation_does_not_run_for_normal_plan() {
        // Given: a consistent project and normal, confident plan.
        let normal_spec = ProjectSpec::default();
        let normal_plan = valid_bundle();
        // When: escalation is evaluated without validation exhaustion.
        // Then: no escalation is required.
        assert!(!escalation_needed(&normal_spec, Some(&normal_plan), false));
    }

    #[test]
    fn escalation_runs_for_extreme_complexity() {
        // Given: a normal project and an extreme plan.
        let normal_spec = ProjectSpec::default();
        let mut extreme_plan = valid_bundle();
        extreme_plan.planning_meta.complexity = "extreme".into();
        // When: escalation is evaluated.
        // Then: escalation is required.
        assert!(escalation_needed(&normal_spec, Some(&extreme_plan), false));
    }

    #[test]
    fn escalation_runs_for_low_confidence() {
        // Given: a normal project and low-confidence plan.
        let normal_spec = ProjectSpec::default();
        let mut low_confidence_plan = valid_bundle();
        low_confidence_plan.planning_meta.confidence = 0.69;
        // When: escalation is evaluated.
        // Then: escalation is required.
        assert!(escalation_needed(
            &normal_spec,
            Some(&low_confidence_plan),
            false
        ));
    }

    #[test]
    fn escalation_runs_for_project_contradiction() {
        // Given: a contradictory project and normal plan.
        let contradictory_spec = ProjectSpec {
            contradictions: vec!["Two incompatible database requirements".into()],
            ..ProjectSpec::default()
        };
        let normal_plan = valid_bundle();
        // When: escalation is evaluated.
        // Then: escalation is required.
        assert!(escalation_needed(
            &contradictory_spec,
            Some(&normal_plan),
            false
        ));
    }

    #[test]
    fn escalation_runs_when_validation_retries_are_exhausted() {
        // Given: a normal project and no validated plan.
        let normal_spec = ProjectSpec::default();
        // When: validation retries are exhausted.
        // Then: escalation is required.
        assert!(escalation_needed(&normal_spec, None, true));
    }
}
