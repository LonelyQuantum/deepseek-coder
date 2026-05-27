use prole_coder_agent_core::{
    context::{ContextBuilder, ContextBuilderConfig, ContextItem, ContextItemKind},
    test_helpers::TestWorkspace,
};

#[test]
#[ignore = "manual Phase 2d large-context benchmark; run with --ignored --nocapture"]
fn context_capsule_large_repository_budget_benchmark() {
    let targets = [200_000usize, 500_000, 900_000];

    for target_bytes in targets {
        let workspace = TestWorkspace::new("phase2d-context-capsule-benchmark");
        let path = format!("src/generated_{target_bytes}.rs");
        let content = deterministic_source_text(target_bytes);
        workspace.write(&path, &content);

        let capsule = ContextBuilder::new(ContextBuilderConfig::new(1_000_000))
            .add_item(ContextItem::workspace_manifest(
                format!(
                    "Workspace Manifest v0\nFiles: 1/1\nEntries:\n- {path} | source | {target_bytes} bytes | normal | untracked\n"
                ),
                "manual Phase 2d large-context benchmark manifest",
            ))
            .add_item(ContextItem::user_task(format!(
                "Inspect the generated {target_bytes} byte repository sample."
            )))
            .add_item(ContextItem::file(
                path.clone(),
                content,
                "manual Phase 2d large-context benchmark source file",
            ))
            .build()
            .expect("large context capsule should fit under the 1M budget");

        println!(
            "phase2d targetBytes={target_bytes} inputTokens={} stablePrefixTokens={} dynamicPreludeTokens={} turnSuffixTokens={} omittedSources={}",
            capsule.token_report.input_tokens,
            capsule.context_built_payload()["stablePrefixTokens"],
            capsule.context_built_payload()["dynamicPreludeTokens"],
            capsule.context_built_payload()["turnSuffixTokens"],
            capsule.token_report.omitted_sources.len()
        );

        assert!(capsule.token_report.input_tokens <= 1_000_000);
        assert!(capsule.token_report.omitted_sources.is_empty());
        assert!(
            capsule
                .token_report
                .included_sources
                .iter()
                .any(|source| source.source.kind == ContextItemKind::File
                    && source.source.path.as_deref() == Some(&path))
        );
    }
}

fn deterministic_source_text(target_bytes: usize) -> String {
    let line = "pub fn generated_value() -> usize { 42 } // phase2d deterministic benchmark\n";
    let mut text = String::with_capacity(target_bytes);
    while text.len() < target_bytes {
        text.push_str(line);
    }
    text.truncate(target_bytes);
    text
}
