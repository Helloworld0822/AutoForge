use crate::domain::ArtifactRef;

pub(super) fn merge(target: &mut Vec<ArtifactRef>, incoming: &[ArtifactRef]) {
    let mut result: Vec<ArtifactRef> = Vec::with_capacity(target.len() + incoming.len());
    for artifact in target.iter().chain(incoming) {
        if let Some(existing) = result
            .iter_mut()
            .find(|existing| existing.key == artifact.key)
        {
            existing.clone_from(artifact);
        } else {
            result.push(artifact.clone());
        }
    }
    *target = result;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_replaces_same_key_and_preserves_other_sources() {
        let artifact = ArtifactRef {
            name: "spec.md".into(),
            key: "p/spec.md".into(),
            uri: "old".into(),
            content_type: "text/markdown".into(),
            sha256: None,
        };
        let updated = ArtifactRef {
            uri: "new".into(),
            ..artifact.clone()
        };
        let other = ArtifactRef {
            key: "p/plan.pdf".into(),
            ..artifact.clone()
        };
        let mut target = vec![artifact.clone(), artifact, other];
        merge(&mut target, &[updated]);
        assert_eq!(target.len(), 2);
        assert_eq!(target[0].uri, "new");
        assert_eq!(target[1].key, "p/plan.pdf");
    }
}
